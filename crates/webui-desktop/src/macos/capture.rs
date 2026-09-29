// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! UI-local public WK viewport capture adapter. Only opaque IDs cross GCD.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use block2::RcBlock;
use objc2::rc::{autoreleasepool, Weak as ObjcWeak};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSBitmapFormat, NSBitmapImageRep, NSGraphicsContext, NSImage, NSWindow};
use objc2_foundation::{NSError, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

use crate::capture::{CaptureError, CaptureOptions, CaptureState, MAX_WEB_CAPTURE_RASTER_BYTES};

mod png;

static NEXT_TARGET: AtomicU64 = AtomicU64::new(1);
thread_local! {
    static TARGETS: RefCell<HashMap<u64, Rc<Target>>> = RefCell::new(HashMap::new());
}

#[link(name = "System")]
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

struct Target {
    view: ObjcWeak<WKWebView>,
    window: ObjcWeak<NSWindow>,
    owner: Weak<CaptureState>,
    dispatch: Arc<Dispatch>,
}

pub(crate) struct Dispatch {
    id: u64,
    closed: AtomicBool,
    queued: AtomicBool,
    submission: Mutex<Option<(u64, u64, CaptureOptions)>>,
}

pub(crate) struct Registration {
    dispatch: Arc<Dispatch>,
    owner: Arc<CaptureState>,
    _ui_only: std::marker::PhantomData<Rc<()>>,
}

impl Registration {
    pub(crate) fn close(&self) {
        self.dispatch.closed.store(true, Ordering::Release);
        self.owner.notify_closed();
        TARGETS.with(|targets| {
            targets.borrow_mut().remove(&self.dispatch.id);
        });
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) fn install(
    owner: &Arc<CaptureState>,
    window: &NSWindow,
    view: &WKWebView,
) -> Registration {
    let dispatch = Arc::new(Dispatch {
        id: NEXT_TARGET.fetch_add(1, Ordering::Relaxed),
        closed: AtomicBool::new(false),
        queued: AtomicBool::new(false),
        submission: Mutex::new(None),
    });
    TARGETS.with(|targets| {
        targets.borrow_mut().insert(
            dispatch.id,
            Rc::new(Target {
                view: ObjcWeak::new(view),
                window: ObjcWeak::new(window),
                owner: Arc::downgrade(owner),
                dispatch: Arc::clone(&dispatch),
            }),
        );
    });
    owner.attach(Arc::clone(&dispatch));
    Registration {
        dispatch,
        owner: Arc::clone(owner),
        _ui_only: std::marker::PhantomData,
    }
}

impl Dispatch {
    pub(crate) fn submit(
        &self,
        id: u64,
        epoch: u64,
        options: CaptureOptions,
    ) -> Result<(), CaptureError> {
        let mut pending = self
            .submission
            .lock()
            .map_err(|_| CaptureError::Scheduler)?;
        if self.closed.load(Ordering::Acquire) {
            return Err(CaptureError::Closed);
        }
        if pending.is_some() {
            return Err(CaptureError::Busy);
        }
        *pending = Some((id, epoch, options));
        drop(pending);
        if !self.queued.swap(true, Ordering::AcqRel) {
            let id = Box::into_raw(Box::new(self.id)).cast::<c_void>();
            // SAFETY: GCD delivers this one boxed target ID on the main queue.
            unsafe {
                dispatch_async_f(std::ptr::addr_of!(_dispatch_main_q).cast_mut(), id, drain);
            }
        }
        Ok(())
    }
}

unsafe extern "C" fn drain(context: *mut c_void) {
    // SAFETY: submit transfers precisely one boxed ID to this GCD callback.
    let id = unsafe { Box::from_raw(context.cast::<u64>()) };
    let target = TARGETS.with(|targets| targets.borrow().get(&id).cloned());
    if let Some(target) = target {
        target.dispatch.queued.store(false, Ordering::Release);
        let pending = target
            .dispatch
            .submission
            .lock()
            .ok()
            .and_then(|mut pending| pending.take());
        if let Some((id, epoch, options)) = pending {
            start_snapshot(&target, id, epoch, options);
        }
    }
}

fn start_snapshot(target: &Target, id: u64, epoch: u64, options: CaptureOptions) {
    let Some(owner) = target.owner.upgrade() else {
        return;
    };
    if target.dispatch.closed.load(Ordering::Acquire) || !owner.current(id, epoch) {
        owner.complete(id, epoch, Err(CaptureError::Cancelled));
        return;
    }
    let Some(view) = target.view.load() else {
        owner.complete(id, epoch, Err(CaptureError::Unavailable));
        return;
    };
    let Some(window) = target.window.load() else {
        owner.complete(id, epoch, Err(CaptureError::Unavailable));
        return;
    };
    if !view
        .window()
        .is_some_and(|attached| std::ptr::eq(&*attached, &*window))
    {
        owner.complete(id, epoch, Err(CaptureError::Unavailable));
        return;
    }
    // SAFETY: GCD's main queue owns this WebKit view and its loading state.
    if unsafe { view.isLoading() } {
        owner.complete(id, epoch, Err(CaptureError::Unavailable));
        return;
    }
    let bounds = view.bounds();
    let scale = window.backingScaleFactor();
    if !bounds.size.width.is_finite()
        || !bounds.size.height.is_finite()
        || !scale.is_finite()
        || bounds.size.width <= 0.0
        || bounds.size.height <= 0.0
        || scale <= 0.0
    {
        owner.complete(id, epoch, Err(CaptureError::Incomplete));
        return;
    }
    // WK's width is in points, while the limits are FINAL bitmap pixels.
    // Both axes use the full visible bounds; nothing is cropped or upscaled.
    let width_points = bounds
        .size
        .width
        .min(f64::from(options.max_width) / scale)
        .min(f64::from(options.max_height) * bounds.size.width / bounds.size.height / scale);
    if !width_points.is_finite() || width_points <= 0.0 {
        owner.complete(id, epoch, Err(CaptureError::TooLarge));
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        owner.complete(id, epoch, Err(CaptureError::Scheduler));
        return;
    };
    // SAFETY: Configuration is initialized and used only on AppKit's thread.
    let config = unsafe { WKSnapshotConfiguration::new(mtm) };
    // SAFETY: The public API's rect is exactly the attached WK view's bounds.
    unsafe {
        config.setRect(bounds);
        config.setSnapshotWidth(Some(&NSNumber::new_f64(width_points)));
        config.setAfterScreenUpdates(true);
    }
    let expected_aspect = bounds.size.height / bounds.size.width;
    let native_pixel_width = bounds.size.width * scale;
    let native_pixel_height = bounds.size.height * scale;
    let weak_owner = target.owner.clone();
    let callback = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        let Some(owner) = weak_owner.upgrade() else {
            return;
        };
        // Abandoned, timed-out, navigated or retired requests must not run
        // either native bitmap drawing or the PNG encoder on a late callback.
        owner.complete_if_current(id, epoch, || {
            // SAFETY: WebKit owns NSError/NSImage for the callback duration.
            if let Some(error) = unsafe { error.as_ref() } {
                return Err(CaptureError::NativeCode(error.code()));
            }
            let Some(image) = (unsafe { image.as_ref() }) else {
                return Err(CaptureError::Incomplete);
            };
            autoreleasepool(|_| {
                encode_png(
                    image,
                    options,
                    expected_aspect,
                    native_pixel_width,
                    native_pixel_height,
                )
            })
        });
    });
    // SAFETY: Public WK API captures composited visible web content only.
    unsafe { view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &callback) };
}

