//! Account, repo and settings management behind the Settings window.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app::Runtime;
use crate::config::{
    Account, AccountKind, FilterSet, OrgFilters, Settings, WatchedRepo, MAX_POLL_INTERVAL_SECS,
    MIN_POLL_INTERVAL_SECS,
};
use crate::providers::github::{is_fine_grained_token, GitHubProvider, DEFAULT_BASE_URL};
use crate::providers::gitlab::{normalize_base_url, GitLabProvider};
use crate::providers::{AccountIdentity, Provider, RepoInfo};

/// Most patterns one list may hold.
pub const MAX_PATTERNS: usize = 50;
/// Most characters one pattern may have.
pub const MAX_PATTERN_CHARS: usize = 200;
/// How long an unwatched repo can be restored.
pub const RESTORE_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("base URL is required")]
    MissingBaseUrl,
    #[error("{0}")]
    BaseUrl(String),
    #[error("{0}")]
    Provider(#[from] crate::providers::ProviderError),
    #[error("{0}")]
    Config(#[from] crate::config::ConfigError),
    #[error("{0}")]
    Secrets(#[from] crate::secrets::SecretError),
    #[error("unknown account")]
    UnknownAccount,
    #[error("this repo is already watched through account {0}")]
    AlreadyWatched(String),
    #[error("the new base URL points at a different instance or user")]
    DifferentInstance,
    #[error("Vigia needs a fine-grained GitHub token (it starts with github_pat_). Classic tokens grant write access and are not accepted.")]
    TokenType,
    #[error("{0}")]
    InvalidInput(String),
}

/// Input for adding an account or testing a connection.
#[derive(Clone, Deserialize)]
pub struct NewAccount {
    pub kind: AccountKind,
    pub label: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub token: String,
    /// The user confirmed that an `http://` base URL sends the token unencrypted.
    #[serde(default)]
    pub allow_insecure: bool,
}

impl std::fmt::Debug for NewAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewAccount")
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .field("allow_insecure", &self.allow_insecure)
            .finish()
    }
}

/// A repo as shown in the picker, with the account that already watches it, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PickerRepo {
    pub repo: RepoInfo,
    pub watched: bool,
    pub watched_via: Option<String>,
}

/// Event emitted while the picker's repo list loads, once per fetched page.
pub const REPO_LIST_PROGRESS_EVENT: &str = "repo-list-progress";

/// Payload of `REPO_LIST_PROGRESS_EVENT`: how many repos of the account have loaded so far.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoListProgress {
    pub account_id: String,
    pub loaded: usize,
}

/// Cache of listed repos per account, so the picker does not hit the API on every open.
/// Clones share the same lists.
#[derive(Clone, Default)]
pub struct RepoListCache {
    lists: Arc<Mutex<HashMap<String, Vec<RepoInfo>>>>,
}

impl RepoListCache {
    /// Stores a list fetched outside the picker, such as at startup. A list already cached
    /// for the account is kept.
    pub fn seed(&self, account_id: &str, repos: Vec<RepoInfo>) {
        self.lists
            .lock()
            .unwrap()
            .entry(account_id.to_string())
            .or_insert(repos);
    }

    pub fn get(&self, account_id: &str) -> Option<Vec<RepoInfo>> {
        self.lists.lock().unwrap().get(account_id).cloned()
    }

    pub fn set(&self, account_id: &str, repos: Vec<RepoInfo>) {
        self.lists
            .lock()
            .unwrap()
            .insert(account_id.to_string(), repos);
    }

    pub fn remove(&self, account_id: &str) {
        self.lists.lock().unwrap().remove(account_id);
    }
}

/// Names one unwatched repo that `restore_watched_repo` can bring back.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RestoreToken {
    pub account_id: String,
    pub repo_id: u64,
}

/// Entries removed by `unwatch_repo`, kept in memory for `RESTORE_TTL` so the webview restores
/// a repo by naming it and never submits the entry itself.
pub struct UnwatchedRepos {
    ttl: Duration,
    entries: Mutex<HashMap<RestoreToken, (WatchedRepo, Instant)>>,
}

impl Default for UnwatchedRepos {
    fn default() -> UnwatchedRepos {
        UnwatchedRepos::with_ttl(RESTORE_TTL)
    }
}

