//! Tauri commands used by the popup and settings windows.

use std::cmp::Ordering;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

use crate::app::Runtime;
use crate::config::{Account, Config, ConfigError, FilterSet, OrgKey, Settings, WatchedRepo};
use crate::links::{check_url, LinkError};
use crate::poller::Snapshot;
use crate::providers::github::InvalidOwner;
use crate::providers::{AccountIdentity, ProviderError};
use crate::secrets::SecretError;
use crate::settings::{
    self, NewAccount, PickerRepo, RepoListCache, RepoListProgress, RestoreToken, SettingsError,
    UnwatchedRepos, REPO_LIST_PROGRESS_EVENT,
};
use crate::updates::{UpdateError, UpdateInfo, UpdateManager, UPDATE_PROGRESS_EVENT};
use crate::windows::{SettingsPane, SettingsTarget};

type CommandResult<T> = Result<T, CommandError>;

/// What kind of failure a command hit, for the frontend to choose its copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Unauthorized,
    Forbidden,
    NotFound,
    RateLimited,
    Network,
    Server,
    Redirected,
    Decode,
    Keychain,
    ReadOnly,
    InvalidInput,
    UnknownAccount,
    Internal,
}

impl ErrorKind {
    /// Whether the failure comes from the user's input, account or network, as opposed to a
    /// fault in the app or its storage.
    fn is_expected(self) -> bool {
        !matches!(
            self,
            ErrorKind::Decode | ErrorKind::Keychain | ErrorKind::Internal
        )
    }
}

/// A failed command as the frontend receives it: `{ "kind": ..., "message": ... }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    pub kind: ErrorKind,
    pub message: String,
}

impl CommandError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> CommandError {
        CommandError {
            kind,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> CommandError {
        CommandError::new(ErrorKind::Internal, error.to_string())
    }
}

impl From<ProviderError> for CommandError {
    fn from(error: ProviderError) -> CommandError {
        let kind = match &error {
            ProviderError::Unauthorized => ErrorKind::Unauthorized,
            ProviderError::RateLimited { .. } => ErrorKind::RateLimited,
            ProviderError::NotFound => ErrorKind::NotFound,
            ProviderError::Forbidden => ErrorKind::Forbidden,
            ProviderError::Redirected { .. } => ErrorKind::Redirected,
            ProviderError::Server { .. } => ErrorKind::Server,
            ProviderError::Network(_) => ErrorKind::Network,
            ProviderError::Decode(_) => ErrorKind::Decode,
        };
        CommandError::new(kind, error.to_string())
    }
}

impl From<ConfigError> for CommandError {
    fn from(error: ConfigError) -> CommandError {
        let kind = match &error {
            ConfigError::ReadOnly => ErrorKind::ReadOnly,
            ConfigError::Io(_) | ConfigError::Parse(_) => ErrorKind::Internal,
        };
        CommandError::new(kind, error.to_string())
    }
}

impl From<SecretError> for CommandError {
    fn from(error: SecretError) -> CommandError {
        CommandError::new(ErrorKind::Keychain, error.to_string())
    }
}

impl From<LinkError> for CommandError {
    fn from(error: LinkError) -> CommandError {
        CommandError::new(ErrorKind::InvalidInput, error.to_string())
    }
}

impl From<InvalidOwner> for CommandError {
    fn from(error: InvalidOwner) -> CommandError {
        CommandError::new(ErrorKind::InvalidInput, error.to_string())
    }
}

impl From<SettingsError> for CommandError {
    fn from(error: SettingsError) -> CommandError {
        match error {
            SettingsError::Provider(e) => e.into(),
            SettingsError::Config(e) => e.into(),
            SettingsError::Secrets(e) => e.into(),
            SettingsError::UnknownAccount => {
                CommandError::new(ErrorKind::UnknownAccount, error.to_string())
            }
            SettingsError::MissingBaseUrl
            | SettingsError::BaseUrl(_)
            | SettingsError::AlreadyWatched(_)
            | SettingsError::DifferentInstance
            | SettingsError::TokenType
            | SettingsError::InvalidInput(_) => {
                CommandError::new(ErrorKind::InvalidInput, error.to_string())
            }
        }
    }
}

impl From<UpdateError> for CommandError {
    fn from(error: UpdateError) -> CommandError {
        let kind = match &error {
            UpdateError::Network(_) => ErrorKind::Network,
            UpdateError::Internal(_) => ErrorKind::Internal,
            UpdateError::NothingPending | UpdateError::AlreadyInstalling => ErrorKind::InvalidInput,
        };
        CommandError::new(kind, error.to_string())
    }
}

/// Converts an error for the frontend and logs it: expected failures at debug, faults at warn.
/// Only the display text is logged, which never includes a token.
fn fail(error: impl Into<CommandError>) -> CommandError {
    let error = error.into();
    if error.kind.is_expected() {
        log::debug!("command failed ({:?}): {}", error.kind, error.message);
    } else {
        log::warn!("command failed ({:?}): {}", error.kind, error.message);
    }
    error
}

#[tauri::command]
pub fn get_snapshot(runtime: State<'_, Arc<Runtime>>) -> Snapshot {
    runtime.snapshot()
}

#[tauri::command]
pub fn refresh_now(runtime: State<'_, Arc<Runtime>>) {
    runtime.refresh_now();
}

#[tauri::command]
pub fn set_paused(runtime: State<'_, Arc<Runtime>>, paused: bool) {
    runtime.set_paused(paused);
}

/// Opens a run or repo URL in the default browser after checking it belongs to the account.
#[tauri::command]
pub fn open_url(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    account_id: String,
    url: String,
) -> CommandResult<()> {
    let checked = checked_account_url(&runtime, &account_id, &url)?;
    app.opener()
        .open_url(checked.as_str(), None::<&str>)
        .map_err(|e| fail(CommandError::internal(e)))
}

#[tauri::command]
pub fn hide_popup(app: AppHandle) {
    crate::tray::hide_popup(&app);
}

/// Opens GitHub's fine-grained token form, prefilled with the grants Vigia needs.
#[tauri::command]
pub fn open_github_token_page(app: AppHandle, owner: Option<String>) -> CommandResult<()> {
    let url = crate::providers::github::token_creation_url(owner.as_deref()).map_err(fail)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| fail(CommandError::internal(e)))
}

