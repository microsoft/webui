// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;
#[test]
fn dispatch_lets_a_handler_register_and_dispatch_without_deadlocking() {
    let registry = EventRegistry::default();
    let nested = Arc::downgrade(&registry.inner);
    let nested_once = Arc::new(AtomicBool::new(false));
    registry
        .on_event(move |_| {
            // Both calls re-enter the registry. While `dispatch` held the
            // handler mutex across callbacks, either one deadlocked the UI
            // thread against a non-reentrant mutex. The weak capture keeps
            // the regression test from creating an owner-handler cycle.
            if !nested_once.swap(true, Ordering::SeqCst) {
                let inner = nested.upgrade().unwrap();
                let nested_registry = EventRegistry { inner };
                nested_registry
                    .on_event(|_| EventResponse::Continue)
                    .unwrap();
                let _ = nested_registry.dispatch(&DesktopEvent::Ready);
            }
            EventResponse::Continue
        })
        .unwrap();
    // Dispatch on a worker so a regression fails this test instead of
    // hanging the suite forever.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let response = registry.dispatch(&DesktopEvent::Ready);
        let _ = tx.send(response);
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)),
        Ok(EventResponse::Continue),
        "re-entrant dispatch deadlocked"
    );
}
#[test]
fn dispatch_runs_every_handler_even_after_one_cancels() {
    let registry = EventRegistry::default();
    let seen = Arc::new(AtomicUsize::new(0));
    let first = Arc::clone(&seen);
    registry
        .on_event(move |_| {
            first.fetch_add(1, Ordering::SeqCst);
            EventResponse::PreventDefault
        })
        .unwrap();
    let second = Arc::clone(&seen);
    registry
        .on_event(move |_| {
            second.fetch_add(1, Ordering::SeqCst);
            EventResponse::Continue
        })
        .unwrap();
    assert_eq!(
        registry.dispatch(&DesktopEvent::Ready),
        EventResponse::PreventDefault
    );
    assert_eq!(
        seen.load(Ordering::SeqCst),
        2,
        "cancelling handler hid the event from later handlers"
    );
}
#[test]
fn subscription_removal_keeps_current_snapshot_and_updates_nested_dispatch() {
    let registry = EventRegistry::default();
    let subscription = Arc::new(Mutex::new(None));
    let nested_once = Arc::new(AtomicBool::new(false));
    let subscribed_calls = Arc::new(AtomicUsize::new(0));

    let holder = Arc::clone(&subscription);
    let nested = registry.clone();
    let once = Arc::clone(&nested_once);
    registry
        .on_event(move |_| {
            if !once.swap(true, Ordering::SeqCst) {
                let removed = holder.lock().unwrap().take();
                drop(removed);
                let _ = nested.dispatch(&DesktopEvent::Ready);
            }
            EventResponse::Continue
        })
        .unwrap();
    let calls = Arc::clone(&subscribed_calls);
    *subscription.lock().unwrap() = Some(
        registry
            .subscribe(move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                EventResponse::Continue
            })
            .unwrap(),
    );

    let _ = registry.dispatch(&DesktopEvent::Ready);
    assert_eq!(subscribed_calls.load(Ordering::SeqCst), 1);
    let _ = registry.dispatch(&DesktopEvent::Ready);
    assert_eq!(subscribed_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn subscription_callback_drop_can_reenter_registry() {
    struct RegisterOnDrop {
        registry: EventRegistry,
        complete: mpsc::Sender<Result<(), EventRegistrationError>>,
    }
    impl Drop for RegisterOnDrop {
        fn drop(&mut self) {
            let result = self.registry.on_event(|_| EventResponse::Continue);
            let _ = self.complete.send(result);
        }
    }

    let registry = EventRegistry::default();
    let (tx, rx) = mpsc::channel();
    let reentrant = RegisterOnDrop {
        registry: registry.clone(),
        complete: tx,
    };
    let subscription = registry
        .subscribe(move |_| {
            let _ = &reentrant;
            EventResponse::Continue
        })
        .unwrap();

    std::thread::spawn(move || drop(subscription));
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)),
        Ok(Ok(())),
        "dropping a callback while holding the registry lock deadlocked"
    );
    assert_eq!(
        registry.dispatch(&DesktopEvent::Ready),
        EventResponse::Continue
    );
}

