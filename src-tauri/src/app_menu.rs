//! The macOS menu bar, shown while Settings is open and the app runs under the Regular
//! activation policy.
//!
//! Settings… and Quit Vigia reuse the status item's menu ids, so the tray's menu handler runs
//! them; this module handles the rest. Linux and Windows have no app menu, and Settings zooms
//! there from its own key handler.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

use crate::zoom::{self, ZoomLevels, ZoomStep};

const ZOOM_RESET_ID: &str = "zoom-reset";
const ZOOM_IN_ID: &str = "zoom-in";
const ZOOM_OUT_ID: &str = "zoom-out";
const DELETE_ID: &str = "edit-delete";
const HELP_ID: &str = "help";
/// Where Vigia Help opens: the user guide in the README.
pub const HELP_URL: &str = "https://github.com/s3rgiosan/vigia#readme";

/// What an app menu item does, by its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMenuAction {
    Zoom(ZoomStep),
    Delete,
    Help,
}

impl AppMenuAction {
    fn from_id(id: &str) -> Option<AppMenuAction> {
        match id {
            ZOOM_RESET_ID => Some(AppMenuAction::Zoom(ZoomStep::Reset)),
            ZOOM_IN_ID => Some(AppMenuAction::Zoom(ZoomStep::In)),
            ZOOM_OUT_ID => Some(AppMenuAction::Zoom(ZoomStep::Out)),
            DELETE_ID => Some(AppMenuAction::Delete),
            HELP_ID => Some(AppMenuAction::Help),
            _ => None,
        }
    }
}

/// Builds the menu bar: App, File, Edit, View, Window and Help.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let version = app.package_info().version.to_string();
    let separator = || PredefinedMenuItem::separator(app);
    let app_menu = Submenu::with_items(
        app,
        "Vigia",
        true,
        &[
            &PredefinedMenuItem::about(
                app,
                Some("About Vigia"),
                Some(crate::tray::about_metadata(version)),
            )?,
            &separator()?,
            &MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?,
            &separator()?,
            &PredefinedMenuItem::services(app, None)?,
            &separator()?,
            &PredefinedMenuItem::hide(app, Some("Hide Vigia"))?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &separator()?,
            &MenuItem::with_id(app, "quit", "Quit Vigia", true, Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let file_menu = Submenu::with_items(
        app,
        "File",
        true,
        &[&PredefinedMenuItem::close_window(app, None)?],
    )?;
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &separator()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &MenuItem::with_id(app, DELETE_ID, "Delete", true, None::<&str>)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &MenuItem::with_id(app, ZOOM_RESET_ID, "Actual Size", true, Some("CmdOrCtrl+0"))?,
            &MenuItem::with_id(app, ZOOM_IN_ID, "Zoom In", true, Some("CmdOrCtrl+="))?,
            &MenuItem::with_id(app, ZOOM_OUT_ID, "Zoom Out", true, Some("CmdOrCtrl+-"))?,
        ],
    )?;
    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &separator()?,
            &PredefinedMenuItem::bring_all_to_front(app, None)?,
        ],
    )?;
    let help_menu = Submenu::with_items(
        app,
        "Help",
        true,
        &[&MenuItem::with_id(
            app,
            HELP_ID,
            "Vigia Help",
            true,
            None::<&str>,
        )?],
    )?;
    Menu::with_items(
        app,
        &[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ],
    )
}

/// Runs the app menu items this module owns; other ids are left to their handlers.
pub fn handle_event<R: Runtime>(app: &AppHandle<R>, event: &MenuEvent) {
    let Some(action) = AppMenuAction::from_id(event.id().as_ref()) else {
        return;
    };
    match action {
        AppMenuAction::Zoom(step) => {
            if let Some(window) = focused_window(app) {
                zoom::apply(&app.state::<ZoomLevels>(), &window, step);
            }
        }
        AppMenuAction::Delete => {
            if let Some(window) = focused_window(app) {
                if let Err(e) = window.eval("document.execCommand('delete')") {
                    log::warn!("delete could not run: {e}");
                }
            }
        }
        AppMenuAction::Help => {
            use tauri_plugin_opener::OpenerExt;
            if let Err(e) = app.opener().open_url(HELP_URL, None::<&str>) {
                log::warn!("help could not be opened: {e}");
            }
        }
    }
}

/// The window with keyboard focus, if any.
fn focused_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.webview_windows()
        .into_values()
        .find(|window| window.is_focused().unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_map_to_actions() {
        assert_eq!(
            AppMenuAction::from_id("zoom-reset"),
            Some(AppMenuAction::Zoom(ZoomStep::Reset))
        );
        assert_eq!(
            AppMenuAction::from_id("zoom-in"),
            Some(AppMenuAction::Zoom(ZoomStep::In))
        );
        assert_eq!(
            AppMenuAction::from_id("zoom-out"),
            Some(AppMenuAction::Zoom(ZoomStep::Out))
        );
        assert_eq!(
            AppMenuAction::from_id("edit-delete"),
            Some(AppMenuAction::Delete)
        );
        assert_eq!(AppMenuAction::from_id("help"), Some(AppMenuAction::Help));
    }

    #[test]
    fn tray_ids_are_left_to_the_tray() {
        for id in ["settings", "quit", "open", "refresh"] {
            assert_eq!(AppMenuAction::from_id(id), None);
        }
    }
}
