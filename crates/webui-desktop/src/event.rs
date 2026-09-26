// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use thiserror::Error;

use crate::window::LiveBackground;
use crate::Rgba;

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

/// Maximum number of persistent and subscribed lifecycle callbacks.
pub const MAX_EVENT_HANDLERS: usize = 256;

/// Lifecycle callback registration error.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum EventRegistrationError {
    #[error("desktop event handler registry is full (max {MAX_EVENT_HANDLERS}); help: drop unused subscriptions before registering more handlers")]
    Capacity,
    #[error("desktop event handler registry is closed; help: register handlers before the desktop frame shuts down")]
    Closed,
    #[error("desktop event handler registry is unavailable because a callback panicked; help: restart the application")]
    Unavailable,
}

struct EventRegistration {
    id: u64,
    handler: Arc<EventHandler>,
}

#[derive(Default)]
struct EventRegistryState {
    handlers: Option<Arc<[EventRegistration]>>,
    next_id: u64,
    closed: bool,
}

/// Thread-safe registration and dispatch container for lifecycle callbacks.
///
/// Dispatch clones one `Arc` snapshot without allocating and releases the lock
/// before invoking callbacks. A dispatch already in progress may finish with
/// its original snapshot; registration and removal affect future dispatches.
/// Nested dispatch observes the current snapshot at the time it starts.
#[derive(Clone, Default)]
pub struct EventRegistry {
    inner: Arc<Mutex<EventRegistryState>>,
}

/// Removes a subscribed lifecycle callback when dropped.
///
/// The subscription holds a weak registry reference, so retaining it in state
/// owned by its callback does not create an ownership cycle.
#[must_use = "dropping the subscription immediately removes the event handler"]
pub struct EventSubscription {
    owner: std::sync::Weak<Mutex<EventRegistryState>>,
    id: u64,
}

impl Drop for EventSubscription {
    fn drop(&mut self) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        remove_event_handler(&owner, self.id);
    }
}

impl EventRegistry {
    /// Register a callback that remains installed until the registry closes.
    ///
    /// # Errors
    ///
    /// Returns [`EventRegistrationError`] when the registry reached its handler
    /// cap, has closed, or its state is unavailable after a callback panic.
    pub fn on_event<F>(&self, handler: F) -> Result<(), EventRegistrationError>
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        self.register(Arc::new(handler)).map(|_| ())
    }

    /// Register a callback removed automatically when its subscription drops.
    ///
    /// # Errors
    ///
    /// Returns [`EventRegistrationError`] when the registry reached its handler
    /// cap, has closed, or its state is unavailable after a callback panic.
    pub fn subscribe<F>(&self, handler: F) -> Result<EventSubscription, EventRegistrationError>
    where
        F: Fn(&DesktopEvent) -> EventResponse + Send + Sync + 'static,
    {
        let id = self.register(Arc::new(handler))?;
        Ok(EventSubscription {
            owner: Arc::downgrade(&self.inner),
            id,
        })
    }

    fn register(&self, handler: Arc<EventHandler>) -> Result<u64, EventRegistrationError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| EventRegistrationError::Unavailable)?;
        if state.closed {
            return Err(EventRegistrationError::Closed);
        }
        let len = state.handlers.as_ref().map_or(0, |handlers| handlers.len());
        if len >= MAX_EVENT_HANDLERS {
            return Err(EventRegistrationError::Capacity);
        }
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        let mut next = Vec::with_capacity(len + 1);
        if let Some(handlers) = state.handlers.as_ref() {
            next.extend(handlers.iter().map(|entry| EventRegistration {
                id: entry.id,
                handler: Arc::clone(&entry.handler),
            }));
        }
        next.push(EventRegistration { id, handler });
        let previous = state.handlers.replace(next.into());
        drop(state);
        drop(previous);
        Ok(id)
    }

    /// Dispatch an event and report cancellation.
    ///
    /// Every handler in the dispatch snapshot observes the event even after one
    /// cancels it, so the outcome does not depend on registration order.
    #[must_use]
    pub fn dispatch(&self, event: &DesktopEvent) -> EventResponse {
        let handlers = {
            let Ok(state) = self.inner.lock() else {
                return EventResponse::Continue;
            };
            if state.closed {
                return EventResponse::Continue;
            }
            state.handlers.as_ref().map(Arc::clone)
        };
        let Some(handlers) = handlers else {
            return EventResponse::Continue;
        };
        let mut response = EventResponse::Continue;
        for registration in handlers.iter() {
            if (registration.handler)(event) == EventResponse::PreventDefault {
                response = EventResponse::PreventDefault;
            }
        }
        response
    }

    /// Close the registry and release all callbacks.
    ///
    /// Dispatch after closure is inert. A dispatch that already cloned its
    /// snapshot may finish before those callbacks are released.
    pub(crate) fn close(&self) {
        let handlers = {
            let Ok(mut state) = self.inner.lock() else {
                return;
            };
            state.closed = true;
            state.handlers.take()
        };
        drop(handlers);
    }
}