#[test]
fn registry_enforces_combined_cap_and_rejects_registration_after_close() {
    let registry = EventRegistry::default();
    for _ in 0..(MAX_EVENT_HANDLERS - 1) {
        registry.on_event(|_| EventResponse::Continue).unwrap();
    }
    let subscription = registry.subscribe(|_| EventResponse::Continue).unwrap();
    assert_eq!(
        registry.on_event(|_| EventResponse::Continue),
        Err(EventRegistrationError::Capacity)
    );
    drop(subscription);
    registry.on_event(|_| EventResponse::Continue).unwrap();

    registry.close();
    assert_eq!(
        registry.subscribe(|_| EventResponse::Continue).err(),
        Some(EventRegistrationError::Closed)
    );
    assert_eq!(
        registry.dispatch(&DesktopEvent::Ready),
        EventResponse::Continue
    );
}

#[test]
fn registry_close_releases_self_capturing_callbacks() {
    struct Released(mpsc::Sender<()>);
    impl Drop for Released {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    let registry = EventRegistry::default();
    let captured_registry = registry.clone();
    let (tx, rx) = mpsc::channel();
    let released = Released(tx);
    registry
        .on_event(move |_| {
            let _ = (&captured_registry, &released);
            EventResponse::Continue
        })
        .unwrap();

    registry.close();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)),
        Ok(()),
        "registry close retained a callback ownership cycle"
    );
}

#[test]
fn empty_window_handle_does_not_allocate_a_command_queue() {
    let handle = WindowHandle::default();
    assert!(handle.inner.lock().unwrap().queue.is_none());
    assert!(handle.drain_commands().is_empty());
    assert!(handle.inner.lock().unwrap().queue.is_none());
}

#[test]
fn wakeup_can_reenter_send_without_deadlocking() {
    let handle = WindowHandle::default();
    let reentrant = handle.clone();
    let (tx, rx) = mpsc::channel();
    handle.set_wakeup(move || {
        let result = reentrant.send(WindowCommand::Center);
        let _ = tx.send(result);
    });

    let sender = handle.clone();
    std::thread::spawn(move || {
        let _ = sender.send(WindowCommand::Focus);
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)),
        Ok(Ok(())),
        "wakeup re-entry deadlocked on the wakeup lock"
    );
    assert_eq!(
        handle.drain_commands(),
        vec![WindowCommand::Focus, WindowCommand::Center]
    );
    handle.close();
}

#[test]
fn installing_wakeup_notifies_an_existing_backlog_once() {
    let handle = WindowHandle::default();
    handle.send(WindowCommand::Focus).unwrap();
    handle.send(WindowCommand::Center).unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&wakes);
    handle.set_wakeup(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });

    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(handle.drain_commands().len(), 2);
}

#[test]
fn wakeups_coalesce_until_drain_without_losing_the_next_wake() {
    let handle = WindowHandle::default();
    let wakes = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&wakes);
    handle.set_wakeup(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });

    handle.send(WindowCommand::Focus).unwrap();
    handle.send(WindowCommand::Center).unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(handle.drain_commands().len(), 2);
    handle.send(WindowCommand::Minimize).unwrap();
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    assert_eq!(handle.drain_commands(), vec![WindowCommand::Minimize]);
}