/// Highlights the Settings pane the page is showing in the native toolbar. Unknown panes are
/// ignored, and so is every pane while the toolbar is disabled.
#[tauri::command]
pub fn select_settings_pane(app: AppHandle, pane: String) {
    let Ok(pane) = pane.parse::<SettingsPane>() else {
        return;
    };
    #[cfg(target_os = "macos")]
    crate::settings_toolbar::select_on_main(&app, pane);
    #[cfg(not(target_os = "macos"))]
    let _ = (app, pane);
}

/// Enables or disables every item of the Settings window's toolbar, for example while a sheet
/// is open. Does nothing when the window is closed.
#[tauri::command]
pub fn set_settings_toolbar_enabled(app: AppHandle, enabled: bool) {
    #[cfg(target_os = "macos")]
    crate::settings_toolbar::set_enabled_on_main(&app, enabled);
    #[cfg(not(target_os = "macos"))]
    let _ = (app, enabled);
}

/// Opens Settings, optionally at a pane (`accounts`, `repos`, `branches` (titled Filters),
/// `general`) with a sheet over it: `add`, or `replace` for the account `account_id`, which must
/// exist. Unknown values are ignored.
#[tauri::command]
pub fn open_settings(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    pane: Option<String>,
    sheet: Option<String>,
    account_id: Option<String>,
) {
    let target = runtime.with_config(|config| {
        let accounts = &config.config().accounts;
        SettingsTarget::new(
            pane.as_deref(),
            sheet.as_deref(),
            account_id.as_deref(),
            |id| accounts.iter().any(|account| account.id == id),
        )
    });
    crate::windows::open_settings(&app, target);
}

/// Everything the Settings window shows, minus tokens.
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub accounts: Vec<Account>,
    pub repos: Vec<RepoView>,
    /// Every organization with at least one watched repo, sorted by host, then owner.
    pub organizations: Vec<OrgView>,
    pub settings: Settings,
    pub read_only: bool,
    pub secrets_blocked: bool,
}

/// A watched repo with the key of the organization it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoView {
    #[serde(flatten)]
    pub watched: WatchedRepo,
    pub host: String,
    pub owner: String,
}