fn remove_event_handler(owner: &Mutex<EventRegistryState>, id: u64) {
    let previous = {
        let Ok(mut state) = owner.lock() else {
            return;
        };
        let Some(handlers) = state.handlers.as_ref() else {
            return;
        };
        let Some(index) = handlers.iter().position(|entry| entry.id == id) else {
            return;
        };
        if handlers.len() == 1 {
            state.handlers.take()
        } else {
            let mut next = Vec::with_capacity(handlers.len() - 1);
            next.extend(
                handlers
                    .iter()
                    .enumerate()
                    .filter(|(entry_index, _)| *entry_index != index)
                    .map(|(_, entry)| EventRegistration {
                        id: entry.id,
                        handler: Arc::clone(&entry.handler),
                    }),
            );
            state.handlers.replace(next.into())
        }
    };
    drop(previous);
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
    /// Replace the native pre-paint and web document background.
    SetBackground(Rgba),
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
/// [`WindowHandle::send`] reports back-pressure instead of growing without bound.
pub const MAX_QUEUED_WINDOW_COMMANDS: usize = 256;
/// Maximum UTF-8 bytes accepted in one queued window title.
pub const MAX_WINDOW_TITLE_BYTES: usize = 16 * 1024;
/// Maximum aggregate UTF-8 title bytes buffered for the native UI thread.
pub const MAX_QUEUED_WINDOW_TITLE_BYTES: usize = 64 * 1024;

/// Command queue error.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum WindowCommandError {
    #[error(
        "desktop window command queue is full; help: wait for the UI thread to drain commands"
    )]
    QueueFull,
    #[error("desktop window title is too large: {size} bytes (max {MAX_WINDOW_TITLE_BYTES}); help: shorten the title before sending it")]
    TitleTooLarge { size: usize },
    #[error("desktop window title queue is full: {queued} of {MAX_QUEUED_WINDOW_TITLE_BYTES} bytes queued, {requested} more requested; help: wait for the UI thread to drain title updates")]
    TitleQueueFull { queued: usize, requested: usize },
    #[error("desktop window command queue is closed; help: stop sending commands after the desktop frame shuts down")]
    Closed,
    #[error(
        "desktop window command queue is unavailable because the native UI thread panicked; help: restart the application"
    )]
    Unavailable,
}

/// Sendable handle that queues commands and wakes the native UI loop.
#[derive(Clone, Default)]
pub struct WindowHandle {
    inner: Arc<Mutex<WindowHandleState>>,
}

#[derive(Default)]
struct WindowHandleState {
    queue: Option<VecDeque<WindowCommand>>,
    background: Option<Arc<LiveBackground>>,
    wakeup: Option<Arc<dyn Fn() + Send + Sync>>,
    queued_title_bytes: usize,
    wake_scheduled: bool,
    closed: bool,
}

