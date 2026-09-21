// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Stable identity for a desktop window.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq, Hash)]
#[serde(transparent)]
pub struct WindowId(pub u64);

impl WindowId {
    /// Identity of the single window every backend opens at startup.
    ///
    /// Multi-window support is not implemented yet, so all three backends must
    /// report this exact value. Handlers can therefore compare against it
    /// portably, and a future multi-window backend can allocate subsequent ids
    /// without renumbering the primary window.
    pub const PRIMARY: Self = Self(1);
}

/// A native lifecycle event delivered on the backend UI thread.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum DesktopEvent {
    /// The native application is ready.
    Ready,
    /// A window changed size.
    WindowResized {
        window_id: WindowId,
        width: u32,
        height: u32,
    },
    /// A window moved.
    WindowMoved { window_id: WindowId, x: i32, y: i32 },
    /// A window state changed.
    WindowMaximized { window_id: WindowId },
    /// A window left the maximized state.
    WindowUnmaximized { window_id: WindowId },
    /// A window was minimized to the dock or taskbar.
    WindowMinimized { window_id: WindowId },
    /// A window was restored from the minimized state.
    WindowRestored { window_id: WindowId },
    /// A window entered native fullscreen.
    WindowEnteredFullscreen { window_id: WindowId },
    /// A window left native fullscreen.
    WindowLeftFullscreen { window_id: WindowId },
    /// A window became the key or foreground window.
    WindowFocused { window_id: WindowId },
    /// A window stopped being the key or foreground window.
    WindowBlurred { window_id: WindowId },
    /// A close request that handlers may prevent.
    WindowCloseRequested { window_id: WindowId },
    /// A window closed.
    WindowClosed { window_id: WindowId },
    /// The native theme changed.
    ThemeChanged { dark: bool },
    /// The display scale factor changed.
    ScaleFactorChanged { scale: f64 },
    /// A navigation request that handlers may prevent.
    NavigationRequested { window_id: WindowId, url: String },
    /// Navigation completed.
    NavigationCompleted { window_id: WindowId, url: String },
    /// The native application is exiting.
    Exiting,
}