/// An organization as the Settings window reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrgView {
    pub host: String,
    pub owner: String,
    pub filters: FilterSetView,
    pub repo_count: usize,
}

/// Filter overrides with every field serialized; `null` inherits the global setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FilterSetView {
    pub branch_patterns: Option<Vec<String>>,
    pub ignored_workflows: Option<Vec<String>>,
    pub include_tags: Option<bool>,
}

impl From<&FilterSet> for FilterSetView {
    fn from(filters: &FilterSet) -> FilterSetView {
        FilterSetView {
            branch_patterns: filters.branch_patterns.clone(),
            ignored_workflows: filters.ignored_workflows.clone(),
            include_tags: filters.include_tags,
        }
    }
}

#[tauri::command]
pub fn get_settings(runtime: State<'_, Arc<Runtime>>) -> SettingsView {
    settings_view(&runtime)
}

/// Builds the Settings window's view of the config.
pub fn settings_view(runtime: &Runtime) -> SettingsView {
    runtime.with_config(|store| {
        let config = store.config();
        let repos: Vec<RepoView> = config
            .repos
            .iter()
            .map(|watched| {
                let key = config.org_key(watched).unwrap_or_default();
                RepoView {
                    watched: watched.clone(),
                    host: key.host,
                    owner: key.owner,
                }
            })
            .collect();
        SettingsView {
            accounts: config.accounts.clone(),
            organizations: org_views(config, &repos),
            repos,
            settings: config.settings.clone(),
            read_only: store.read_only(),
            secrets_blocked: runtime.secrets.is_blocked(),
        }
    })
}

/// One view per organization of `repos`, counting its repos and carrying its saved overrides.
/// Repos without a known host belong to no organization.
fn org_views(config: &Config, repos: &[RepoView]) -> Vec<OrgView> {
    let mut organizations: Vec<OrgView> = Vec::new();
    for repo in repos.iter().filter(|r| !r.host.is_empty()) {
        let key = OrgKey {
            host: repo.host.clone(),
            owner: repo.owner.clone(),
        };
        let existing = organizations
            .iter_mut()
            .find(|o| key.matches(&o.host, &o.owner));
        match existing {
            Some(org) => org.repo_count += 1,
            None => organizations.push(OrgView {
                filters: FilterSetView::from(key.filters_in(&config.organizations)),
                host: key.host,
                owner: key.owner,
                repo_count: 1,
            }),
        }
    }
    organizations.sort_by(|a, b| {
        natural_cmp(&a.host, &b.host).then_with(|| natural_cmp(&a.owner, &b.owner))
    });
    organizations
}

/// Compares case-insensitively, with runs of digits compared by value, so `repo-2` sorts
/// before `repo-10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(l), Some(r)) if l.is_ascii_digit() && r.is_ascii_digit() => {
                let l_digits = take_digits(&mut left);
                let r_digits = take_digits(&mut right);
                let l_trimmed = l_digits.trim_start_matches('0');
                let r_trimmed = r_digits.trim_start_matches('0');
                let order = l_trimmed
                    .len()
                    .cmp(&r_trimmed.len())
                    .then_with(|| l_trimmed.cmp(r_trimmed));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(l), Some(r)) => {
                let order = l.to_lowercase().cmp(r.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                left.next();
                right.next();
            }
        }
    }
}

fn take_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(c) = chars.next_if(char::is_ascii_digit) {
        digits.push(c);
    }
    digits
}

#[tauri::command]
pub async fn test_connection(input: NewAccount) -> CommandResult<AccountIdentity> {
    settings::test_connection(&input).await.map_err(fail)
}

#[tauri::command]
pub async fn add_account(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    input: NewAccount,
) -> CommandResult<Account> {
    let account = settings::add_account(&runtime, input).await.map_err(fail)?;
    crate::notifications::ensure_permission(&app);
    Ok(account)
}

#[tauri::command]
pub fn rename_account(
    runtime: State<'_, Arc<Runtime>>,
    account_id: String,
    label: String,
) -> CommandResult<()> {
    settings::rename_account(&runtime, &account_id, &label).map_err(fail)
}

#[tauri::command]
pub async fn replace_token(
    runtime: State<'_, Arc<Runtime>>,
    account_id: String,
    token: String,
) -> CommandResult<()> {
    settings::replace_token(&runtime, &account_id, &token)
        .await
        .map_err(fail)
}