fn encode_png(
    image: &NSImage,
    options: CaptureOptions,
    expected_aspect: f64,
    native_pixel_width: f64,
    native_pixel_height: f64,
) -> Result<(u32, u32, Vec<u8>), CaptureError> {
    let rep = image
        .representations()
        .firstObject()
        .ok_or(CaptureError::Incomplete)?;
    let width = u32::try_from(rep.pixelsWide()).map_err(|_| CaptureError::Incomplete)?;
    let height = u32::try_from(rep.pixelsHigh()).map_err(|_| CaptureError::Incomplete)?;
    let raw_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(CaptureError::TooLarge)?;
    if width == 0
        || height == 0
        || width > options.max_width
        || height > options.max_height
        || raw_bytes > MAX_WEB_CAPTURE_RASTER_BYTES
    {
        return Err(CaptureError::TooLarge);
    }
    // A clipped/partial WK image is not an acceptable representation of the
    // complete visible viewport. Allow at most one output-pixel of rounding.
    let expected_height = f64::from(width) * expected_aspect;
    if !expected_height.is_finite() || (f64::from(height) - expected_height).abs() > 1.5 {
        return Err(CaptureError::Incomplete);
    }
    if !native_pixel_width.is_finite()
        || !native_pixel_height.is_finite()
        || f64::from(width) > native_pixel_width + 1.0
        || f64::from(height) > native_pixel_height + 1.0
    {
        return Err(CaptureError::Incomplete);
    }
    // SAFETY: Null planes ask AppKit to allocate precisely the checked RGBA
    // bitmap. No TIFF, window image, JS data URL, or screen permission is used.
    let bitmap = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            isize::try_from(width).map_err(|_| CaptureError::TooLarge)?,
            isize::try_from(height).map_err(|_| CaptureError::TooLarge)?,
            8, 4, true, false, &NSString::from_str("NSDeviceRGBColorSpace"),
            NSBitmapFormat::empty(),
            isize::try_from(width).map_err(|_| CaptureError::TooLarge)? * 4,
            32,
        )
    }
    .ok_or_else(|| CaptureError::Native("bounded bitmap allocation failed".into()))?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)
        .ok_or_else(|| CaptureError::Native("bitmap graphics context unavailable".into()))?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    image.drawInRect(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(f64::from(width), f64::from(height)),
    ));
    NSGraphicsContext::restoreGraphicsState_class();
    // AppKit was asked to allocate exactly width*height*4, with one RGBA
    // plane and an explicit width*4 stride. Refuse any other layout before
    // borrowing the raw pixels; the pointer lives only while bitmap does.
    if bitmap.isPlanar()
        || bitmap.numberOfPlanes() != 1
        || bitmap.samplesPerPixel() != 4
        || bitmap.bitsPerPixel() != 32
        || bitmap.bytesPerRow() != isize::try_from(width).map_err(|_| CaptureError::TooLarge)? * 4
        || bitmap.bitmapFormat() != NSBitmapFormat::empty()
    {
        return Err(CaptureError::Incomplete);
    }
    let ptr = std::ptr::NonNull::new(bitmap.bitmapData()).ok_or(CaptureError::Incomplete)?;
    // SAFETY: AppKit owns an initialized, non-planar RGBA buffer with the
    // exact explicit row stride and dimensions checked above. It outlives
    // this synchronous bounded PNG encoder, and no Objective-C call mutates
    // the representation while the slice is borrowed.
    let pixels = unsafe { std::slice::from_raw_parts(ptr.as_ptr(), raw_bytes) };
    let png = png::encode_rgba(pixels, width, height, options.max_png_bytes)?;
    Ok((width, height, png))
}