impl WindowHandle {
    pub(crate) fn with_background(background: Arc<LiveBackground>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WindowHandleState {
                background: Some(background),
                ..WindowHandleState::default()
            })),
        }
    }

    /// Install the backend wakeup callback.
    ///
    /// Installing a callback while commands are queued schedules one wakeup.
    /// The callback is always invoked after releasing the queue lock.
    pub fn set_wakeup<F>(&self, wakeup: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let wakeup: Arc<dyn Fn() + Send + Sync> = Arc::new(wakeup);
        let (previous, notify) = {
            let Ok(mut state) = self.inner.lock() else {
                return;
            };
            if state.closed {
                return;
            }
            let previous = state.wakeup.replace(Arc::clone(&wakeup));
            let has_backlog = state.queue.as_ref().is_some_and(|queue| !queue.is_empty());
            let notify = has_backlog.then(|| {
                state.wake_scheduled = true;
                wakeup
            });
            (previous, notify)
        };
        drop(previous);
        if let Some(notify) = notify {
            notify();
        }
    }

    /// Drain queued commands on the UI thread.
    #[must_use]
    pub fn drain_commands(&self) -> Vec<WindowCommand> {
        let queue = {
            let Ok(mut state) = self.inner.lock() else {
                return Vec::new();
            };
            state.wake_scheduled = false;
            state.queued_title_bytes = 0;
            state.queue.take()
        };
        queue.map_or_else(Vec::new, VecDeque::into)
    }

    /// Queue a native command.
    ///
    /// Success means the command was accepted into the bounded queue, not that
    /// the native UI thread has already applied it.
    pub fn send(&self, command: WindowCommand) -> Result<(), WindowCommandError> {
        let command = normalize_command(command);
        let notify = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| WindowCommandError::Unavailable)?;
            if state.closed {
                return Err(WindowCommandError::Closed);
            }
            let (command, title_bytes) = command?;
            let queue_len = state.queue.as_ref().map_or(0, VecDeque::len);
            if queue_len >= MAX_QUEUED_WINDOW_COMMANDS {
                return Err(WindowCommandError::QueueFull);
            }
            if title_bytes > MAX_QUEUED_WINDOW_TITLE_BYTES - state.queued_title_bytes {
                return Err(WindowCommandError::TitleQueueFull {
                    queued: state.queued_title_bytes,
                    requested: title_bytes,
                });
            }
            state.queued_title_bytes += title_bytes;
            if let WindowCommand::SetBackground(color) = command {
                if let Some(background) = &state.background {
                    background.set(color);
                }
            }
            state
                .queue
                .get_or_insert_with(VecDeque::new)
                .push_back(command);
            if state.wake_scheduled {
                None
            } else if let Some(wakeup) = state.wakeup.as_ref().map(Arc::clone) {
                state.wake_scheduled = true;
                Some(wakeup)
            } else {
                None
            }
        };
        if let Some(notify) = notify {
            notify();
        }
        Ok(())
    }

    /// Close the command session and discard pending commands.
    ///
    /// The frame owner calls this during teardown. Wakeup callbacks and queued
    /// command payloads are released only after the state lock is released.
    pub(crate) fn close(&self) {
        let (wakeup, queue) = {
            let Ok(mut state) = self.inner.lock() else {
                return;
            };
            state.closed = true;
            state.wake_scheduled = false;
            state.queued_title_bytes = 0;
            (state.wakeup.take(), state.queue.take())
        };
        drop(wakeup);
        drop(queue);
    }

    /// Queue title update.
    pub fn set_title(&self, title: impl Into<String>) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::SetTitle(title.into()))
    }
    /// Queue a native and document background update without changing the bundle manifest.
    pub fn set_background(&self, color: Rgba) -> Result<(), WindowCommandError> {
        self.send(WindowCommand::SetBackground(color))
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
    /// Queue a native close request.
    pub fn request_close(&self) -> Result<(), WindowCommandError> {
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

fn normalize_command(command: WindowCommand) -> Result<(WindowCommand, usize), WindowCommandError> {
    let WindowCommand::SetTitle(title) = command else {
        return Ok((command, 0));
    };
    let size = title.len();
    if size > MAX_WINDOW_TITLE_BYTES {
        return Err(WindowCommandError::TitleTooLarge { size });
    }
    let title = if title.capacity() == size {
        title
    } else {
        title.into_boxed_str().into_string()
    };
    Ok((WindowCommand::SetTitle(title), size))
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
/// and `window` are skipped. WebView2 waits for pointer movement before
/// starting a native drag, since its move loop would consume a double-click.
pub const DRAG_REGION_SCRIPT: &str = concat!(
    "(()=>{const p=m=>window.webuiHostPostMessage&&window.webuiHostPostMessage(JSON.stringify(m));",
    "const d=e=>{const q=typeof e.composedPath==='function'?e.composedPath():[];",
    "for(let i=0;i<q.length;i++){const n=q[i];if(!n||n.nodeType!==1)continue;",
    "if(n.hasAttribute('webui-no-drag'))return false;if(n.hasAttribute('webui-drag'))return true}return false};",
    "let a=null,w=!!window.chrome?.webview;document.addEventListener('pointerdown',e=>{a=null;",
    "if(e.button!==0||!d(e))return;if(!w){p('start-drag');return}",
    "a=[e.pointerId,e.screenX,e.screenY]});",
    "if(w){document.addEventListener('pointermove',e=>{",
    "if(!a||e.pointerId!==a[0])return;if(!(e.buttons&1)){a=null;return}",
    "if(Math.abs(e.screenX-a[1])<4&&Math.abs(e.screenY-a[2])<4)return;",
    "a=null;p('start-drag')});",
    "const end=e=>{if(a&&e.pointerId===a[0])a=null};",
    "document.addEventListener('pointerup',end);document.addEventListener('pointercancel',end);",
    "window.addEventListener('blur',()=>{a=null})}",
    "document.addEventListener('dblclick',e=>{if(e.button===0&&d(e))p('toggle-maximize')})})();"
);
#[cfg(test)]
#[path = "event_tests.rs"]
mod tests;