/// Response returned by a lifecycle handler.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EventResponse {
    /// Allow the default native behavior to proceed.
    #[default]
    Continue,
    /// Cancel the default native behavior, for cancellable events only.
    PreventDefault,
}
/// UI-thread lifecycle callback. Handlers must not block the UI thread.
pub type EventHandler = dyn Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static;
/// Thread-safe registration and dispatch container for lifecycle callbacks.
///
/// The handler list is copy-on-write. Registration rebuilds it, and dispatch
/// clones the `Arc` and releases the lock before invoking anything, so a
/// handler is free to register another handler or dispatch a nested event
/// without re-entering a non-reentrant mutex on the UI thread.
#[derive(Clone, Default)]
pub struct EventRegistry {
    handlers: Arc<Mutex<Arc<[Arc<EventHandler>]>>>,
}
impl EventRegistry {
    /// Register a callback invoked by the backend UI thread.
    pub fn on_event<F>(&self, handler: F)
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        if let Ok(mut slot) = self.handlers.lock() {
            let mut next = Vec::with_capacity(slot.len() + 1);
            next.extend(slot.iter().map(Arc::clone));
            next.push(Arc::new(handler));
            *slot = next.into();
        }
    }
    /// Dispatch an event and report cancellation.
    ///
    /// Every handler observes the event even after one cancels it, so the
    /// outcome does not depend on registration order.
    #[must_use]
    pub fn dispatch(&self, event: &DesktopEvent) -> EventResponse {
        // Clone the list and drop the guard before invoking user callbacks.
        // Holding it across a callback would deadlock the UI thread the moment
        // a handler registered another handler or dispatched a nested event.
        let Ok(handlers) = self.handlers.lock().map(|slot| Arc::clone(&slot)) else {
            return EventResponse::Continue;
        };
        let mut response = EventResponse::Continue;
        for handler in handlers.iter() {
            if handler(event) == EventResponse::PreventDefault {
                response = EventResponse::PreventDefault;
            }
        }
        response
    }
}
/// Event JavaScript serialization error.
#[derive(Debug, Error)]
pub enum EventJavascriptError {
    #[error("failed to serialize desktop event for JavaScript: {0}")]
    Serialization(serde_json::Error),
}
impl DesktopEvent {
    /// Return the `webui:*` custom event name.
    #[must_use]
    pub const fn javascript_name(&self) -> &'static str {
        match self {
            Self::Ready => "webui:ready",
            Self::WindowResized { .. } => "webui:window-resized",
            Self::WindowMoved { .. } => "webui:window-moved",
            Self::WindowMaximized { .. } => "webui:window-maximized",
            Self::WindowUnmaximized { .. } => "webui:window-unmaximized",
            Self::WindowMinimized { .. } => "webui:window-minimized",
            Self::WindowRestored { .. } => "webui:window-restored",
            Self::WindowEnteredFullscreen { .. } => "webui:window-entered-fullscreen",
            Self::WindowLeftFullscreen { .. } => "webui:window-left-fullscreen",
            Self::WindowFocused { .. } => "webui:window-focused",
            Self::WindowBlurred { .. } => "webui:window-blurred",
            Self::WindowCloseRequested { .. } => "webui:window-close-requested",
            Self::WindowClosed { .. } => "webui:window-closed",
            Self::ThemeChanged { .. } => "webui:theme-changed",
            Self::ScaleFactorChanged { .. } => "webui:scale-factor-changed",
            Self::NavigationRequested { .. } => "webui:navigation-requested",
            Self::NavigationCompleted { .. } => "webui:navigation-completed",
            Self::Exiting => "webui:exiting",
        }
    }
    /// Serialize exact JavaScript evaluated by every backend.
    pub fn to_javascript(&self) -> Result<String, EventJavascriptError> {
        let detail = serde_json::to_string(self).map_err(EventJavascriptError::Serialization)?;
        let name = self.javascript_name();
        // `javascript_name` returns a closed set of ASCII `webui:*` literals that
        // need no escaping, so the quotes are emitted directly rather than paying
        // for a formatter or a second serde pass.
        debug_assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\'),
            "event name must be escape-free ASCII"
        );
        let mut script = String::with_capacity(detail.len() + name.len() + 54);
        script.push_str("window.dispatchEvent(new CustomEvent(\"");
        script.push_str(name);
        script.push_str("\",{detail:");
        script.push_str(&detail);
        script.push_str("}));");
        Ok(script)
    }
}
/// A command queued for execution on the native UI thread.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    /// Replace the native window title.
    SetTitle(String),
    /// Resize the window's content area, in logical pixels.
    SetSize { width: u32, height: u32 },
    /// Minimize the window to the dock or taskbar.
    Minimize,
    /// Maximize the window to fill the work area.
    Maximize,
    /// Restore the window from the maximized state.
    Unmaximize,
    /// Enter or leave native fullscreen.
    SetFullscreen(bool),
    /// Center the window on its current display.
    Center,
    /// Raise the window and give it keyboard focus.
    Focus,
    /// Close the window, firing `WindowCloseRequested` first.
    Close,
    /// Begin a host-driven window drag, used by `webui-drag` regions.
    StartDrag,
    /// Pin the window above or below other windows.
    SetAlwaysOnTop(bool),
}
/// Maximum number of commands buffered for the native UI thread before
/// `WindowHandle::send` reports back-pressure instead of growing without bound.
pub const MAX_QUEUED_WINDOW_COMMANDS: usize = 256;

