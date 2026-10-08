//! The Settings window's toolbar: a real AppKit preferences toolbar with SF Symbols, so the
//! pane switcher looks and behaves exactly like System Settings-style windows. Clicking an item
//! emits `settings-pane` to the page; the page reports its pane back with `select_settings_pane`.
//! While a sheet is open the page disables the toolbar with `set_settings_toolbar_enabled`; the
//! items are then greyed out and `select` leaves the highlighted pane as it is.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSImage, NSToolbar, NSToolbarDelegate, NSToolbarDisplayMode, NSToolbarItem,
    NSToolbarItemIdentifier, NSWindow, NSWindowToolbarStyle,
};
use objc2_foundation::{NSArray, NSString};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::windows::{SettingsPane, SETTINGS_LABEL};

pub const PANE_EVENT: &str = "settings-pane";

pub struct Ivars {
    app: AppHandle,
    /// Whether the toolbar items accept clicks and `select` changes the highlighted pane.
    enabled: Cell<bool>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and this class does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "VigiaSettingsToolbarDelegate"]
    #[ivars = Ivars]
    pub struct ToolbarDelegate;

    impl ToolbarDelegate {
        #[unsafe(method(selectPane:))]
        fn select_pane(&self, sender: &NSToolbarItem) {
            let id = sender.itemIdentifier().to_string();
            let Some(pane) = selection(self.ivars().enabled.get(), &id) else {
                return;
            };
            let _ = self.ivars().app.emit_to(SETTINGS_LABEL, PANE_EVENT, pane.as_str());
        }

        /// Toolbar validation re-enables items whose target answers yes, so the answer follows
        /// the enabled flag.
        #[unsafe(method(validateToolbarItem:))]
        fn validate_toolbar_item(&self, _item: &NSToolbarItem) -> bool {
            self.ivars().enabled.get()
        }
    }

    unsafe impl NSObjectProtocol for ToolbarDelegate {}

    unsafe impl NSToolbarDelegate for ToolbarDelegate {
        #[unsafe(method_id(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
        fn item_for_identifier(
            &self,
            _toolbar: &NSToolbar,
            identifier: &NSToolbarItemIdentifier,
            _will_insert: bool,
        ) -> Option<Retained<NSToolbarItem>> {
            self.make_item(identifier)
        }

        #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
        fn default_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            identifiers()
        }

        #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
        fn allowed_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            identifiers()
        }

        #[unsafe(method_id(toolbarSelectableItemIdentifiers:))]
        fn selectable_identifiers(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSToolbarItemIdentifier>> {
            identifiers()
        }
    }
);

impl ToolbarDelegate {
    fn make_item(&self, identifier: &NSToolbarItemIdentifier) -> Option<Retained<NSToolbarItem>> {
        let pane: SettingsPane = identifier.to_string().parse().ok()?;
        let label = NSString::from_str(pane.title());
        let item =
            NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(self.mtm()), identifier);
        item.setLabel(&label);
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(pane.symbol()),
            Some(&label),
        );
        item.setImage(image.as_deref());
        // SAFETY: the delegate outlives the toolbar (kept in DELEGATE) and responds to the selector.
        unsafe {
            item.setTarget(Some(self as &AnyObject));
            item.setAction(Some(sel!(selectPane:)));
        }
        Some(item)
    }

    fn new(mtm: MainThreadMarker, app: AppHandle) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            app,
            enabled: Cell::new(true),
        });
        // SAFETY: NSObject's init takes no arguments and returns an initialized instance.
        unsafe { msg_send![super(this), init] }
    }
}

fn identifiers() -> Retained<NSArray<NSToolbarItemIdentifier>> {
    let ids: Vec<Retained<NSString>> = SettingsPane::ALL
        .iter()
        .map(|pane| NSString::from_str(pane.as_str()))
        .collect();
    NSArray::from_retained_slice(&ids)
}

thread_local! {
    // The toolbar only holds its delegate weakly, so the delegate is kept alive here.
    static DELEGATE: RefCell<Option<Retained<ToolbarDelegate>>> = const { RefCell::new(None) };
}

