//! The Settings window: created on demand, focused when it already exists.
//!
//! Under the Accessory activation policy a new window may not receive keyboard focus, so the
//! policy switches to Regular while Settings is open and back to Accessory when it closes.
//! The window has a native preferences toolbar (see `settings_toolbar`).

use std::fmt;
use std::str::FromStr;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use url::form_urlencoded;

pub const SETTINGS_LABEL: &str = "settings";
/// Sent to an open Settings window with the pane and sheet it should show.
pub const SETTINGS_OPEN_EVENT: &str = "settings-open";
const SETTINGS_WIDTH: f64 = 680.0;
const SETTINGS_HEIGHT: f64 = 560.0;

/// The value is not one of the known ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownId;

impl fmt::Display for UnknownId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown id")
    }
}

impl std::error::Error for UnknownId {}

/// A Settings pane: its id in URLs, events and toolbar items, its title and its SF Symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsPane {
    Accounts,
    Repos,
    /// Titled Filters; the id stays `branches`.
    Branches,
    General,
}

impl SettingsPane {
    /// Every pane, in toolbar order.
    pub const ALL: [SettingsPane; 4] = [
        SettingsPane::Accounts,
        SettingsPane::Repos,
        SettingsPane::Branches,
        SettingsPane::General,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SettingsPane::Accounts => "accounts",
            SettingsPane::Repos => "repos",
            SettingsPane::Branches => "branches",
            SettingsPane::General => "general",
        }
    }

    /// The toolbar label, also used as the window title.
    pub fn title(self) -> &'static str {
        match self {
            SettingsPane::Accounts => "Accounts",
            SettingsPane::Repos => "Repositories",
            SettingsPane::Branches => "Filters",
            SettingsPane::General => "General",
        }
    }

    /// The SF Symbol of the toolbar item.
    pub fn symbol(self) -> &'static str {
        match self {
            SettingsPane::Accounts => "person.crop.circle",
            SettingsPane::Repos => "folder",
            SettingsPane::Branches => "line.3.horizontal.decrease.circle",
            SettingsPane::General => "gearshape",
        }
    }
}

impl FromStr for SettingsPane {
    type Err = UnknownId;

    fn from_str(id: &str) -> Result<SettingsPane, UnknownId> {
        SettingsPane::ALL
            .into_iter()
            .find(|pane| pane.as_str() == id)
            .ok_or(UnknownId)
    }
}

/// A sheet Settings can open over the Accounts pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsSheet {
    /// Add Account.
    Add,
    /// Replace Token, for one account.
    Replace,
}

impl SettingsSheet {
    pub const ALL: [SettingsSheet; 2] = [SettingsSheet::Add, SettingsSheet::Replace];

    pub fn as_str(self) -> &'static str {
        match self {
            SettingsSheet::Add => "add",
            SettingsSheet::Replace => "replace",
        }
    }
}

impl FromStr for SettingsSheet {
    type Err = UnknownId;

    fn from_str(id: &str) -> Result<SettingsSheet, UnknownId> {
        SettingsSheet::ALL
            .into_iter()
            .find(|sheet| sheet.as_str() == id)
            .ok_or(UnknownId)
    }
}

/// Where Settings opens: a pane, and a sheet over it with the account it applies to. Each part
/// is kept only when valid. Serialized, it is the `settings-open` event payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SettingsTarget {
    pub pane: Option<SettingsPane>,
    pub sheet: Option<SettingsSheet>,
    pub account_id: Option<String>,
}

impl SettingsTarget {
    /// Keeps a known pane and sheet and drops anything else. A Replace sheet needs an account
    /// for which `account_exists` holds; without one the sheet and the account are dropped. The
    /// account is kept only for a Replace sheet.
    pub fn new(
        pane: Option<&str>,
        sheet: Option<&str>,
        account_id: Option<&str>,
        account_exists: impl Fn(&str) -> bool,
    ) -> SettingsTarget {
        let pane = pane.and_then(|p| p.parse().ok());
        let sheet = sheet.and_then(|s| s.parse().ok());
        let known_account = account_id.filter(|id| account_exists(id));
        let (sheet, account_id) = match (sheet, known_account) {
            (Some(SettingsSheet::Replace), Some(id)) => {
                (Some(SettingsSheet::Replace), Some(id.to_string()))
            }
            (Some(SettingsSheet::Replace), None) => (None, None),
            (sheet, _) => (sheet, None),
        };
        SettingsTarget {
            pane,
            sheet,
            account_id,
        }
    }