/// Command queue error.
#[derive(Debug, Error)]
pub enum WindowCommandError {
    #[error(
        "desktop window command queue is full; help: wait for the UI thread to drain commands"
    )]
    QueueFull,
    #[error(
        "desktop window command queue is unavailable because the native UI thread panicked; help: restart the application"
    )]
    Unavailable,
}
/// Sendable handle that queues commands and wakes the native UI loop.
#[derive(Clone, Default)]
pub struct WindowHandle {
    inner: Arc<WindowHandleInner>,
}
struct WindowHandleInner {
    queue: Mutex<VecDeque<WindowCommand>>,
    wakeup: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
impl Default for WindowHandleInner {
    fn default() -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(32)),
            wakeup: Mutex::new(None),
        }
    }
}
impl WindowHandle {
    /// Install the backend wakeup callback.
    pub fn set_wakeup<F>(&self, wakeup: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        if let Ok(mut slot) = self.inner.wakeup.lock() {
            *slot = Some(Arc::new(wakeup));
        }
    }
    /// Drain queued commands on the UI thread.
    #[must_use]
    pub fn drain_commands(&self) -> Vec<WindowCommand> {
        let Ok(mut queue) = self.inner.queue.lock() else {
            return Vec::new();
        };
        queue.drain(..).collect()
    }
    /// Queue a native command.
    pub fn send(&self, command: WindowCommand) -> Result<(), WindowCommandError> {
        let mut q = self
            .inner
            .queue
            .lock()
            .map_err(|_| WindowCommandError::Unavailable)?;
        if q.len() >= MAX_QUEUED_WINDOW_COMMANDS {
            return Err(WindowCommandError::QueueFull);
        }
        q.push_back(command);
        drop(q);
        if let Ok(w) = self.inner.wakeup.lock() {
            if let Some(w) = w.as_ref() {
                w();
            }
        }
        Ok(())
    }
    /// Queue title update.
    pub fn set_title(&self, title: impl Into<String>) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::SetTitle(title.into()))
    }
    /// Queue size update.
    pub fn set_size(&self, width: u32, height: u32) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::SetSize { width, height })
    }
    /// Queue minimization.
    pub fn minimize(&self) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::Minimize)
    }
    /// Queue maximization.
    pub fn maximize(&self) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::Maximize)
    }
    /// Queue unmaximization.
    pub fn unmaximize(&self) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::Unmaximize)
    }
    /// Queue fullscreen change.
    pub fn fullscreen(&self, value: bool) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::SetFullscreen(value))
    }
    /// Queue centering.
    pub fn center(&self) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::Center)
    }
    /// Queue focus.
    pub fn focus(&self) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::Focus)
    }
    /// Queue closing.
    pub fn close(&self) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::Close)
    }
    /// Queue native drag.
    pub fn start_drag(&self) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::StartDrag)
    }
    /// Queue always-on-top change.
    pub fn set_always_on_top(&self, value: bool) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::SetAlwaysOnTop(value))
    }
}
/// Strictly bounded host message sent from injected web content.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopHostMessage {
    StartDrag,
    Minimize,
    ToggleMaximize,
    Close,
}
/// Maximum untrusted script-message payload size.
pub const MAX_HOST_MESSAGE_BYTES: usize = 256;
/// Host-message parsing error.
#[derive(Debug, Error)]
pub enum DesktopHostMessageError {
    #[error("desktop host message is too large: {size} bytes (max {MAX_HOST_MESSAGE_BYTES}); help: send only a supported command")]
    TooLarge { size: usize },
    #[error("invalid desktop host message; help: use start-drag, minimize, toggle-maximize, or close: {0}")]
    Invalid(serde_json::Error),
}
impl DesktopHostMessage {
    /// Parse untrusted JSON.
    pub fn from_json(input: &str) -> Result<Self, DesktopHostMessageError> {
        if input.len() > MAX_HOST_MESSAGE_BYTES {
            return Err(DesktopHostMessageError::TooLarge { size: input.len() });
        }
        serde_json::from_str(input).map_err(DesktopHostMessageError::Invalid)
    }
}
/// Shared script for `[webui-drag]` and `[webui-no-drag]` regions. Backends expose `window.webuiHostPostMessage`.
/// Script injected by every backend to translate `webui-drag` regions into host
/// messages.
///
/// The walk uses `composedPath()` rather than `target.closest()` because WebUI
/// renders through Web Components: a `document`-level listener sees shadow DOM
/// events retargeted to the host element, so `closest()` would never observe a
/// drag region declared inside a component's shadow root. The path is scanned
/// from the innermost node outward, so the nearest `webui-drag` or
/// `webui-no-drag` ancestor wins, and non-element entries such as `document`
/// and `window` are skipped.
pub const DRAG_REGION_SCRIPT: &str = "(()=>{const p=m=>window.webuiHostPostMessage&&window.webuiHostPostMessage(JSON.stringify(m));const d=e=>{const q=typeof e.composedPath==='function'?e.composedPath():[];for(let i=0;i<q.length;i++){const n=q[i];if(!n||n.nodeType!==1)continue;if(n.hasAttribute('webui-no-drag'))return false;if(n.hasAttribute('webui-drag'))return true}return false};document.addEventListener('pointerdown',e=>{if(e.button===0&&d(e))p('start-drag')});document.addEventListener('dblclick',e=>{if(e.button===0&&d(e))p('toggle-maximize')})})();";
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;
    #[test]
    fn dispatch_lets_a_handler_register_and_dispatch_without_deadlocking() {
        let registry = EventRegistry::default();
        let nested = registry.clone();
        let nested_once = Arc::new(AtomicBool::new(false));
        registry.on_event(move |_| {
            // Both calls re-enter the registry. While `dispatch` held the
            // handler mutex across callbacks, either one deadlocked the UI
            // thread against a non-reentrant mutex.
            if !nested_once.swap(true, Ordering::SeqCst) {
                nested.on_event(|_| EventResponse::Continue);
                let _ = nested.dispatch(&DesktopEvent::Ready);
            }
            EventResponse::Continue
        });
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
        registry.on_event(move |_| {
            first.fetch_add(1, Ordering::SeqCst);
            EventResponse::PreventDefault
        });
        let second = Arc::clone(&seen);
        registry.on_event(move |_| {
            second.fetch_add(1, Ordering::SeqCst);
            EventResponse::Continue
        });
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
}