fn ns_window(window: &WebviewWindow) -> Option<&NSWindow> {
    let ptr = window.ns_window().ok()?;
    // SAFETY: Tauri returns the window's live NSWindow pointer; it is used on the main thread
    // while the window exists.
    unsafe { (ptr as *const NSWindow).as_ref() }
}

/// The pane a selection request switches to: the pane named by `id`, while the toolbar is
/// enabled.
fn selection(enabled: bool, id: &str) -> Option<SettingsPane> {
    if !enabled {
        return None;
    }
    id.parse().ok()
}

/// Whether the toolbar of the open Settings window is enabled; true when there is none.
fn toolbar_enabled() -> bool {
    DELEGATE.with(|cell| {
        cell.borrow()
            .as_ref()
            .is_none_or(|delegate| delegate.ivars().enabled.get())
    })
}

/// Installs the preferences toolbar on the Settings window. Must run on the main thread.
pub fn install(app: &AppHandle, window: &WebviewWindow, selected: SettingsPane) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::error!("settings toolbar must be installed on the main thread");
        return;
    };
    let Some(ns_window) = ns_window(window) else {
        return;
    };
    let delegate = ToolbarDelegate::new(mtm, app.clone());
    let toolbar =
        NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), &NSString::from_str("VigiaSettings"));
    toolbar.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    toolbar.setDisplayMode(NSToolbarDisplayMode::IconAndLabel);
    toolbar.setAllowsUserCustomization(false);
    toolbar.setSelectedItemIdentifier(Some(&NSString::from_str(selected.as_str())));
    ns_window.setToolbar(Some(&toolbar));
    ns_window.setToolbarStyle(NSWindowToolbarStyle::Preference);
    log::debug!(
        "settings toolbar installed with {} items",
        toolbar.items().len()
    );
    DELEGATE.with(|cell| *cell.borrow_mut() = Some(delegate));
}

/// Drops the toolbar delegate once the Settings window is gone. Must run on the main thread.
pub fn release() {
    DELEGATE.with(|cell| *cell.borrow_mut() = None);
}

/// Highlights `pane` in the toolbar, unless the toolbar is disabled. Must run on the main thread.
pub fn select(window: &WebviewWindow, pane: SettingsPane) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let Some(pane) = selection(toolbar_enabled(), pane.as_str()) else {
        return;
    };
    if let Some(toolbar) = ns_window(window).and_then(|w| w.toolbar()) {
        toolbar.setSelectedItemIdentifier(Some(&NSString::from_str(pane.as_str())));
    }
}

/// Runs `select` on the main thread for the Settings window, if it is open.
pub fn select_on_main(app: &AppHandle, pane: SettingsPane) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(SETTINGS_LABEL) {
            select(&window, pane);
        }
    });
}

/// Enables or disables every toolbar item of the Settings window on the main thread. Does
/// nothing when the window or its toolbar does not exist.
pub fn set_enabled_on_main(app: &AppHandle, enabled: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(window) = handle.get_webview_window(SETTINGS_LABEL) else {
            return;
        };
        let Some(toolbar) = ns_window(&window).and_then(|w| w.toolbar()) else {
            return;
        };
        DELEGATE.with(|cell| {
            if let Some(delegate) = cell.borrow().as_ref() {
                delegate.ivars().enabled.set(enabled);
            }
        });
        for item in toolbar.items().iter() {
            item.setEnabled(enabled);
        }
        toolbar.validateVisibleItems();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_enabled_toolbar_selects_known_panes() {
        for pane in SettingsPane::ALL {
            assert_eq!(selection(true, pane.as_str()), Some(pane));
        }
        assert_eq!(selection(true, "filters"), None);
    }

    #[test]
    fn a_disabled_toolbar_selects_nothing() {
        for pane in SettingsPane::ALL {
            assert_eq!(selection(false, pane.as_str()), None);
        }
    }

    #[test]
    fn toolbar_counts_as_enabled_without_a_settings_window() {
        assert!(toolbar_enabled());
    }
}