    /// The page URL for a new Settings window.
    pub fn url(&self) -> String {
        let mut query = form_urlencoded::Serializer::new(String::new());
        query.append_pair("view", "settings");
        if let Some(pane) = self.pane {
            query.append_pair("pane", pane.as_str());
        }
        if let Some(sheet) = self.sheet {
            query.append_pair("sheet", sheet.as_str());
        }
        if let Some(account) = &self.account_id {
            query.append_pair("account", account);
        }
        format!("index.html?{}", query.finish())
    }

    /// The pane the window shows first.
    pub fn initial_pane(&self) -> SettingsPane {
        self.pane.unwrap_or(SettingsPane::Accounts)
    }
}

/// Enables or disables launch at login to match the setting.
pub fn sync_autostart(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let current = manager.is_enabled().unwrap_or(false);
    let result = match autostart_change(current, enabled) {
        AutostartChange::Keep => return,
        AutostartChange::Enable => manager.enable(),
        AutostartChange::Disable => manager.disable(),
    };
    if let Err(e) = result {
        log::warn!("launch at login could not be changed: {e}");
    }
}

/// What `sync_autostart` changes to bring launch at login to the setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutostartChange {
    Keep,
    Enable,
    Disable,
}

fn autostart_change(current: bool, wanted: bool) -> AutostartChange {
    match (current, wanted) {
        (false, true) => AutostartChange::Enable,
        (true, false) => AutostartChange::Disable,
        _ => AutostartChange::Keep,
    }
}

/// Opens the Settings window at `target`, or focuses it and sends `SETTINGS_OPEN_EVENT` when it
/// exists. The check and the build run on the main thread, so concurrent calls cannot both try
/// to create the window.
pub fn open_settings(app: &AppHandle, target: SettingsTarget) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || open_settings_on_main(&handle, target)) {
        log::error!("could not schedule opening settings: {e}");
    }
}

