// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::rc::Rc;
use std::sync::atomic::AtomicUsize;
use std::task::Poll;

#[derive(Default)]
struct Signal(AtomicUsize);

impl IpcWake for Signal {
    fn wake(&self) -> Result<(), IpcError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn consumed_nested_wake_is_rescheduled_without_polling_successor_inline() {
    let signal = Arc::new(Signal::default());
    let driver = Rc::new(NativeIpcTasks::new(signal.clone(), 2));
    let ran = Rc::new(Cell::new(0));
    let weak = Rc::downgrade(&driver);
    let marked = Rc::clone(&ran);
    let nested_signal = signal.clone();
    driver
        .spawn(async move {
            let driver = weak.upgrade().unwrap();
            driver
                .spawn(async move {
                    marked.set(marked.get() + 1);
                })
                .unwrap();
            assert_eq!(nested_signal.0.swap(0, Ordering::SeqCst), 1);
            driver.poll_ready();
        })
        .unwrap();
    assert_eq!(signal.0.swap(0, Ordering::SeqCst), 1);
    driver.poll_ready();
    assert_eq!(ran.get(), 0);
    assert_eq!(driver.inner.count(), 1);
    assert_eq!(signal.0.swap(0, Ordering::SeqCst), 1);
    driver.poll_ready();
    assert_eq!(ran.get(), 1);
    assert_eq!(driver.inner.count(), 0);
    assert_eq!(signal.0.load(Ordering::SeqCst), 0);
    driver.poll_ready();
    assert_eq!(ran.get(), 1);
    assert_eq!(signal.0.load(Ordering::SeqCst), 0);
}

#[test]
fn nested_pump_preserves_running_tasks_self_wake() {
    let signal = Arc::new(Signal::default());
    let driver = Rc::new(NativeIpcTasks::new(signal.clone(), 1));
    let weak = Rc::downgrade(&driver);
    let nested_signal = signal.clone();
    let polls = Rc::new(Cell::new(0));
    let count = Rc::clone(&polls);
    driver
        .spawn(std::future::poll_fn(move |context| {
            count.set(count.get() + 1);
            if count.get() == 1 {
                context.waker().wake_by_ref();
                assert_eq!(nested_signal.0.swap(0, Ordering::SeqCst), 1);
                weak.upgrade().unwrap().poll_ready();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        }))
        .unwrap();
    signal.0.store(0, Ordering::SeqCst);
    driver.poll_ready();
    assert_eq!(polls.get(), 1);
    assert_eq!(signal.0.swap(0, Ordering::SeqCst), 1);
    driver.poll_ready();
    assert_eq!(polls.get(), 2);
    assert_eq!(signal.0.load(Ordering::SeqCst), 0);
}

#[test]
fn nested_drain_does_not_reschedule_after_close_or_without_ready_work() {
    for close in [false, true] {
        let signal = Arc::new(Signal::default());
        let driver = Rc::new(NativeIpcTasks::new(signal.clone(), 1));
        let weak = Rc::downgrade(&driver);
        driver
            .spawn(async move {
                let driver = weak.upgrade().unwrap();
                driver.poll_ready();
                if close {
                    driver.close();
                }
            })
            .unwrap();
        signal.0.store(0, Ordering::SeqCst);
        driver.poll_ready();
        assert_eq!(driver.inner.count(), 0);
        assert_eq!(signal.0.load(Ordering::SeqCst), 0);
    }
}
