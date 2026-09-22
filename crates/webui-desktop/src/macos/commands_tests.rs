// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

#[test]
fn wake_targets_its_owner_without_any_focused_window_or_fallback() {
    let first = Rc::new(Cell::new(0));
    let second = Rc::new(Cell::new(0));
    let count = Rc::clone(&first);
    let owner = register_target(Rc::new(move || count.set(count.get() + 1)));
    let count = Rc::clone(&second);
    let other = register_target(Rc::new(move || count.set(count.get() + 1)));
    drain_target(owner.id);
    assert_eq!(first.get(), 1);
    assert_eq!(second.get(), 0);
    owner.close();
    drain_target(owner.id);
    assert_eq!(first.get(), 1);
    assert_eq!(second.get(), 0);
    drain_target(other.id);
    assert_eq!(second.get(), 1);
}

#[test]
fn callback_can_detach_its_target_without_a_registry_borrow() {
    let slot = Rc::new(RefCell::new(None::<std::rc::Weak<CommandWake>>));
    let inside = Rc::clone(&slot);
    let target = Rc::new(register_target(Rc::new(move || {
        let target = inside.borrow().as_ref().and_then(std::rc::Weak::upgrade);
        if let Some(target) = target {
            target.close();
        }
    })));
    *slot.borrow_mut() = Some(Rc::downgrade(&target));
    drain_target(target.id);
    assert!(target.closed.get());
    drain_target(target.id);
}
