// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native application menu bar construction and command dispatch.

use crate::DesktopMenu;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
/// Own both the menu and its command receiver: AppKit menu targets are weak.
#[must_use = "retain the native menu owner until the window session ends"]
pub(super) struct NativeMenu {
    menu: Retained<NSMenu>,
    _target: Option<Retained<DesktopMenuTarget>>,
}

impl NativeMenu {
    pub(super) fn menu(&self) -> &NSMenu {
        &self.menu
    }
}

/// Build the application's main menu.
///
/// When `menus` is empty, a minimal default App/Edit menu is installed so the
/// app can quit (`Cmd+Q`) and use standard text-editing shortcuts (`Cmd+C`,
/// `Cmd+V`, ...) even though the manifest declared no menu bar.
pub(super) fn build_main_menu<F>(
    mtm: MainThreadMarker,
    menus: &[DesktopMenu],
    dispatch: F,
) -> NativeMenu
where
    F: Fn(&str) + 'static,
{
    let bar = NSMenu::new(mtm);
    if menus.is_empty() {
        bar.addItem(&submenu_item(mtm, &default_app_menu(mtm)));
        bar.addItem(&submenu_item(mtm, &default_edit_menu(mtm)));
        return NativeMenu {
            menu: bar,
            _target: None,
        };
    }
    let target = DesktopMenuTarget::new(mtm, Box::new(dispatch));
    for menu in menus {
        bar.addItem(&submenu_item(mtm, &build_menu(mtm, menu, &target)));
    }
    NativeMenu {
        menu: bar,
        _target: Some(target),
    }
}

fn submenu_item(mtm: MainThreadMarker, submenu: &NSMenu) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setSubmenu(Some(submenu));
    item
}

fn build_menu(
    mtm: MainThreadMarker,
    menu: &DesktopMenu,
    target: &Retained<DesktopMenuTarget>,
) -> Retained<NSMenu> {
    let native = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(&menu.label));
    for item in &menu.items {
        let native_item = NSMenuItem::new(mtm);
        native_item.setTitle(&NSString::from_str(&item.label));
        if let Some(accelerator) = &item.accelerator {
            apply_accelerator(&native_item, accelerator);
        }
        if let Some(command) = &item.command {
            let index = target.push_command(command.clone());
            native_item.setTag(index);
            // SAFETY: `target` is retained by the returned NativeMenu owner,
            // and `performDesktopMenuCommand:` matches the selector implemented
            // on `DesktopMenuTarget` below.
            unsafe {
                native_item.setTarget(Some(target));
                native_item.setAction(Some(sel!(performDesktopMenuCommand:)));
            }
        }
        native.addItem(&native_item);
    }
    native
}

fn default_app_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    let app_name = process_name();
    menu.addItem(&standard_item(
        mtm,
        &format!("About {app_name}"),
        sel!(orderFrontStandardAboutPanel:),
        "",
    ));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&standard_item(
        mtm,
        &format!("Hide {app_name}"),
        sel!(hide:),
        "h",
    ));
    menu.addItem(&standard_item(
        mtm,
        "Hide Others",
        sel!(hideOtherApplications:),
        "h",
    ));
    menu.addItem(&standard_item(
        mtm,
        "Show All",
        sel!(unhideAllApplications:),
        "",
    ));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&standard_item(
        mtm,
        &format!("Quit {app_name}"),
        sel!(terminate:),
        "q",
    ));
    menu
}

fn default_edit_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    menu.addItem(&standard_item(mtm, "Undo", sel!(undo:), "z"));
    menu.addItem(&standard_item(mtm, "Redo", sel!(redo:), "Z"));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&standard_item(mtm, "Cut", sel!(cut:), "x"));
    menu.addItem(&standard_item(mtm, "Copy", sel!(copy:), "c"));
    menu.addItem(&standard_item(mtm, "Paste", sel!(paste:), "v"));
    menu.addItem(&standard_item(mtm, "Select All", sel!(selectAll:), "a"));
    menu
}

fn standard_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key_equivalent: &str,
) -> Retained<NSMenuItem> {
    // SAFETY: `initWithTitle:action:keyEquivalent:` only stores the given
    // title, selector, and key string; the action's target stays `nil` and
    // is resolved by AppKit's responder chain at invocation time.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key_equivalent),
        )
    }
}

/// Parse a platform-neutral accelerator like `"CmdOrCtrl+Shift+R"` into an
/// AppKit key equivalent and modifier mask, applied in place.
fn apply_accelerator(item: &NSMenuItem, accelerator: &str) {
    let mut mask = NSEventModifierFlags::empty();
    let mut key = String::new();
    for part in accelerator.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "cmd" | "cmdorctrl" | "command" | "super" => mask |= NSEventModifierFlags::Command,
            "ctrl" | "control" => mask |= NSEventModifierFlags::Control,
            "alt" | "option" => mask |= NSEventModifierFlags::Option,
            "shift" => mask |= NSEventModifierFlags::Shift,
            other => key = other.to_string(),
        }
    }
    if key.is_empty() {
        return;
    }
    item.setKeyEquivalent(&NSString::from_str(&key));
    item.setKeyEquivalentModifierMask(mask);
}

fn process_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "App".to_string())
}

struct MenuTargetIvars {
    dispatch: Box<dyn Fn(&str)>,
    commands: std::cell::RefCell<Vec<String>>,
}

define_class!(
    // SAFETY: Target is an NSObject subclass with no Drop implementation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTargetIvars]
    struct DesktopMenuTarget;

    // SAFETY: NSObjectProtocol has no additional safety requirements.
    unsafe impl NSObjectProtocol for DesktopMenuTarget {}

    impl DesktopMenuTarget {
        #[unsafe(method(performDesktopMenuCommand:))]
        fn perform_desktop_menu_command(&self, sender: &NSMenuItem) {
            let ivars = self.ivars();
            let tag = usize::try_from(sender.tag()).unwrap_or(usize::MAX);
            let Some(command) = ivars.commands.borrow().get(tag).cloned() else {
                return;
            };
            let script = menu_command_script(&command);
            (ivars.dispatch)(&script);
        }
    }
);

impl DesktopMenuTarget {
    fn new(mtm: MainThreadMarker, dispatch: Box<dyn Fn(&str)>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuTargetIvars {
            dispatch,
            commands: std::cell::RefCell::new(Vec::new()),
        });
        // SAFETY: NSObject init has the expected signature for this subclass.
        unsafe { msg_send![super(this), init] }
    }

    /// Register a command string and return its stable tag index.
    fn push_command(&self, command: String) -> objc2::ffi::NSInteger {
        let mut commands = self.ivars().commands.borrow_mut();
        commands.push(command);
        objc2::ffi::NSInteger::try_from(commands.len() - 1).unwrap_or(0)
    }
}

/// Build the JS dispatched for a menu item's `command` id.
///
/// Apps listen for `window.addEventListener("webui:menu-command", ...)` and
/// read `event.detail.id` to route the action.
#[must_use]
fn menu_command_script(command: &str) -> String {
    let id = serde_json::to_string(command).unwrap_or_else(|_| "\"\"".to_string());
    format!("window.dispatchEvent(new CustomEvent(\"webui:menu-command\",{{detail:{{id:{id}}}}}));")
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    #[test]
    fn command_script_embeds_id_as_json() {
        let script = super::menu_command_script("open-preferences");
        assert!(script.contains("webui:menu-command"));
        assert!(script.contains("id:\"open-preferences\""));
    }
}