impl UnwatchedRepos {
    pub fn with_ttl(ttl: Duration) -> UnwatchedRepos {
        UnwatchedRepos {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Keeps `watched` and returns the token that restores it. Expired entries are dropped.
    pub fn keep(&self, watched: WatchedRepo) -> RestoreToken {
        let token = RestoreToken {
            account_id: watched.account_id.clone(),
            repo_id: watched.repo.id,
        };
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, (_, kept_at)| kept_at.elapsed() < self.ttl);
        entries.insert(token.clone(), (watched, Instant::now()));
        token
    }

    /// Removes and returns the entry for `token`, unless it is unknown or expired.
    pub fn take(&self, token: &RestoreToken) -> Option<WatchedRepo> {
        let (watched, kept_at) = self.entries.lock().unwrap().remove(token)?;
        let age = kept_at.elapsed();
        (age < self.ttl).then_some(watched)
    }

    /// Puts an entry back with a fresh expiry, after a restore could not be saved.
    fn put_back(&self, watched: WatchedRepo) {
        self.keep(watched);
    }
}

/// Builds the provider for a token. `github_api` is the GitHub API root.
fn provider_for(
    kind: AccountKind,
    base_url: Option<&str>,
    token: &str,
    github_api: &str,
) -> Result<Box<dyn Provider>, SettingsError> {
    Ok(match kind {
        AccountKind::GitHub => {
            if !is_fine_grained_token(token) {
                return Err(SettingsError::TokenType);
            }
            Box::new(GitHubProvider::with_base_url(token.trim(), github_api)?)
        }
        AccountKind::GitLab => {
            let base = base_url.ok_or(SettingsError::MissingBaseUrl)?;
            Box::new(GitLabProvider::new(base, token)?)
        }
    })
}

/// Validates the token and returns the identity. Nothing is stored.
pub async fn test_connection(input: &NewAccount) -> Result<AccountIdentity, SettingsError> {
    test_connection_against(input, DEFAULT_BASE_URL).await
}

/// `test_connection` with GitHub reached at `github_api`, so tests can use a local server.
pub async fn test_connection_against(
    input: &NewAccount,
    github_api: &str,
) -> Result<AccountIdentity, SettingsError> {
    let base = normalized_base(input)?;
    let provider = provider_for(input.kind, base.as_deref(), &input.token, github_api)?;
    Ok(provider.validate().await?)
}

fn normalized_base(input: &NewAccount) -> Result<Option<String>, SettingsError> {
    match input.kind {
        AccountKind::GitHub => Ok(None),
        AccountKind::GitLab => {
            let raw = input
                .base_url
                .as_deref()
                .ok_or(SettingsError::MissingBaseUrl)?;
            let url = normalize_base_url(raw).map_err(|e| SettingsError::BaseUrl(e.to_string()))?;
            if url.scheme() == "http" && !input.allow_insecure {
                return Err(SettingsError::BaseUrl(
                    "an http:// base URL sends the token unencrypted; confirm to continue".into(),
                ));
            }
            Ok(Some(url.to_string()))
        }
    }
}

/// Validates the token, stores it, saves the account, and restarts polling.
pub async fn add_account(
    runtime: &Arc<Runtime>,
    input: NewAccount,
) -> Result<Account, SettingsError> {
    let base = normalized_base(&input)?;
    let provider = provider_for(input.kind, base.as_deref(), &input.token, DEFAULT_BASE_URL)?;
    let identity = provider.validate().await?;

    let mut account = Account::new(input.kind, input.label.trim(), base);
    account.user_id = Some(identity.user_id);
    account.login = Some(identity.login);

    runtime.secrets.set_token(&account.id, &input.token)?;
    let saved = runtime.update_config(|c| {
        c.accounts.push(account.clone());
    });
    if let Err(e) = saved {
        let _ = runtime.secrets.remove_token(&account.id);
        return Err(e.into());
    }
    runtime.restart();
    Ok(account)
}

pub fn rename_account(
    runtime: &Arc<Runtime>,
    account_id: &str,
    label: &str,
) -> Result<(), SettingsError> {
    runtime.with_config_mut(|config| {
        if config.config().account(account_id).is_none() {
            return Err(SettingsError::UnknownAccount);
        }
        config.update(|c| {
            if let Some(a) = c.accounts.iter_mut().find(|a| a.id == account_id) {
                a.label = label.trim().to_string();
            }
        })?;
        Ok(())
    })?;
    runtime.restart();
    Ok(())
}

/// Validates the new token against the same user, stores it, and restarts polling.
pub async fn replace_token(
    runtime: &Arc<Runtime>,
    account_id: &str,
    token: &str,
) -> Result<(), SettingsError> {
    let account = runtime
        .with_config(|c| c.config().account(account_id).cloned())
        .ok_or(SettingsError::UnknownAccount)?;
    let provider = provider_for(
        account.kind,
        account.base_url.as_deref(),
        token,
        DEFAULT_BASE_URL,
    )?;
    let identity = provider.validate().await?;
    if account.user_id.is_some_and(|id| id != identity.user_id) {
        return Err(SettingsError::DifferentInstance);
    }
    runtime.secrets.set_token(account_id, token)?;
    // A new token earns a fresh auth error notification if it is ever rejected.
    runtime.with_notifier(|n| n.forget_auth(account_id));
    runtime.restart();
    Ok(())
}

/// Removes the account, its repos, its token, and its cached list, then restarts polling.
pub fn delete_account(
    runtime: &Arc<Runtime>,
    cache: &RepoListCache,
    account_id: &str,
) -> Result<(), SettingsError> {
    runtime.with_config_mut(|config| {
        if config.config().account(account_id).is_none() {
            return Err(SettingsError::UnknownAccount);
        }
        config.update(|c| c.remove_account(account_id))?;
        Ok(())
    })?;
    // Keychain writes may be blocked; the config is already consistent without the token.
    if let Err(e) = runtime.secrets.remove_token(account_id) {
        log::warn!("token for deleted account {account_id} not removed: {e}");
    }
    cache.remove(account_id);
    runtime.store.remove_account(account_id);
    runtime.restart();
    Ok(())
}

/// Removes every account with its repos, tokens and cached lists. Settings are kept.
/// Returns how many accounts were removed.
pub fn delete_all_accounts(
    runtime: &Arc<Runtime>,
    cache: &RepoListCache,
) -> Result<usize, SettingsError> {
    let ids = runtime.with_config_mut(|config| -> Result<Vec<String>, SettingsError> {
        let ids: Vec<String> = config
            .config()
            .accounts
            .iter()
            .map(|a| a.id.clone())
            .collect();
        config.update(|c| {
            c.accounts.clear();
            c.repos.clear();
        })?;
        Ok(ids)
    })?;
    for id in &ids {
        // Keychain writes may be blocked; the config is already consistent without the tokens.
        if let Err(e) = runtime.secrets.remove_token(id) {
            log::warn!("token for deleted account {id} not removed: {e}");
        }
        cache.remove(id);
        runtime.store.remove_account(id);
    }
    runtime.restart();
    Ok(ids.len())
}

/// Lists an account's repos for the picker, from the cache unless `refresh` is set.
/// `on_page` receives the running count of loaded repos after each page.
pub async fn picker_repos(
    runtime: &Arc<Runtime>,
    cache: &RepoListCache,
    account_id: &str,
    refresh: bool,
    on_page: &(dyn Fn(usize) + Send + Sync),
) -> Result<Vec<PickerRepo>, SettingsError> {
    let config = runtime.with_config(|c| c.config().clone());
    let account = config
        .account(account_id)
        .cloned()
        .ok_or(SettingsError::UnknownAccount)?;
    let listed = match cache.get(account_id).filter(|_| !refresh) {
        Some(list) => list,
        None => {
            let provider = runtime.provider_for(&account)?;
            let list = provider.list_repos(on_page).await?;
            cache.set(account_id, list.clone());
            list
        }
    };
    Ok(mark_watched(
        &config.accounts,
        &config.repos,
        &account,
        listed,
    ))
}

/// Marks each listed repo as watched here, or watched through another account of the same instance.
pub fn mark_watched(
    accounts: &[Account],
    watched: &[WatchedRepo],
    account: &Account,
    listed: Vec<RepoInfo>,
) -> Vec<PickerRepo> {
    listed
        .into_iter()
        .map(|repo| {
            let via = watched
                .iter()
                .filter(|w| w.repo.id == repo.id)
                .find_map(|w| {
                    let other = accounts.iter().find(|a| a.id == w.account_id)?;
                    same_instance(account, other).then(|| other.clone())
                });
            match via {
                Some(other) if other.id == account.id => PickerRepo {
                    repo,
                    watched: true,
                    watched_via: None,
                },
                Some(other) => PickerRepo {
                    repo,
                    watched: false,
                    watched_via: Some(other.label),
                },
                None => PickerRepo {
                    repo,
                    watched: false,
                    watched_via: None,
                },
            }
        })
        .collect()
}

/// Two accounts share an instance when both are GitHub, or both are GitLab with the same base URL.
pub fn same_instance(a: &Account, b: &Account) -> bool {
    match (a.kind, b.kind) {
        (AccountKind::GitHub, AccountKind::GitHub) => true,
        (AccountKind::GitLab, AccountKind::GitLab) => a.base_url == b.base_url,
        _ => false,
    }
}

/// Replaces the watched set of one account with `repo_ids`, taken from the cached list.
pub fn set_watched(
    runtime: &Arc<Runtime>,
    cache: &RepoListCache,
    account_id: &str,
    repo_ids: &[u64],
) -> Result<usize, SettingsError> {
    let listed = cache.get(account_id).unwrap_or_default();
    let count = runtime.with_config_mut(|config| -> Result<usize, SettingsError> {
        let current = config.config().clone();
        let account = current
            .account(account_id)
            .cloned()
            .ok_or(SettingsError::UnknownAccount)?;

        for id in repo_ids {
            let clash = current.repos.iter().find(|w| {
                w.repo.id == *id
                    && w.account_id != account_id
                    && current
                        .account(&w.account_id)
                        .is_some_and(|other| same_instance(&account, other))
            });
            if let Some(w) = clash {
                let label = current
                    .account(&w.account_id)
                    .map(|a| a.label.clone())
                    .unwrap_or_default();
                return Err(SettingsError::AlreadyWatched(label));
            }
        }

        let mut next: Vec<WatchedRepo> = Vec::new();
        for id in repo_ids {
            let existing = current
                .repos
                .iter()
                .find(|w| w.account_id == account_id && w.repo.id == *id);
            let info = listed
                .iter()
                .find(|r| r.id == *id)
                .cloned()
                .or_else(|| existing.map(|w| w.repo.clone()));
            let Some(info) = info else {
                continue;
            };
            next.push(WatchedRepo {
                account_id: account_id.to_string(),
                repo: info,
                branch_patterns: existing.and_then(|w| w.branch_patterns.clone()),
                ignored_workflows: existing.and_then(|w| w.ignored_workflows.clone()),
                include_tags: existing.and_then(|w| w.include_tags),
            });
        }
        let count = next.len();
        config.update(|c| {
            c.repos.retain(|w| w.account_id != account_id);
            c.repos.extend(next.clone());
        })?;
        Ok(count)
    })?;
    runtime.restart();
    Ok(count)
}

/// Stops watching one repo of an account and keeps its entry in `unwatched`. Returns the
/// token that restores it, or `None` when the repo was not watched.
pub fn unwatch_repo(
    runtime: &Arc<Runtime>,
    unwatched: &UnwatchedRepos,
    account_id: &str,
    repo_id: u64,
) -> Result<Option<RestoreToken>, SettingsError> {
    let removed = runtime.with_config_mut(|config| {
        let removed = config
            .config()
            .repos
            .iter()
            .find(|w| w.account_id == account_id && w.repo.id == repo_id)
            .cloned();
        config.update(|c| {
            c.repos
                .retain(|w| !(w.account_id == account_id && w.repo.id == repo_id));
        })?;
        Ok::<_, SettingsError>(removed)
    })?;
    runtime.restart();
    Ok(removed.map(|watched| unwatched.keep(watched)))
}

/// Watches a repo again with the entry `unwatch_repo` kept for `token`, overrides included.
/// Returns whether it was added: nothing changes when the entry expired or is unknown, its
/// account is gone, or the repo is already watched, directly or through another account of
/// the same instance.
pub fn restore_watched_repo(
    runtime: &Arc<Runtime>,
    unwatched: &UnwatchedRepos,
    token: &RestoreToken,
) -> Result<bool, SettingsError> {
    let added = runtime.with_config_mut(|config| -> Result<bool, SettingsError> {
        if config.read_only() {
            return Err(crate::config::ConfigError::ReadOnly.into());
        }
        let Some(watched) = unwatched.take(token) else {
            return Ok(false);
        };
        let current = config.config();
        let Some(account) = current.account(&watched.account_id) else {
            return Ok(false);
        };
        let already = current.repos.iter().any(|w| {
            w.repo.id == watched.repo.id
                && current
                    .account(&w.account_id)
                    .is_some_and(|other| same_instance(account, other))
        });
        if already {
            return Ok(false);
        }
        if let Err(e) = config.update(|c| c.repos.push(watched.clone())) {
            unwatched.put_back(watched);
            return Err(e.into());
        }
        Ok(true)
    })?;
    if added {
        runtime.restart();
    }
    Ok(added)
}

/// Per-repo branch override. `None` clears it; an empty list means the default branch only.
pub fn set_repo_branches(
    runtime: &Arc<Runtime>,
    account_id: &str,
    repo_id: u64,
    patterns: Option<Vec<String>>,
) -> Result<(), SettingsError> {
    let patterns = patterns.map(clean_patterns);
    if let Some(patterns) = &patterns {
        check_patterns(patterns)?;
    }
    runtime.update_config(|c| {
        if let Some(w) = c
            .repos
            .iter_mut()
            .find(|w| w.account_id == account_id && w.repo.id == repo_id)
        {
            w.branch_patterns = patterns;
        }
    })?;
    runtime.restart();
    Ok(())
}

/// Per-repo workflow ignore list. `None` clears it; an empty list ignores nothing for the repo.
pub fn set_repo_ignored_workflows(
    runtime: &Arc<Runtime>,
    account_id: &str,
    repo_id: u64,
    patterns: Option<Vec<String>>,
) -> Result<(), SettingsError> {
    let patterns = patterns.map(clean_patterns);
    if let Some(patterns) = &patterns {
        check_patterns(patterns)?;
    }
    runtime.update_config(|c| {
        if let Some(w) = c
            .repos
            .iter_mut()
            .find(|w| w.account_id == account_id && w.repo.id == repo_id)
        {
            w.ignored_workflows = patterns;
        }
    })?;
    runtime.restart();
    Ok(())
}

/// Per-repo tag choice. `None` uses the global setting.
pub fn set_repo_include_tags(
    runtime: &Arc<Runtime>,
    account_id: &str,
    repo_id: u64,
    include: Option<bool>,
) -> Result<(), SettingsError> {
    runtime.update_config(|c| {
        if let Some(w) = c
            .repos
            .iter_mut()
            .find(|w| w.account_id == account_id && w.repo.id == repo_id)
        {
            w.include_tags = include;
        }
    })?;
    runtime.restart();
    Ok(())
}

/// Shown when `set_org_filters` names an organization with no watched repo and no saved entry.
pub const UNKNOWN_ORG: &str = "No watched repositories in that organization.";

/// Saves the filter overrides of one organization in a single save. A `None` field inherits the
/// global setting; an empty list is kept as an override. An entry left with no override is
/// dropped. The owner is stored as the watched repos spell it.
pub fn set_org_filters(
    runtime: &Arc<Runtime>,
    host: &str,
    owner: &str,
    filters: FilterSet,
) -> Result<(), SettingsError> {
    let filters = FilterSet {
        branch_patterns: filters.branch_patterns.map(clean_patterns),
        ignored_workflows: filters.ignored_workflows.map(clean_patterns),
        include_tags: filters.include_tags,
    };
    let lists = [&filters.branch_patterns, &filters.ignored_workflows];
    for patterns in lists.into_iter().flatten() {
        check_patterns(patterns)?;
    }
    let host = host.trim().to_ascii_lowercase();
    let owner = owner.trim();
    runtime.with_config_mut(|config| {
        if config.read_only() {
            return Err(crate::config::ConfigError::ReadOnly.into());
        }
        let current = config.config();
        let watched_owner = current
            .repos
            .iter()
            .filter_map(|r| current.org_key(r))
            .find(|key| key.matches(&host, owner))
            .map(|key| key.owner);
        let saved = current
            .organizations
            .iter()
            .position(|o| o.matches(&host, owner));
        let display_owner = match (watched_owner, saved) {
            (Some(display), _) => display,
            (None, Some(index)) => current.organizations[index].owner.clone(),
            (None, None) => return Err(SettingsError::InvalidInput(UNKNOWN_ORG.into())),
        };
        config.update(|c| {
            let entry = OrgFilters {
                host,
                owner: display_owner,
                filters,
            };
            match saved {
                Some(index) => c.organizations[index] = entry,
                None => c.organizations.push(entry),
            }
        })?;
        Ok(())
    })?;
    runtime.restart();
    Ok(())
}

/// Clears the branch, workflow and tag overrides of one repo in a single save.
pub fn clear_repo_overrides(
    runtime: &Arc<Runtime>,
    account_id: &str,
    repo_id: u64,
) -> Result<(), SettingsError> {
    runtime.with_config_mut(|config| {
        if config.read_only() {
            return Err(crate::config::ConfigError::ReadOnly.into());
        }
        if config.config().account(account_id).is_none() {
            return Err(SettingsError::UnknownAccount);
        }
        config.update(|c| {
            if let Some(w) = c
                .repos
                .iter_mut()
                .find(|w| w.account_id == account_id && w.repo.id == repo_id)
            {
                w.branch_patterns = None;
                w.ignored_workflows = None;
                w.include_tags = None;
            }
        })?;
        Ok(())
    })?;
    runtime.restart();
    Ok(())
}

/// Saves the settings with the interval clamped to its range and pattern lists checked.
pub fn update_settings(runtime: &Arc<Runtime>, settings: Settings) -> Result<(), SettingsError> {
    let mut settings = settings;
    settings.poll_interval_secs = settings
        .poll_interval_secs
        .clamp(MIN_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS);
    settings.branch_patterns = clean_patterns(settings.branch_patterns);
    settings.ignored_workflows = clean_patterns(settings.ignored_workflows);
    check_patterns(&settings.branch_patterns)?;
    check_patterns(&settings.ignored_workflows)?;
    runtime.update_config(|c| c.settings = settings)?;
    runtime.restart();
    Ok(())
}

/// Rejects a list longer than `MAX_PATTERNS` or a pattern longer than `MAX_PATTERN_CHARS`.
pub fn check_patterns(patterns: &[String]) -> Result<(), SettingsError> {
    if patterns.len() > MAX_PATTERNS {
        return Err(SettingsError::InvalidInput(format!(
            "a list can hold at most {MAX_PATTERNS} patterns"
        )));
    }
    let too_long = patterns
        .iter()
        .any(|p| p.chars().count() > MAX_PATTERN_CHARS);
    if too_long {
        return Err(SettingsError::InvalidInput(format!(
            "a pattern can have at most {MAX_PATTERN_CHARS} characters"
        )));
    }
    Ok(())
}

/// Trims patterns and drops empty ones.
pub fn clean_patterns(patterns: Vec<String>) -> Vec<String> {
    patterns
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(label: &str) -> Account {
        Account::new(AccountKind::GitHub, label, None)
    }

    fn gl(label: &str, base: &str) -> Account {
        Account::new(AccountKind::GitLab, label, Some(base.into()))
    }

    fn repo(id: u64) -> RepoInfo {
        RepoInfo {
            id,
            full_name: format!("acme/r{id}"),
            web_url: format!("https://github.com/acme/r{id}"),
            default_branch: "main".into(),
        }
    }

    #[test]
    fn same_instance_rules() {
        assert!(same_instance(&gh("a"), &gh("b")));
        assert!(same_instance(
            &gl("a", "https://x.test/"),
            &gl("b", "https://x.test/")
        ));
        assert!(!same_instance(
            &gl("a", "https://x.test/"),
            &gl("b", "https://y.test/")
        ));
        assert!(!same_instance(&gh("a"), &gl("b", "https://x.test/")));
    }

    #[test]
    fn mark_watched_distinguishes_here_and_elsewhere() {
        let a = gh("Work");
        let b = gh("Home");
        let other_instance = gl("GL", "https://x.test/");
        let watched = vec![
            WatchedRepo {
                account_id: a.id.clone(),
                repo: repo(1),
                branch_patterns: None,
                ignored_workflows: None,
                include_tags: None,
            },
            WatchedRepo {
                account_id: b.id.clone(),
                repo: repo(2),
                branch_patterns: None,
                ignored_workflows: None,
                include_tags: None,
            },
            WatchedRepo {
                account_id: other_instance.id.clone(),
                repo: repo(3),
                branch_patterns: None,
                ignored_workflows: None,
                include_tags: None,
            },
        ];
        let accounts = vec![a.clone(), b.clone(), other_instance];
        let marked = mark_watched(
            &accounts,
            &watched,
            &a,
            vec![repo(1), repo(2), repo(3), repo(4)],
        );
        assert!(marked[0].watched && marked[0].watched_via.is_none());
        assert!(!marked[1].watched);
        assert_eq!(marked[1].watched_via.as_deref(), Some("Home"));
        // Same numeric ID on another instance is a different repo.
        assert!(!marked[2].watched && marked[2].watched_via.is_none());
        assert!(!marked[3].watched && marked[3].watched_via.is_none());
    }

    fn watched(account_id: &str, id: u64) -> WatchedRepo {
        WatchedRepo {
            account_id: account_id.into(),
            repo: repo(id),
            branch_patterns: Some(vec!["main".into()]),
            ignored_workflows: None,
            include_tags: Some(true),
        }
    }

    #[test]
    fn unwatched_entries_are_taken_once() {
        let store = UnwatchedRepos::default();
        let token = store.keep(watched("a", 1));
        assert_eq!(
            token,
            RestoreToken {
                account_id: "a".into(),
                repo_id: 1
            }
        );
        assert_eq!(store.take(&token), Some(watched("a", 1)));
        assert_eq!(store.take(&token), None);
    }

    #[test]
    fn unwatched_entries_expire() {
        let store = UnwatchedRepos::with_ttl(Duration::ZERO);
        let token = store.keep(watched("a", 1));
        assert_eq!(store.take(&token), None);
    }

    #[test]
    fn keeping_an_entry_drops_expired_ones() {
        let store = UnwatchedRepos::with_ttl(Duration::ZERO);
        store.keep(watched("a", 1));
        store.keep(watched("a", 2));
        assert_eq!(store.entries.lock().unwrap().len(), 1);
    }

    #[test]
    fn restore_tokens_serialize_as_account_and_repo_ids() {
        let token = RestoreToken {
            account_id: "a".into(),
            repo_id: 7,
        };
        assert_eq!(
            serde_json::to_value(&token).unwrap(),
            serde_json::json!({ "account_id": "a", "repo_id": 7 })
        );
    }

    #[test]
    fn seeding_keeps_a_cached_list() {
        let cache = RepoListCache::default();
        cache.seed("a", vec![repo(1)]);
        cache.seed("a", vec![repo(2)]);
        assert_eq!(cache.get("a"), Some(vec![repo(1)]));
        let shared = cache.clone();
        shared.set("b", vec![repo(3)]);
        assert_eq!(cache.get("b"), Some(vec![repo(3)]));
    }

    #[test]
    fn pattern_lists_are_bounded() {
        let at_limit = vec!["x".repeat(MAX_PATTERN_CHARS); MAX_PATTERNS];
        assert!(check_patterns(&at_limit).is_ok());
        let too_many = vec!["main".to_string(); MAX_PATTERNS + 1];
        assert!(matches!(
            check_patterns(&too_many),
            Err(SettingsError::InvalidInput(_))
        ));
        let too_long = vec!["é".repeat(MAX_PATTERN_CHARS + 1)];
        assert!(matches!(
            check_patterns(&too_long),
            Err(SettingsError::InvalidInput(_))
        ));
    }

    #[test]
    fn clean_patterns_trims_and_drops_empty() {
        assert_eq!(
            clean_patterns(vec![
                " main ".into(),
                "".into(),
                "release/*".into(),
                "  ".into()
            ]),
            vec!["main".to_string(), "release/*".to_string()]
        );
    }
}