fn open_settings_on_main(app: &AppHandle, target: SettingsTarget) {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        #[cfg(target_os = "macos")]
        if let Some(pane) = target.pane {
            crate::settings_toolbar::select(&window, pane);
        }
        if let Err(e) = window.emit_to(SETTINGS_LABEL, SETTINGS_OPEN_EVENT, &target) {
            log::warn!("settings target not sent: {e}");
        }
        return;
    }

    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let built =
        WebviewWindowBuilder::new(app, SETTINGS_LABEL, WebviewUrl::App(target.url().into()))
            .title(target.initial_pane().title())
            .inner_size(SETTINGS_WIDTH, SETTINGS_HEIGHT)
            .resizable(false)
            .minimizable(true)
            .maximizable(false)
            .build();

    match built {
        Ok(window) => {
            let handle = app.clone();
            window.on_window_event(move |event| {
                if let WindowEvent::Destroyed = event {
                    #[cfg(target_os = "macos")]
                    {
                        let app = handle.clone();
                        let _ = handle.run_on_main_thread(move || {
                            crate::settings_toolbar::release();
                            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                        });
                    }
                    #[cfg(not(target_os = "macos"))]
                    let _ = &handle;
                }
            });
            // The pane switcher is a native preferences toolbar.
            #[cfg(target_os = "macos")]
            crate::settings_toolbar::install(app, &window, target.initial_pane());
            let _ = window.set_focus();
        }
        Err(e) => {
            log::error!("could not open settings: {e}");
            #[cfg(target_os = "macos")]
            if app.get_webview_window(SETTINGS_LABEL).is_none() {
                let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none_exist(_: &str) -> bool {
        false
    }

    fn target(pane: Option<&str>, sheet: Option<&str>) -> SettingsTarget {
        SettingsTarget::new(pane, sheet, None, none_exist)
    }

    #[test]
    fn pane_ids_round_trip() {
        for pane in SettingsPane::ALL {
            assert_eq!(pane.as_str().parse::<SettingsPane>(), Ok(pane));
        }
        let ids: Vec<&str> = SettingsPane::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(ids, ["accounts", "repos", "branches", "general"]);
    }

    #[test]
    fn sheet_ids_round_trip() {
        for sheet in SettingsSheet::ALL {
            assert_eq!(sheet.as_str().parse::<SettingsSheet>(), Ok(sheet));
        }
        assert_eq!("add".parse(), Ok(SettingsSheet::Add));
        assert_eq!("replace".parse(), Ok(SettingsSheet::Replace));
    }

    #[test]
    fn unknown_ids_are_rejected() {
        for id in ["", "Repos", "filters", "../secrets", "accounts "] {
            assert_eq!(id.parse::<SettingsPane>(), Err(UnknownId));
        }
        for id in ["", "Add", "delete", "add&view=popup"] {
            assert_eq!(id.parse::<SettingsSheet>(), Err(UnknownId));
        }
    }

    #[test]
    fn every_pane_has_its_toolbar_title_and_symbol() {
        assert_eq!(SettingsPane::Accounts.title(), "Accounts");
        assert_eq!(SettingsPane::Repos.title(), "Repositories");
        assert_eq!(SettingsPane::Branches.title(), "Filters");
        assert_eq!(SettingsPane::General.title(), "General");
        assert_eq!(SettingsPane::Accounts.symbol(), "person.crop.circle");
        assert_eq!(SettingsPane::Repos.symbol(), "folder");
        assert_eq!(
            SettingsPane::Branches.symbol(),
            "line.3.horizontal.decrease.circle"
        );
        assert_eq!(SettingsPane::General.symbol(), "gearshape");
    }

    #[test]
    fn known_panes_and_sheets_are_kept() {
        for pane in SettingsPane::ALL {
            assert_eq!(target(Some(pane.as_str()), None).pane, Some(pane));
        }
        assert_eq!(target(None, Some("add")).sheet, Some(SettingsSheet::Add));
    }

    #[test]
    fn unknown_values_are_dropped() {
        assert_eq!(
            target(Some("../secrets"), Some("delete")),
            SettingsTarget::default()
        );
        assert_eq!(
            target(Some("Repos"), Some("add&view=popup")),
            SettingsTarget::default()
        );
    }

    #[test]
    fn replace_requires_a_known_account() {
        let known = |id: &str| id == "acct-1";
        let replace = SettingsTarget::new(None, Some("replace"), Some("acct-1"), known);
        assert_eq!(replace.sheet, Some(SettingsSheet::Replace));
        assert_eq!(replace.account_id.as_deref(), Some("acct-1"));

        let unknown = SettingsTarget::new(Some("accounts"), Some("replace"), Some("acct-2"), known);
        assert_eq!(unknown.pane, Some(SettingsPane::Accounts));
        assert_eq!(unknown.sheet, None);
        assert_eq!(unknown.account_id, None);

        let missing = SettingsTarget::new(None, Some("replace"), None, known);
        assert_eq!(missing, SettingsTarget::default());
    }

    #[test]
    fn account_is_kept_only_for_a_replace_sheet() {
        let known = |_: &str| true;
        let add = SettingsTarget::new(None, Some("add"), Some("acct-1"), known);
        assert_eq!(add.sheet, Some(SettingsSheet::Add));
        assert_eq!(add.account_id, None);
        let bare = SettingsTarget::new(Some("repos"), None, Some("acct-1"), known);
        assert_eq!(bare.account_id, None);
    }

    #[test]
    fn url_carries_only_valid_params() {
        assert_eq!(target(None, None).url(), "index.html?view=settings");
        assert_eq!(
            target(Some("repos"), None).url(),
            "index.html?view=settings&pane=repos"
        );
        assert_eq!(
            target(Some("accounts"), Some("add")).url(),
            "index.html?view=settings&pane=accounts&sheet=add"
        );
        assert_eq!(
            target(Some("nope"), Some("add")).url(),
            "index.html?view=settings&sheet=add"
        );
    }

    #[test]
    fn url_encodes_the_account() {
        let replace = SettingsTarget::new(
            Some("accounts"),
            Some("replace"),
            Some("a b&view=popup"),
            |_| true,
        );
        assert_eq!(
            replace.url(),
            "index.html?view=settings&pane=accounts&sheet=replace&account=a+b%26view%3Dpopup"
        );
    }

    #[test]
    fn event_payload_uses_null_for_missing_values() {
        let json = serde_json::to_value(target(Some("general"), None)).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "pane": "general", "sheet": null, "account_id": null })
        );
        let replace = SettingsTarget::new(None, Some("replace"), Some("acct-1"), |_| true);
        assert_eq!(
            serde_json::to_value(replace).unwrap(),
            serde_json::json!({ "pane": null, "sheet": "replace", "account_id": "acct-1" })
        );
    }

    #[test]
    fn initial_pane_defaults_to_accounts() {
        assert_eq!(
            SettingsTarget::default().initial_pane(),
            SettingsPane::Accounts
        );
        let repos = target(Some("repos"), Some("add"));
        assert_eq!(repos.initial_pane(), SettingsPane::Repos);
    }

    #[test]
    fn autostart_changes_only_when_it_differs_from_the_setting() {
        assert_eq!(autostart_change(false, false), AutostartChange::Keep);
        assert_eq!(autostart_change(true, true), AutostartChange::Keep);
        assert_eq!(autostart_change(false, true), AutostartChange::Enable);
        assert_eq!(autostart_change(true, false), AutostartChange::Disable);
    }
}
