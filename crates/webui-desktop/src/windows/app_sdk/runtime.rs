// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Framework-dependent Windows App Runtime bootstrap and official HWND interop.

use std::marker::PhantomData;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};
use windows::core::{s, w, HRESULT, HSTRING, PCSTR, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HMODULE, HWND, S_OK};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_FLAGS, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};

use super::bindings::Microsoft::UI::Dispatching::{DispatcherQueue, DispatcherQueueController};
use super::bindings::Microsoft::UI::WindowId;

const BOOTSTRAP_DLL: &str = "Microsoft.WindowsAppRuntime.Bootstrap.dll";
const RELEASE: u32 = 0x0001_0008;
const MINIMUM_VERSION: u64 = 0x1f40_03b2_06a5_0000;
const INSTALL_HELP: &str = "install the architecture-matching Windows App Runtime 1.8 (8000.946.1701.0 or newer 1.8 servicing release) and Visual C++ Redistributable; see https://learn.microsoft.com/windows/apps/windows-app-sdk/downloads";

type Initialize = unsafe extern "system" fn(u32, PCWSTR, u64, u32) -> HRESULT;
type Shutdown = unsafe extern "system" fn();
type WindowIdFromWindow = unsafe extern "system" fn(HWND, *mut WindowId) -> HRESULT;

/// Owns the bootstrap for one UI-thread run, outliving its windows and SDK objects.
pub(in crate::windows) struct Runtime {
    dispatcher: Dispatcher,
    _interop: Module,
    convert: WindowIdFromWindow,
    _bootstrap: Bootstrap,
    _thread: PhantomData<Rc<()>>,
}

struct Bootstrap {
    _bootstrap: Module,
    shutdown: Shutdown,
}

impl Runtime {
    pub(in crate::windows) fn initialize() -> Result<Self> {
        let executable = std::env::current_exe().context("cannot locate the desktop executable")?;
        let directory = executable
            .parent()
            .context("desktop executable has no directory")?;
        Self::from_directory(directory)
    }