#[tauri::command(async)]
pub fn delete_account(
    runtime: State<'_, Arc<Runtime>>,
    cache: State<'_, RepoListCache>,
    account_id: String,
) -> CommandResult<()> {
    settings::delete_account(&runtime, &cache, &account_id).map_err(fail)
}

#[tauri::command(async)]
pub fn delete_all_accounts(
    runtime: State<'_, Arc<Runtime>>,
    cache: State<'_, RepoListCache>,
) -> CommandResult<usize> {
    settings::delete_all_accounts(&runtime, &cache).map_err(fail)
}

/// Lists an account's repos for the picker and reports progress per page through
/// `REPO_LIST_PROGRESS_EVENT`.
#[tauri::command]
pub async fn list_picker_repos(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    cache: State<'_, RepoListCache>,
    account_id: String,
    refresh: bool,
) -> CommandResult<Vec<PickerRepo>> {
    let progress_account = account_id.clone();
    let on_page = move |loaded: usize| {
        let progress = RepoListProgress {
            account_id: progress_account.clone(),
            loaded,
        };
        if let Err(e) = app.emit(REPO_LIST_PROGRESS_EVENT, progress) {
            log::debug!("repo list progress not sent: {e}");
        }
    };
    settings::picker_repos(&runtime, &cache, &account_id, refresh, &on_page)
        .await
        .map_err(fail)
}

#[tauri::command]
pub fn set_watched(
    runtime: State<'_, Arc<Runtime>>,
    cache: State<'_, RepoListCache>,
    account_id: String,
    repo_ids: Vec<u64>,
) -> CommandResult<usize> {
    settings::set_watched(&runtime, &cache, &account_id, &repo_ids).map_err(fail)
}

/// Stops watching a repo. Returns the token `restore_watched_repo` takes to undo it.
#[tauri::command]
pub fn unwatch_repo(
    runtime: State<'_, Arc<Runtime>>,
    unwatched: State<'_, UnwatchedRepos>,
    account_id: String,
    repo_id: u64,
) -> CommandResult<Option<RestoreToken>> {
    settings::unwatch_repo(&runtime, &unwatched, &account_id, repo_id).map_err(fail)
}

/// Watches a repo again from the entry kept for `token`. Returns whether it was added.
#[tauri::command]
pub fn restore_watched_repo(
    runtime: State<'_, Arc<Runtime>>,
    unwatched: State<'_, UnwatchedRepos>,
    token: RestoreToken,
) -> CommandResult<bool> {
    settings::restore_watched_repo(&runtime, &unwatched, &token).map_err(fail)
}

/// Clears a repo's branch, workflow and tag overrides together.
#[tauri::command]
pub fn clear_repo_overrides(
    runtime: State<'_, Arc<Runtime>>,
    account_id: String,
    repo_id: u64,
) -> CommandResult<()> {
    settings::clear_repo_overrides(&runtime, &account_id, repo_id).map_err(fail)
}

#[tauri::command]
pub fn set_repo_branches(
    runtime: State<'_, Arc<Runtime>>,
    account_id: String,
    repo_id: u64,
    patterns: Option<Vec<String>>,
) -> CommandResult<()> {
    settings::set_repo_branches(&runtime, &account_id, repo_id, patterns).map_err(fail)
}

#[tauri::command]
pub fn set_repo_ignored_workflows(
    runtime: State<'_, Arc<Runtime>>,
    account_id: String,
    repo_id: u64,
    patterns: Option<Vec<String>>,
) -> CommandResult<()> {
    settings::set_repo_ignored_workflows(&runtime, &account_id, repo_id, patterns).map_err(fail)
}

/// Saves an organization's filter overrides; a `null` field inherits the global setting.
#[tauri::command]
pub fn set_org_filters(
    runtime: State<'_, Arc<Runtime>>,
    host: String,
    owner: String,
    filters: FilterSet,
) -> CommandResult<()> {
    settings::set_org_filters(&runtime, &host, &owner, filters).map_err(fail)
}

