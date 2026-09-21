// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Attach the Windows command wakeup only to an installed native receiver.

use anyhow::{Context, Result};

use crate::WindowHandle;

pub(super) fn attach<F>(receiver: Option<WindowHandle>, wake: F) -> Result<()>
where
    F: Fn() + Send + Sync + 'static,
{
    let receiver = receiver.context(
        "cannot attach a Windows command wakeup before FrameState is installed; attach it after set_window_state",
    )?;
    receiver.set_wakeup(wake);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::{WindowCommand, WindowHandle};

    #[test]
    fn receiver_guard_preserves_backlog_until_attachment_and_drain() {
        let handle = WindowHandle::default();
        let pending_wakes = Arc::new(AtomicUsize::new(0));
        handle.set_title("queued before initialization").unwrap();

        let wake_count = Arc::clone(&pending_wakes);
        assert!(super::attach(None, move || {
            wake_count.fetch_add(1, Ordering::SeqCst);
        })
        .is_err());
        // A nested initialization pump has no wake to consume before a receiver
        // exists. The production channel must retain both accepted commands.
        assert_eq!(pending_wakes.swap(0, Ordering::SeqCst), 0);
        handle.minimize().unwrap();
        assert_eq!(pending_wakes.load(Ordering::SeqCst), 0);

        let wake_count = Arc::clone(&pending_wakes);
        super::attach(Some(handle.clone()), move || {
            wake_count.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert_eq!(pending_wakes.swap(0, Ordering::SeqCst), 1);
        let commands = handle.drain_commands();
        assert_eq!(commands.len(), 2);
        assert!(matches!(
            &commands[0],
            WindowCommand::SetTitle(title) if title == "queued before initialization"
        ));
        assert!(matches!(commands[1], WindowCommand::Minimize));

        handle.set_title("queued after startup drain").unwrap();
        assert_eq!(pending_wakes.load(Ordering::SeqCst), 1);
    }
}