    fn from_directory(directory: &Path) -> Result<Self> {
        let bootstrap = Bootstrap::from_directory(directory)?;
        let dispatcher =
            Dispatcher::current().context("cannot initialize the Windows App SDK UI dispatcher")?;
        let interop = Module::load(
            w!("Microsoft.Internal.FrameworkUdk.dll"),
            LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
        .context("cannot load Windows App SDK HWND interop from the initialized runtime")?;
        // SAFETY: Microsoft.UI.Interop.h defines this exact export and ABI.
        // Keep the module alive through every associated HWND's destruction.
        let convert: WindowIdFromWindow =
            unsafe { std::mem::transmute(interop.export(s!("Windowing_GetWindowIdFromWindow"))?) };
        Ok(Self {
            dispatcher,
            _interop: interop,
            convert,
            _bootstrap: bootstrap,
            _thread: PhantomData,
        })
    }

    pub(super) fn window_id(&self, hwnd: HWND) -> Result<WindowId> {
        let mut id = WindowId::default();
        // SAFETY: The HWND is live and `id` is writable for this synchronous call.
        unsafe { (self.convert)(hwnd, &mut id).ok()? };
        Ok(id)
    }

    pub(super) fn dispatcher(&self) -> &DispatcherQueue {
        &self.dispatcher.queue
    }
}

struct Dispatcher {
    queue: DispatcherQueue,
    controller: Option<DispatcherQueueController>,
}

impl Dispatcher {
    fn current() -> windows::core::Result<Self> {
        match DispatcherQueue::GetForCurrentThread() {
            Ok(queue) => Ok(Self {
                queue,
                controller: None,
            }),
            // The generated non-null projection maps the documented null
            // result (no queue on this thread) to Error::empty, retaining S_OK.
            Err(error) if error.code() == S_OK => {
                let controller = DispatcherQueueController::CreateOnCurrentThread()?;
                let queue = controller.DispatcherQueue()?;
                Ok(Self {
                    queue,
                    controller: Some(controller),
                })
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        if let Some(controller) = &self.controller {
            if let Err(error) = controller.ShutdownQueue() {
                eprintln!("WebUI: failed to shut down the Windows App SDK dispatcher: {error}");
            }
        }
    }
}

impl Bootstrap {
    fn from_directory(directory: &Path) -> Result<Self> {
        let path = directory.join(BOOTSTRAP_DLL);
        let bootstrap = Module::load(
            PCWSTR(HSTRING::from(path.as_os_str()).as_ptr()),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
        .with_context(|| {
            format!(
                "cannot load {}; help: distribute the matching Windows App SDK bootstrap DLL beside the executable",
                path.display()
            )
        })?;
        // SAFETY: These exports and signatures are defined by pinned MddBootstrap.h;
        // the owned module remains loaded until after shutdown.
        let (initialize, shutdown): (Initialize, Shutdown) = unsafe {
            (
                std::mem::transmute::<unsafe extern "system" fn() -> isize, Initialize>(
                    bootstrap.export(s!("MddBootstrapInitialize2"))?,
                ),
                std::mem::transmute::<unsafe extern "system" fn() -> isize, Shutdown>(
                    bootstrap.export(s!("MddBootstrapShutdown"))?,
                ),
            )
        };
        initialize_runtime(initialize)?;
        Ok(Self {
            _bootstrap: bootstrap,
            shutdown,
        })
    }
}

fn initialize_runtime(initialize: Initialize) -> Result<()> {
    // SAFETY: PACKAGE_VERSION is an eight-byte union passed by value. Its
    // Version member, release and empty tag come from WindowsAppSDK-VersionInfo.h.
    unsafe { initialize(RELEASE, w!(""), MINIMUM_VERSION, 0).ok() }
        .with_context(|| format!("Windows App Runtime initialization failed; help: {INSTALL_HELP}"))
}

impl Drop for Bootstrap {
    fn drop(&mut self) {
        // SAFETY: Initialization succeeded, and the guard outlives all SDK users.
        unsafe { (self.shutdown)() };
    }
}

struct Module(HMODULE);

impl Module {
    fn load(path: PCWSTR, flags: LOAD_LIBRARY_FLAGS) -> windows::core::Result<Self> {
        // SAFETY: Callers provide a terminated wide path alive for this call.
        unsafe { LoadLibraryExW(path, None, flags).map(Self) }
    }

    fn export(&self, name: PCSTR) -> windows::core::Result<unsafe extern "system" fn() -> isize> {
        // SAFETY: The module is owned and name is a terminated static export name.
        unsafe { GetProcAddress(self.0, name) }.ok_or_else(windows::core::Error::from_thread)
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: Each successful LoadLibraryExW owns exactly one module reference.
        if let Err(error) = unsafe { FreeLibrary(self.0) } {
            eprintln!("WebUI: failed to release Windows App SDK module: {error}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn missing_bootstrap_reports_required_companion_without_initializing_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let error = Runtime::from_directory(directory.path()).err().unwrap();
        let message = error.to_string();
        assert!(message.contains(BOOTSTRAP_DLL));
        assert!(message.contains("beside the executable"));
    }

    #[test]
    fn missing_shared_runtime_reports_release_architecture_and_installer() {
        unsafe extern "system" fn unavailable(_: u32, _: PCWSTR, _: u64, _: u32) -> HRESULT {
            HRESULT(0x8007_3d54_u32.cast_signed())
        }
        let error = initialize_runtime(unavailable).err().unwrap();
        let message = error.to_string();
        assert!(message.contains("8000.946.1701.0"));
        assert!(message.contains("architecture-matching"));
        assert!(message.contains("https://learn.microsoft.com/"));
        assert!(error.chain().count() > 1);
    }

    #[test]
    fn borrowing_an_existing_ui_queue_does_not_shut_it_down() {
        use super::super::bindings::Microsoft::UI::Dispatching::DispatcherQueueHandler;
        let _com = crate::windows::initialize_com().unwrap();
        let runtime = Runtime::initialize().unwrap();
        let borrowed = Dispatcher::current().unwrap();
        assert!(borrowed.controller.is_none());
        drop(borrowed);
        let work = DispatcherQueueHandler::new(|| Ok(()));
        assert!(runtime.dispatcher().TryEnqueue(&work).unwrap());
    }
}