#[tauri::command]
pub fn set_repo_include_tags(
    runtime: State<'_, Arc<Runtime>>,
    account_id: String,
    repo_id: u64,
    include: Option<bool>,
) -> CommandResult<()> {
    settings::set_repo_include_tags(&runtime, &account_id, repo_id, include).map_err(fail)
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    runtime: State<'_, Arc<Runtime>>,
    settings: Settings,
) -> CommandResult<()> {
    let launch_at_login = settings.launch_at_login;
    let before = runtime.with_config(|c| c.config().settings.clone());
    let enables_notifications = crate::notifications::enables_notifications(&before, &settings);
    settings::update_settings(&runtime, settings).map_err(fail)?;
    if enables_notifications {
        crate::notifications::ensure_permission(&app);
    }
    crate::windows::sync_autostart(&app, launch_at_login);
    Ok(())
}

/// Checks for an update now. Returns the update found, or `null` when the app is current.
#[tauri::command]
pub async fn check_for_updates_now(
    updates: State<'_, Arc<UpdateManager>>,
) -> CommandResult<Option<UpdateInfo>> {
    updates.check_now().await.map_err(fail)
}

/// Downloads and installs the pending update, emitting `update-progress` along the way, then
/// relaunches the app.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    updates: State<'_, Arc<UpdateManager>>,
) -> CommandResult<()> {
    let emitter = app.clone();
    let progress = move |progress| {
        let _ = emitter.emit(UPDATE_PROGRESS_EVENT, progress);
    };
    updates.install(&progress).await.map_err(fail)?;
    log::info!("update installed, relaunching");
    app.request_restart();
    Ok(())
}

#[tauri::command(async)]
pub fn retry_secrets(runtime: State<'_, Arc<Runtime>>) -> bool {
    retry_secrets_and_restart(&runtime)
}

/// Overwrites the Keychain item with an empty token map after the user confirmed discarding
/// the unreadable one. Returns whether the item was reset.
#[tauri::command(async)]
pub fn reset_secrets(runtime: State<'_, Arc<Runtime>>) -> bool {
    reset_secrets_and_restart(&runtime)
}

/// Checks `url` against the account's host. Errors name an unknown account or a foreign URL.
pub fn checked_account_url(
    runtime: &Runtime,
    account_id: &str,
    url: &str,
) -> CommandResult<url::Url> {
    let account = runtime
        .with_config(|c| c.config().account(account_id).cloned())
        .ok_or_else(|| fail(SettingsError::UnknownAccount))?;
    check_url(&account, url).map_err(fail)
}

/// Reads the token store again and restarts the workers when it is readable. Returns whether
/// it was.
pub fn retry_secrets_and_restart(runtime: &Arc<Runtime>) -> bool {
    let state = runtime.secrets.load();
    let ok = !matches!(state, crate::secrets::LoadState::Failed(_));
    if ok {
        runtime.restart();
    }
    ok
}

