// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Toplevel window-state transitions: GDK wiring plus the pure diffing logic
//! it drives.
//!
//! [`diff_toplevel_state`] intentionally has no GTK/GDK dependency in its
//! signature so it can be unit tested on any platform, including the macOS
//! development machine this was authored on where GTK4/WebKitGTK cannot be
//! compiled.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::{gdk, prelude::*, ApplicationWindow};
use webkit6::WebView;
use webui_desktop::{DesktopEvent, EventRegistry, WindowId};

use super::backend::dispatch_event;

/// Snapshot of the toplevel bits WebUI's platform-neutral lifecycle contract
/// cares about, decoupled from `gdk::ToplevelState` so the transition logic
/// below is a pure function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ToplevelFlags {
    pub(crate) minimized: bool,
    pub(crate) maximized: bool,
    pub(crate) fullscreen: bool,
}

/// Compute the ordered `DesktopEvent`s implied by a toplevel state
/// transition.
///
/// Order is fixed (minimized/restored, then maximized/unmaximized, then
/// fullscreen enter/leave) so simultaneous bit flips still produce a
/// deterministic event sequence for handlers.
pub(crate) fn diff_toplevel_state(
    previous: ToplevelFlags,
    current: ToplevelFlags,
    window_id: WindowId,
) -> Vec<DesktopEvent> {
    let mut events = Vec::with_capacity(3);
    if previous.minimized != current.minimized {
        events.push(if current.minimized {
            DesktopEvent::WindowMinimized { window_id }
        } else {
            DesktopEvent::WindowRestored { window_id }
        });
    }
    if previous.maximized != current.maximized {
        events.push(if current.maximized {
            DesktopEvent::WindowMaximized { window_id }
        } else {
            DesktopEvent::WindowUnmaximized { window_id }
        });
    }
    if previous.fullscreen != current.fullscreen {
        events.push(if current.fullscreen {
            DesktopEvent::WindowEnteredFullscreen { window_id }
        } else {
            DesktopEvent::WindowLeftFullscreen { window_id }
        });
    }
    events
}

/// Consolidated source of truth for `WindowMinimized`/`WindowRestored`,
/// `WindowMaximized`/`WindowUnmaximized`, and
/// `WindowEnteredFullscreen`/`WindowLeftFullscreen`.
///
/// GTK's `ApplicationWindow` only realizes a `gdk::Surface` once the window
/// is shown, so the toplevel and its `state` property notification are wired
/// up on `connect_realize` rather than eagerly; `window.surface()` returns
/// `None` before that point and must not panic.
pub(crate) fn connect_toplevel_state_events(
    window: &ApplicationWindow,
    webview: &WebView,
    events: &EventRegistry,
) {
    let webview = webview.clone();
    let events = events.clone();
    let previous = Rc::new(Cell::new(ToplevelFlags::default()));
    // `realize` can fire again if the window is unrealized and re-shown.
    // Without this guard each realize would attach another `state-notify`
    // handler, so every later transition would be dispatched more than once.
    let connected = Rc::new(Cell::new(false));
    window.connect_realize(move |window| {
        if connected.replace(true) {
            return;
        }
        let Some(surface) = window.surface() else {
            connected.set(false);
            return;
        };
        let Some(toplevel) = surface.downcast_ref::<gdk::Toplevel>() else {
            connected.set(false);
            return;
        };
        // Seed the baseline from the just-realized surface so the very first
        // `notify::state` emission diffs against reality instead of the
        // all-false default, which would otherwise fabricate spurious
        // events if the window is realized already maximized or fullscreen.
        previous.set(toplevel_flags(toplevel.state()));
        let webview = webview.clone();
        let events = events.clone();
        let previous = Rc::clone(&previous);
        toplevel.connect_state_notify(move |toplevel| {
            let current = toplevel_flags(toplevel.state());
            let last = previous.replace(current);
            for event in diff_toplevel_state(last, current, WindowId::PRIMARY) {
                dispatch_event(&events, &webview, &event);
            }
        });
    });
}

/// Project the GDK toplevel state bits WebUI's neutral contract cares about
/// into the platform-independent [`ToplevelFlags`] diffed above.
fn toplevel_flags(state: gdk::ToplevelState) -> ToplevelFlags {
    ToplevelFlags {
        minimized: state.contains(gdk::ToplevelState::MINIMIZED),
        maximized: state.contains(gdk::ToplevelState::MAXIMIZED),
        fullscreen: state.contains(gdk::ToplevelState::FULLSCREEN),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW_ID: WindowId = WindowId::PRIMARY;

    #[test]
    fn no_change_emits_nothing() {
        let flags = ToplevelFlags {
            minimized: true,
            maximized: false,
            fullscreen: false,
        };
        assert!(diff_toplevel_state(flags, flags, WINDOW_ID).is_empty());
    }

    #[test]
    fn minimize_then_restore() {
        let normal = ToplevelFlags::default();
        let minimized = ToplevelFlags {
            minimized: true,
            ..normal
        };
        assert_eq!(
            diff_toplevel_state(normal, minimized, WINDOW_ID),
            vec![DesktopEvent::WindowMinimized {
                window_id: WINDOW_ID
            }]
        );
        assert_eq!(
            diff_toplevel_state(minimized, normal, WINDOW_ID),
            vec![DesktopEvent::WindowRestored {
                window_id: WINDOW_ID
            }]
        );
    }

    #[test]
    fn maximize_then_unmaximize() {
        let normal = ToplevelFlags::default();
        let maximized = ToplevelFlags {
            maximized: true,
            ..normal
        };
        assert_eq!(
            diff_toplevel_state(normal, maximized, WINDOW_ID),
            vec![DesktopEvent::WindowMaximized {
                window_id: WINDOW_ID
            }]
        );
        assert_eq!(
            diff_toplevel_state(maximized, normal, WINDOW_ID),
            vec![DesktopEvent::WindowUnmaximized {
                window_id: WINDOW_ID
            }]
        );
    }

    #[test]
    fn fullscreen_enter_and_leave() {
        let normal = ToplevelFlags::default();
        let fullscreen = ToplevelFlags {
            fullscreen: true,
            ..normal
        };
        assert_eq!(
            diff_toplevel_state(normal, fullscreen, WINDOW_ID),
            vec![DesktopEvent::WindowEnteredFullscreen {
                window_id: WINDOW_ID
            }]
        );
        assert_eq!(
            diff_toplevel_state(fullscreen, normal, WINDOW_ID),
            vec![DesktopEvent::WindowLeftFullscreen {
                window_id: WINDOW_ID
            }]
        );
    }

    #[test]
    fn simultaneous_transitions_emit_in_fixed_order() {
        let normal = ToplevelFlags::default();
        let all = ToplevelFlags {
            minimized: true,
            maximized: true,
            fullscreen: true,
        };
        assert_eq!(
            diff_toplevel_state(normal, all, WINDOW_ID),
            vec![
                DesktopEvent::WindowMinimized {
                    window_id: WINDOW_ID
                },
                DesktopEvent::WindowMaximized {
                    window_id: WINDOW_ID
                },
                DesktopEvent::WindowEnteredFullscreen {
                    window_id: WINDOW_ID
                },
            ]
        );
    }

    #[test]
    fn unrelated_bits_do_not_trigger_events() {
        // Regression: only minimized/maximized/fullscreen are part of the
        // neutral contract; other GDK toplevel state bits (sticky, tiled,
        // above, below, focused) must never synthesize a `DesktopEvent`.
        let normal = ToplevelFlags::default();
        assert!(diff_toplevel_state(normal, normal, WINDOW_ID).is_empty());
    }
}
