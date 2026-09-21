// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#[cfg(target_os = "macos")]
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

#[cfg(target_os = "macos")]
use webui_desktop::DesktopMenu;

// Reuse the private production module in a main-thread harness rather than
// exposing a testing API or faking AppKit thread affinity.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
#[path = "../src/macos/menu.rs"]
mod menu;

#[cfg(target_os = "macos")]
fn response_buffers(c: &mut Criterion) {
    use std::hint::black_box;

    use objc2::rc::autoreleasepool;
    use objc2_foundation::NSData;

    let mut group = c.benchmark_group("macos_response_buffer");
    for size in [64 * 1024, 1024 * 1024, 16 * 1024 * 1024] {
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("copy", size), &size, |b, &size| {
            b.iter_batched(
                || vec![0x5a; size],
                |body| {
                    autoreleasepool(|_| {
                        black_box(NSData::with_bytes(&body));
                    })
                },
                BatchSize::PerIteration,
            );
        });
        group.bench_with_input(BenchmarkId::new("transfer", size), &size, |b, &size| {
            b.iter_batched(
                || vec![0x5a; size],
                |body| {
                    autoreleasepool(|_| {
                        black_box(NSData::from_vec(body));
                    })
                },
                BatchSize::PerIteration,
            );
        });
    }
    group.finish();
}

#[cfg(target_os = "macos")]
criterion_group!(benches, response_buffers, menu_lifetime_guard);
#[cfg(target_os = "macos")]
criterion_main!(benches);

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn menu_lifetime_guard(_c: &mut Criterion) {
    menu_guard::check();
    eprintln!("Native menu ownership, autorelease, and single-dispatch guard: passed");
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod menu_guard {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use objc2::rc::{autoreleasepool, Weak};
    use objc2::{msg_send, MainThreadMarker, Message};

    use super::{menu, DesktopMenu};

    pub(super) fn check() {
        let Some(mtm) = MainThreadMarker::new() else {
            panic!("the native menu regression guard must run on the main thread");
        };
        let menus: Vec<DesktopMenu> = match serde_json::from_str(
            r#"[{"id":"guard","label":"Guard","items":[{"id":"once","label":"Once","command":"once"}]}]"#,
        ) {
            Ok(menus) => menus,
            Err(error) => panic!("invalid menu regression fixture: {error}"),
        };
        let calls = Rc::new(Cell::new(0));
        let last_script = Rc::new(RefCell::new(String::new()));
        let sink_calls = Rc::clone(&calls);
        let sink_script = Rc::clone(&last_script);
        let (owner, bar, weak_target) = autoreleasepool(|_| {
            let owner = menu::build_main_menu(mtm, &menus, move |script| {
                sink_calls.set(sink_calls.get() + 1);
                *sink_script.borrow_mut() = script.to_owned();
            });
            let bar = owner.menu().retain();
            let Some(group) = bar.itemAtIndex(0) else {
                panic!("missing menu group");
            };
            let Some(submenu) = group.submenu() else {
                panic!("missing submenu");
            };
            let Some(item) = submenu.itemAtIndex(0) else {
                panic!("missing command item");
            };
            let Some(target) = item.target() else {
                panic!("menu command target was released during construction");
            };
            let weak_target = Weak::from_retained(&target);
            (owner, bar, weak_target)
        });
        autoreleasepool(|_| {
            let Some(target) = weak_target.load() else {
                panic!("menu target did not survive the autorelease boundary");
            };
            let Some(item) = bar
                .itemAtIndex(0)
                .and_then(|group| group.submenu())
                .and_then(|submenu| submenu.itemAtIndex(0))
            else {
                panic!("menu command item was released");
            };
            // SAFETY: This is the production DesktopMenuTarget and its command
            // selector, invoked on the main thread with a live NSMenuItem.
            let _: () = unsafe { msg_send![&*target, performDesktopMenuCommand: &*item] };
        });
        assert_eq!(calls.get(), 1);
        assert!(last_script.borrow().contains("id:\"once\""));
        drop(owner);
        autoreleasepool(|_| assert!(weak_target.load().is_none()));
        // The retained AppKit menu above must not retain its weak target.
        drop(bar);
    }
}