/// Empties the token store and restarts the workers. Returns whether the store was written.
pub fn reset_secrets_and_restart(runtime: &Arc<Runtime>) -> bool {
    match runtime.secrets.reset() {
        Ok(()) => {
            runtime.restart();
            true
        }
        Err(e) => {
            log::warn!("could not reset the stored tokens: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(error: impl Into<CommandError>) -> ErrorKind {
        error.into().kind
    }

    #[test]
    fn natural_order_ignores_case_and_compares_numbers_by_value() {
        assert_eq!(natural_cmp("repo-2", "repo-10"), Ordering::Less);
        assert_eq!(natural_cmp("Repo-10", "repo-9"), Ordering::Greater);
        assert_eq!(natural_cmp("Acme", "example"), Ordering::Less);
        assert_eq!(natural_cmp("acme", "acme-2"), Ordering::Less);
    }

    #[test]
    fn provider_errors_keep_their_kind() {
        let cases = [
            (ProviderError::Unauthorized, ErrorKind::Unauthorized),
            (ProviderError::Forbidden, ErrorKind::Forbidden),
            (ProviderError::NotFound, ErrorKind::NotFound),
            (
                ProviderError::RateLimited {
                    retry_after: None,
                    reset_at: None,
                    remaining: Some(0),
                },
                ErrorKind::RateLimited,
            ),
            (ProviderError::Network("down".into()), ErrorKind::Network),
            (ProviderError::Server { status: 502 }, ErrorKind::Server),
            (
                ProviderError::Redirected {
                    location: "https://elsewhere.example.test/".into(),
                },
                ErrorKind::Redirected,
            ),
            (ProviderError::Decode("bad".into()), ErrorKind::Decode),
        ];
        for (error, expected) in cases {
            assert_eq!(kind(error.clone()), expected, "{error:?}");
            assert_eq!(kind(SettingsError::Provider(error)), expected);
        }
    }

    #[test]
    fn settings_errors_map_to_kinds() {
        let cases = [
            (SettingsError::MissingBaseUrl, ErrorKind::InvalidInput),
            (
                SettingsError::BaseUrl("bad".into()),
                ErrorKind::InvalidInput,
            ),
            (
                SettingsError::AlreadyWatched("b".into()),
                ErrorKind::InvalidInput,
            ),
            (SettingsError::DifferentInstance, ErrorKind::InvalidInput),
            (SettingsError::TokenType, ErrorKind::InvalidInput),
            (
                SettingsError::InvalidInput("long".into()),
                ErrorKind::InvalidInput,
            ),
            (SettingsError::UnknownAccount, ErrorKind::UnknownAccount),
            (
                SettingsError::Config(ConfigError::ReadOnly),
                ErrorKind::ReadOnly,
            ),
            (
                SettingsError::Secrets(SecretError::WriteBlocked),
                ErrorKind::Keychain,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(kind(error), expected);
        }
    }

    #[test]
    fn config_secret_and_link_errors_map_to_kinds() {
        let io = std::io::Error::other("disk");
        assert_eq!(kind(ConfigError::Io(io)), ErrorKind::Internal);
        let parse = serde_json::from_str::<u8>("x").unwrap_err();
        assert_eq!(kind(ConfigError::Parse(parse)), ErrorKind::Internal);
        assert_eq!(kind(ConfigError::ReadOnly), ErrorKind::ReadOnly);
        assert_eq!(
            kind(SecretError::AccessDenied("no".into())),
            ErrorKind::Keychain
        );
        assert_eq!(
            kind(SecretError::Corrupt("syntax".into())),
            ErrorKind::Keychain
        );
        assert_eq!(kind(LinkError::WrongHost), ErrorKind::InvalidInput);
        assert_eq!(kind(LinkError::Invalid), ErrorKind::InvalidInput);
        assert_eq!(kind(InvalidOwner), ErrorKind::InvalidInput);
        assert_eq!(
            CommandError::internal("opener failed").kind,
            ErrorKind::Internal
        );
    }

    #[test]
    fn update_errors_map_to_kinds() {
        let cases = [
            (UpdateError::Network("offline".into()), ErrorKind::Network),
            (UpdateError::Internal("bad".into()), ErrorKind::Internal),
            (UpdateError::NothingPending, ErrorKind::InvalidInput),
            (UpdateError::AlreadyInstalling, ErrorKind::InvalidInput),
        ];
        for (error, expected) in cases {
            assert_eq!(kind(error), expected);
        }
    }

    #[test]
    fn command_errors_serialize_as_kind_and_message() {
        let error = fail(SettingsError::UnknownAccount);
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({ "kind": "unknown_account", "message": "unknown account" })
        );
        let error = fail(ProviderError::RateLimited {
            retry_after: Some(5),
            reset_at: None,
            remaining: None,
        });
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            serde_json::json!({ "kind": "rate_limited", "message": "rate limited" })
        );
    }

    #[test]
    fn every_kind_has_its_wire_name() {
        let names = [
            (ErrorKind::Unauthorized, "unauthorized"),
            (ErrorKind::Forbidden, "forbidden"),
            (ErrorKind::NotFound, "not_found"),
            (ErrorKind::RateLimited, "rate_limited"),
            (ErrorKind::Network, "network"),
            (ErrorKind::Server, "server"),
            (ErrorKind::Redirected, "redirected"),
            (ErrorKind::Decode, "decode"),
            (ErrorKind::Keychain, "keychain"),
            (ErrorKind::ReadOnly, "read_only"),
            (ErrorKind::InvalidInput, "invalid_input"),
            (ErrorKind::UnknownAccount, "unknown_account"),
            (ErrorKind::Internal, "internal"),
        ];
        for (kind, name) in names {
            assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(name));
        }
    }
}