#[test]
fn command_queue_bounds_individual_and_aggregate_title_bytes() {
    let handle = WindowHandle::default();
    assert_eq!(
        handle.set_title("x".repeat(MAX_WINDOW_TITLE_BYTES + 1)),
        Err(WindowCommandError::TitleTooLarge {
            size: MAX_WINDOW_TITLE_BYTES + 1
        })
    );
    let mut oversized_allocation = String::with_capacity(1024 * 1024);
    oversized_allocation.push('x');
    handle
        .send(WindowCommand::SetTitle(oversized_allocation))
        .unwrap();
    assert_eq!(handle.inner.lock().unwrap().queued_title_bytes, 1);
    let normalized = handle.drain_commands();
    let WindowCommand::SetTitle(title) = &normalized[0] else {
        panic!("expected queued title command");
    };
    assert_eq!(title.capacity(), title.len());
    assert_eq!(title.len(), 1);

    for _ in 0..(MAX_QUEUED_WINDOW_TITLE_BYTES / MAX_WINDOW_TITLE_BYTES) {
        let mut title = String::with_capacity(1024 * 1024);
        title.push_str(&"x".repeat(MAX_WINDOW_TITLE_BYTES));
        handle.send(WindowCommand::SetTitle(title)).unwrap();
    }
    assert_eq!(
        handle.inner.lock().unwrap().queued_title_bytes,
        MAX_QUEUED_WINDOW_TITLE_BYTES
    );
    assert_eq!(
        handle.set_title("x"),
        Err(WindowCommandError::TitleQueueFull {
            queued: MAX_QUEUED_WINDOW_TITLE_BYTES,
            requested: 1
        })
    );
    let normalized = handle.drain_commands();
    assert_eq!(
        normalized.len(),
        MAX_QUEUED_WINDOW_TITLE_BYTES / MAX_WINDOW_TITLE_BYTES
    );
    for command in normalized {
        let WindowCommand::SetTitle(title) = command else {
            panic!("expected queued title command");
        };
        assert_eq!(title.capacity(), title.len());
    }
    handle.set_title("x").unwrap();
}

#[test]
fn command_close_rejects_senders_and_releases_wakeup_outside_lock() {
    struct SendOnDrop {
        handle: WindowHandle,
        complete: mpsc::Sender<Result<(), WindowCommandError>>,
    }
    impl Drop for SendOnDrop {
        fn drop(&mut self) {
            let _ = self.complete.send(self.handle.focus());
        }
    }

    let handle = WindowHandle::default();
    handle.set_title("pending").unwrap();
    let (tx, rx) = mpsc::channel();
    let reentrant = SendOnDrop {
        handle: handle.clone(),
        complete: tx,
    };
    handle.set_wakeup(move || {
        let _ = &reentrant;
    });

    let closer = handle.clone();
    std::thread::spawn(move || closer.close());
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)),
        Ok(Err(WindowCommandError::Closed)),
        "wakeup destruction re-entered while the command lock was held"
    );
    assert_eq!(handle.focus(), Err(WindowCommandError::Closed));
    assert_eq!(
        handle.set_title("x".repeat(MAX_WINDOW_TITLE_BYTES + 1)),
        Err(WindowCommandError::Closed)
    );
    assert!(handle.drain_commands().is_empty());
}

#[test]
fn event_js() {
    assert!(DesktopEvent::WindowResized {
        window_id: WindowId(1),
        width: 2,
        height: 3
    }
    .to_javascript()
    .unwrap()
    .contains("webui:window-resized"));
}
#[test]
fn event_js_emits_a_quoted_event_name() {
    let script = DesktopEvent::Ready.to_javascript().unwrap();
    assert!(
        script.starts_with("window.dispatchEvent(new CustomEvent(\"webui:ready\","),
        "unexpected script: {script}"
    );
}
#[test]
fn drag_script_resolves_regions_through_shadow_dom() {
    // Regression: a `document`-level listener sees shadow DOM events
    // retargeted to the host, so `target.closest()` cannot find a drag
    // region declared inside a component's shadow root.
    assert!(DRAG_REGION_SCRIPT.contains("composedPath"));
    assert!(!DRAG_REGION_SCRIPT.contains("closest"));
}
#[test]
fn drag_script_ignores_non_primary_buttons() {
    assert!(DRAG_REGION_SCRIPT.contains("e.button===0"));
}
#[test]
fn command_queue_applies_back_pressure_at_the_documented_cap() {
    let handle = WindowHandle::default();
    for _ in 0..MAX_QUEUED_WINDOW_COMMANDS {
        handle.send(WindowCommand::Focus).unwrap();
    }
    assert!(matches!(
        handle.send(WindowCommand::Focus),
        Err(WindowCommandError::QueueFull)
    ));
    assert_eq!(handle.drain_commands().len(), MAX_QUEUED_WINDOW_COMMANDS);
    // Draining on the UI thread must relieve the back-pressure.
    handle.send(WindowCommand::Focus).unwrap();
}
#[test]
fn host_messages() {
    assert_eq!(
        DesktopHostMessage::from_json("\"start-drag\"").unwrap(),
        DesktopHostMessage::StartDrag
    );
    assert!(DesktopHostMessage::from_json("{}").is_err());
    assert!(DesktopHostMessage::from_json(&"x".repeat(257)).is_err());
}
