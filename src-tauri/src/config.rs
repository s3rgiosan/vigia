//! `config.json`: accounts without tokens, watched repos, filters, and settings.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::providers::RepoInfo;

pub const SCHEMA_VERSION: u32 = 1;
pub const FILE_NAME: &str = "config.json";
/// Debug builds use their own file so they never touch a release build's config.
pub const DEV_FILE_NAME: &str = "config.dev.json";

pub const MIN_POLL_INTERVAL_SECS: u64 = 15;
pub const MAX_POLL_INTERVAL_SECS: u64 = 3600;
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    #[default]
    GitHub,
    GitLab,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub kind: AccountKind,
    pub label: String,
    /// Normalized GitLab base URL. Absent for GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Provider user ID from token validation. GitHub accounts of one user share a rate limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

/// Branch, workflow and tag overrides at one level of the filter resolution. A `None` field
/// inherits the next level; an empty list is an override that means "nothing".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterSet {
    /// Branch glob patterns. An empty list means the default branch only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_patterns: Option<Vec<String>>,
    /// Workflow glob patterns to ignore. An empty list ignores nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_workflows: Option<Vec<String>>,
    /// Whether tag runs count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_tags: Option<bool>,
}

impl FilterSet {
    /// No overrides: every field inherits the next level.
    pub const INHERIT: FilterSet = FilterSet {
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    };

    pub fn is_empty(&self) -> bool {
        *self == FilterSet::INHERIT
    }
}

/// The host GitHub organizations are keyed by.
pub const GITHUB_HOST: &str = "github.com";

/// Identifies an organization: the provider host and the first path segment of a repo's full
/// name (a GitHub owner or a GitLab top-level group, personal namespaces included).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrgKey {
    /// `github.com`, or the GitLab base URL's lowercase host with `:port` when the port is not
    /// the scheme's default.
    pub host: String,
    /// The owner as the repo's full name spells it.
    pub owner: String,
}

impl OrgKey {
    /// The organization of a repo named `full_name` reached through `account`.
    pub fn of(account: &Account, full_name: &str) -> OrgKey {
        let owner = full_name.split('/').next().unwrap_or_default();
        OrgKey {
            host: org_host(account),
            owner: owner.to_string(),
        }
    }

    /// Hosts and owners compare case-insensitively.
    pub fn matches(&self, host: &str, owner: &str) -> bool {
        self.host.eq_ignore_ascii_case(host) && self.owner.eq_ignore_ascii_case(owner)
    }

    /// The overrides saved for this organization in `organizations`, or none.
    pub fn filters_in<'a>(&self, organizations: &'a [OrgFilters]) -> &'a FilterSet {
        organizations
            .iter()
            .find(|o| o.matches(&self.host, &self.owner))
            .map_or(&FilterSet::INHERIT, |o| &o.filters)
    }
}

/// The host part of the organization keys of an account's repos.
pub fn org_host(account: &Account) -> String {
    match account.kind {
        AccountKind::GitHub => GITHUB_HOST.to_string(),
        AccountKind::GitLab => account
            .base_url
            .as_deref()
            .and_then(|raw| url::Url::parse(raw).ok())
            .and_then(|url| {
                let host = url.host_str()?.to_ascii_lowercase();
                Some(match url.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host,
                })
            })
            .unwrap_or_default(),
    }
}

/// Filter overrides shared by every watched repo of one organization, across accounts. A repo's
/// own override wins over these, and a `None` field inherits the global setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgFilters {
    pub host: String,
    pub owner: String,
    #[serde(flatten)]
    pub filters: FilterSet,
}

impl OrgFilters {
    /// Whether this entry belongs to the organization at `host` and `owner`, ignoring case.
    pub fn matches(&self, host: &str, owner: &str) -> bool {
        self.host.eq_ignore_ascii_case(host) && self.owner.eq_ignore_ascii_case(owner)
    }
}

impl Account {
    pub fn new(kind: AccountKind, label: &str, base_url: Option<String>) -> Account {
        Account {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            label: label.to_string(),
            base_url,
            user_id: None,
            login: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchedRepo {
    pub account_id: String,
    pub repo: RepoInfo,
    /// Glob patterns overriding the organization and global filters. `None` inherits them; an empty
    /// list means the default branch only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_patterns: Option<Vec<String>>,
    /// Workflow glob patterns overriding the organization and global ignore lists. `None` inherits
    /// them; an empty list ignores nothing for this repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignored_workflows: Option<Vec<String>>,
    /// Whether tag runs count for this repo. `None` inherits the organization or global setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_tags: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub poll_interval_secs: u64,
    pub exclude_pull_requests: bool,
    /// Global glob patterns. Empty means the default branch only.
    pub branch_patterns: Vec<String>,
    /// Global workflow glob patterns to ignore. Empty ignores nothing.
    pub ignored_workflows: Vec<String>,
    /// Whether runs started by a tag count, in addition to the runs the branch filter keeps.
    pub include_tags: bool,
    pub notify_failures: bool,
    pub notify_recoveries: bool,
    pub launch_at_login: bool,
    /// Checks GitHub for a newer release a minute after launch and once a day.
    #[serde(default = "default_true")]
    pub check_for_updates: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            exclude_pull_requests: true,
            branch_patterns: Vec::new(),
            ignored_workflows: Vec::new(),
            include_tags: false,
            notify_failures: true,
            notify_recoveries: false,
            launch_at_login: false,
            check_for_updates: true,
        }
    }
}

impl Settings {
    /// The configured interval, kept within its allowed range.
    pub fn poll_interval_secs(&self) -> u64 {
        self.poll_interval_secs
            .clamp(MIN_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub accounts: Vec<Account>,
    pub repos: Vec<WatchedRepo>,
    /// Organization-level filter overrides. Only entries with at least one override are kept.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub organizations: Vec<OrgFilters>,
    pub settings: Settings,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            schema_version: SCHEMA_VERSION,
            accounts: Vec::new(),
            repos: Vec::new(),
            organizations: Vec::new(),
            settings: Settings::default(),
        }
    }
}

impl Config {
    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// The organization of a watched repo, or `None` when its account is unknown.
    pub fn org_key(&self, repo: &WatchedRepo) -> Option<OrgKey> {
        let account = self.account(&repo.account_id)?;
        Some(OrgKey::of(account, &repo.repo.full_name))
    }

    /// The overrides of a watched repo's organization, or none when nothing is saved for it.
    pub fn org_filters_of(&self, repo: &WatchedRepo) -> &FilterSet {
        self.org_key(repo).map_or(&FilterSet::INHERIT, |key| {
            key.filters_in(&self.organizations)
        })
    }

    /// Drops organization entries that override nothing.
    pub fn drop_empty_organizations(&mut self) {
        self.organizations.retain(|o| !o.filters.is_empty());
    }

    pub fn repos_of<'a>(
        &'a self,
        account_id: &'a str,
    ) -> impl Iterator<Item = &'a WatchedRepo> + 'a {
        self.repos
            .iter()
            .filter(move |r| r.account_id == account_id)
    }

    /// Removes an account together with its watched repos.
    pub fn remove_account(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
        self.repos.retain(|r| r.account_id != id);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read or write the config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the config file is not valid: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("the config was written by a newer Vigia and is read-only")]
    ReadOnly,
}

/// Only the version field, read before the rest so a newer file with changed shapes is still
/// recognized as newer.
#[derive(Deserialize)]
struct VersionProbe {
    #[serde(default = "current_version")]
    schema_version: u32,
}

fn current_version() -> u32 {
    SCHEMA_VERSION
}

/// Upgrades a config read from an older schema in place.
fn migrate(config: &mut Config, _from_version: u32) {
    config.schema_version = SCHEMA_VERSION;
}

/// Owns the config file. Saves are atomic and refused when the file is newer than this build
/// or could not be read.
#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
    config: Config,
    read_only: bool,
    loaded_from_file: bool,
    load_error: Option<String>,
}

impl ConfigStore {
    /// Loads the file, or starts with defaults when it does not exist.
    pub fn load(path: &Path) -> Result<ConfigStore, ConfigError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let mut read_only = false;
        let mut loaded_from_file = false;
        let config = match bytes {
            None => Config::default(),
            Some(bytes) => {
                let probe = serde_json::from_slice::<VersionProbe>(&bytes)?;
                if probe.schema_version > SCHEMA_VERSION {
                    log::warn!(
                        "config schema {} is newer than {}; running read-only",
                        probe.schema_version,
                        SCHEMA_VERSION
                    );
                    read_only = true;
                    serde_json::from_slice::<Config>(&bytes).unwrap_or_default()
                } else {
                    let mut config = serde_json::from_slice::<Config>(&bytes)?;
                    migrate(&mut config, probe.schema_version);
                    loaded_from_file = true;
                    config
                }
            }
        };
        Ok(ConfigStore {
            path: path.to_path_buf(),
            config,
            read_only,
            loaded_from_file,
            load_error: None,
        })
    }

    /// Like `load`, but a file that does not parse is moved aside and replaced by defaults,
    /// and an I/O failure yields a read-only store with defaults that records the message.
    pub fn load_or_recover(path: &Path) -> Result<ConfigStore, ConfigError> {
        match ConfigStore::load(path) {
            Err(ConfigError::Parse(e)) => {
                let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
                let backup = path.with_file_name(format!("{FILE_NAME}.corrupt-{stamp}"));
                log::error!(
                    "config does not parse ({e}); moving it to {} and starting fresh",
                    backup.display()
                );
                match fs::rename(path, &backup) {
                    Ok(()) => ConfigStore::load(path).or_else(|e| Ok(ConfigStore::unavailable(&e))),
                    Err(e) => Ok(ConfigStore::unavailable(&e.into())),
                }
            }
            Err(e @ ConfigError::Io(_)) => Ok(ConfigStore::unavailable(&e)),
            other => other,
        }
    }

    /// A read-only store with defaults, used when the file cannot be read or replaced.
    pub fn unavailable(error: &ConfigError) -> ConfigStore {
        log::error!("config unavailable, running read-only: {error}");
        ConfigStore {
            path: PathBuf::new(),
            config: Config::default(),
            read_only: true,
            loaded_from_file: false,
            load_error: Some(error.to_string()),
        }
    }

    pub fn in_memory(config: Config) -> ConfigStore {
        ConfigStore {
            path: PathBuf::new(),
            config,
            read_only: false,
            loaded_from_file: false,
            load_error: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// True when an existing file parsed at the current or an older schema. False for a
    /// missing file, a recovered corrupt file, a newer file, or an unreadable file.
    pub fn loaded_from_file(&self) -> bool {
        self.loaded_from_file
    }

    /// Why the file could not be loaded, when the store fell back to read-only defaults.
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Applies a change and writes the file. Nothing changes in memory when the write fails.
    pub fn update(&mut self, change: impl FnOnce(&mut Config)) -> Result<(), ConfigError> {
        if self.read_only {
            return Err(ConfigError::ReadOnly);
        }
        let mut next = self.config.clone();
        change(&mut next);
        next.drop_empty_organizations();
        next.schema_version = SCHEMA_VERSION;
        if !self.path.as_os_str().is_empty() {
            write_atomic(&self.path, &next)?;
        }
        self.config = next;
        Ok(())
    }
}

/// Orphaned tokens are removed only when the config reflects what the user saved: it came from
/// an existing file, is writable, and the secret store is usable.
pub fn should_prune_secrets(config: &ConfigStore, secrets_blocked: bool) -> bool {
    config.loaded_from_file() && !config.read_only() && !secrets_blocked
}

fn write_atomic(path: &Path, config: &Config) -> Result<(), ConfigError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(&mut temp, config)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        (dir, path)
    }

    #[test]
    fn missing_file_starts_with_defaults() {
        let (_dir, path) = temp_path();
        let store = ConfigStore::load(&path).unwrap();
        assert_eq!(store.config(), &Config::default());
        assert!(!store.read_only());
    }

    #[test]
    fn update_writes_and_reloads() {
        let (_dir, path) = temp_path();
        let mut store = ConfigStore::load(&path).unwrap();
        store
            .update(|c| {
                c.accounts
                    .push(Account::new(AccountKind::GitHub, "work", None));
                c.settings.poll_interval_secs = 120;
            })
            .unwrap();
        let again = ConfigStore::load(&path).unwrap();
        assert_eq!(again.config().accounts.len(), 1);
        assert_eq!(again.config().settings.poll_interval_secs, 120);
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn newer_schema_is_read_only() {
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "schema_version": 99, "future_field": true }"#).unwrap();
        let mut store = ConfigStore::load(&path).unwrap();
        assert!(store.read_only());
        let err = store
            .update(|c| c.settings.poll_interval_secs = 1)
            .unwrap_err();
        assert!(matches!(err, ConfigError::ReadOnly));
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("future_field"));
    }

    #[test]
    fn newer_schema_with_changed_shapes_stays_untouched() {
        let (_dir, path) = temp_path();
        let raw = r#"{ "schema_version": 99, "accounts": "now a string", "repos": 7 }"#;
        fs::write(&path, raw).unwrap();
        let store = ConfigStore::load_or_recover(&path).unwrap();
        assert!(store.read_only());
        assert!(store.load_error().is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
        let backups = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .count();
        assert_eq!(backups, 0);
    }

    #[test]
    fn save_stamps_the_current_schema_version() {
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{ "schema_version": 0 }"#).unwrap();
        let mut store = ConfigStore::load(&path).unwrap();
        assert_eq!(store.config().schema_version, SCHEMA_VERSION);
        store.update(|c| c.schema_version = 0).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
    }

    #[test]
    fn io_failure_falls_back_to_a_read_only_store() {
        let (_dir, path) = temp_path();
        // A directory at the config path makes the read fail with an I/O error.
        fs::create_dir(&path).unwrap();
        let mut store = ConfigStore::load_or_recover(&path).unwrap();
        assert!(store.read_only());
        assert!(store.load_error().is_some());
        assert!(matches!(
            store.update(|c| c.settings.poll_interval_secs = 99),
            Err(ConfigError::ReadOnly)
        ));
    }

    #[test]
    fn invalid_json_is_an_error() {
        let (_dir, path) = temp_path();
        fs::write(&path, "{ nope").unwrap();
        assert!(matches!(
            ConfigStore::load(&path),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn corrupt_file_is_backed_up_and_replaced_by_defaults() {
        let (_dir, path) = temp_path();
        fs::write(&path, "{ nope").unwrap();
        let store = ConfigStore::load_or_recover(&path).unwrap();
        assert_eq!(store.config(), &Config::default());
        assert!(!store.read_only());
        let backups: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
            .collect();
        assert_eq!(backups.len(), 1);
    }

    #[test]
    fn loaded_from_file_is_true_only_for_a_parsed_existing_file() {
        let (_dir, path) = temp_path();
        assert!(!ConfigStore::load(&path).unwrap().loaded_from_file());
        fs::write(&path, r#"{"schema_version": 1}"#).unwrap();
        assert!(ConfigStore::load(&path).unwrap().loaded_from_file());
        assert!(ConfigStore::load_or_recover(&path)
            .unwrap()
            .loaded_from_file());
    }

    #[test]
    fn recovered_and_unavailable_stores_are_not_loaded_from_file() {
        let (_dir, path) = temp_path();
        fs::write(&path, "{ not json").unwrap();
        let store = ConfigStore::load_or_recover(&path).unwrap();
        assert!(!store.loaded_from_file());
        assert!(!should_prune_secrets(&store, false));
        let error = ConfigError::Io(std::io::Error::other("denied"));
        assert!(!ConfigStore::unavailable(&error).loaded_from_file());
        assert!(!ConfigStore::in_memory(Config::default()).loaded_from_file());
    }

    #[test]
    fn prune_requires_a_loaded_writable_config_and_usable_secrets() {
        let (_dir, path) = temp_path();
        fs::write(&path, r#"{"schema_version": 1}"#).unwrap();
        let store = ConfigStore::load(&path).unwrap();
        assert!(should_prune_secrets(&store, false));
        assert!(!should_prune_secrets(&store, true));
        fs::write(&path, r#"{"schema_version": 9999}"#).unwrap();
        let newer = ConfigStore::load(&path).unwrap();
        assert!(!should_prune_secrets(&newer, false));
    }

    #[test]
    fn interval_floor_applies() {
        let settings = Settings {
            poll_interval_secs: 3,
            ..Default::default()
        };
        assert_eq!(settings.poll_interval_secs(), MIN_POLL_INTERVAL_SECS);
    }

    #[test]
    fn interval_ceiling_applies() {
        let settings = Settings {
            poll_interval_secs: u64::MAX,
            ..Default::default()
        };
        assert_eq!(settings.poll_interval_secs(), MAX_POLL_INTERVAL_SECS);
    }

    #[test]
    fn update_checks_default_to_on_for_files_without_the_setting() {
        let (_dir, path) = temp_path();
        fs::write(
            &path,
            r#"{"schema_version": 1, "settings": {"launch_at_login": true}}"#,
        )
        .unwrap();
        let store = ConfigStore::load(&path).unwrap();
        assert!(store.config().settings.check_for_updates);
        assert!(store.config().settings.launch_at_login);
        assert!(Settings::default().check_for_updates);
    }

    #[test]
    fn update_checks_turned_off_survive_a_save() {
        let (_dir, path) = temp_path();
        let mut store = ConfigStore::load_or_recover(&path).unwrap();
        store
            .update(|c| c.settings.check_for_updates = false)
            .unwrap();
        let reloaded = ConfigStore::load(&path).unwrap();
        assert!(!reloaded.config().settings.check_for_updates);
    }

    #[test]
    fn removing_an_account_drops_its_repos() {
        let mut config = Config::default();
        let a = Account::new(AccountKind::GitHub, "a", None);
        let b = Account::new(AccountKind::GitLab, "b", Some("https://x.test/".into()));
        let repo = RepoInfo {
            id: 1,
            full_name: "x/y".into(),
            web_url: "https://x.test/x/y".into(),
            default_branch: "main".into(),
        };
        config.repos.push(WatchedRepo {
            account_id: a.id.clone(),
            repo: repo.clone(),
            branch_patterns: None,
            ignored_workflows: None,
            include_tags: None,
        });
        config.repos.push(WatchedRepo {
            account_id: b.id.clone(),
            repo,
            branch_patterns: Some(vec!["release/*".into()]),
            ignored_workflows: None,
            include_tags: None,
        });
        config.accounts.push(a.clone());
        config.accounts.push(b);
        config.remove_account(&a.id);
        assert_eq!(config.accounts.len(), 1);
        assert_eq!(config.repos.len(), 1);
        assert_ne!(config.repos[0].account_id, a.id);
    }

    #[test]
    fn ignored_workflows_round_trip_and_old_files_load() {
        let old = r#"{"schema_version":1,"accounts":[],"repos":[{"account_id":"a","repo":{"id":1,"full_name":"acme/widgets","web_url":"https://example.test/acme/widgets","default_branch":"main"}}],"settings":{"poll_interval_secs":60}}"#;
        let config: Config = serde_json::from_str(old).unwrap();
        assert!(config.settings.ignored_workflows.is_empty());
        assert_eq!(config.repos[0].ignored_workflows, None);
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("\"ignored_workflows\":null"));

        let mut config = config;
        config.settings.ignored_workflows = vec!["Dependabot*".into()];
        config.repos[0].ignored_workflows = Some(vec![]);
        let json = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        assert_eq!(back.repos[0].ignored_workflows, Some(vec![]));
    }

    #[test]
    fn include_tags_round_trips_and_old_files_load() {
        let old = r#"{"schema_version":1,"accounts":[],"repos":[{"account_id":"a","repo":{"id":1,"full_name":"acme/widgets","web_url":"https://example.test/acme/widgets","default_branch":"main"}}],"settings":{"poll_interval_secs":60}}"#;
        let config: Config = serde_json::from_str(old).unwrap();
        assert!(!config.settings.include_tags);
        assert_eq!(config.repos[0].include_tags, None);
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("\"include_tags\":null"));
        assert!(json.contains("\"include_tags\":false"));

        let mut config = config;
        config.settings.include_tags = true;
        config.repos[0].include_tags = Some(false);
        let json = serde_json::to_string(&config).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        assert_eq!(back.repos[0].include_tags, Some(false));
    }

    #[test]
    fn organizations_round_trip_and_old_files_load() {
        let old = r#"{"schema_version":1,"accounts":[{"id":"a","kind":"github","label":"Acme"}],"repos":[],"settings":{"poll_interval_secs":60}}"#;
        let config: Config = serde_json::from_str(old).unwrap();
        assert!(config.organizations.is_empty());
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("organizations"));

        let mut config = config;
        config.organizations.push(OrgFilters {
            host: "github.com".into(),
            owner: "Acme".into(),
            filters: FilterSet {
                branch_patterns: Some(vec![]),
                ignored_workflows: Some(vec![]),
                include_tags: None,
            },
        });
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains(
            r#""organizations":[{"host":"github.com","owner":"Acme","branch_patterns":[],"ignored_workflows":[]}]"#
        ));
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        assert_eq!(back.organizations[0].filters.branch_patterns, Some(vec![]));
        assert_eq!(back.organizations[0].filters.include_tags, None);
    }

    #[test]
    fn account_filters_from_an_unreleased_build_are_ignored() {
        let raw = r#"{"schema_version":1,"accounts":[{"id":"a","kind":"github","label":"Acme","filters":{"branch_patterns":["release/*"],"include_tags":true}}],"repos":[],"settings":{}}"#;
        let (_dir, path) = temp_path();
        fs::write(&path, raw).unwrap();
        let mut store = ConfigStore::load(&path).unwrap();
        assert!(!store.read_only());
        assert_eq!(store.config().accounts[0].label, "Acme");
        assert!(store.config().organizations.is_empty());
        store
            .update(|c| c.settings.poll_interval_secs = 90)
            .unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("filters"));
    }

    #[test]
    fn saving_drops_organizations_that_override_nothing() {
        let (_dir, path) = temp_path();
        let mut store = ConfigStore::load(&path).unwrap();
        store
            .update(|c| {
                c.organizations = vec![
                    OrgFilters {
                        host: "github.com".into(),
                        owner: "acme".into(),
                        filters: FilterSet::INHERIT,
                    },
                    OrgFilters {
                        host: "github.com".into(),
                        owner: "example".into(),
                        filters: FilterSet {
                            ignored_workflows: Some(vec![]),
                            ..Default::default()
                        },
                    },
                ];
            })
            .unwrap();
        let again = ConfigStore::load(&path).unwrap();
        assert_eq!(again.config().organizations.len(), 1);
        assert_eq!(again.config().organizations[0].owner, "example");
        assert_eq!(
            again.config().organizations[0].filters.ignored_workflows,
            Some(vec![])
        );
    }

    fn gitlab(base_url: &str) -> Account {
        Account::new(AccountKind::GitLab, "gl", Some(base_url.into()))
    }

    #[test]
    fn org_keys_use_the_host_and_the_first_path_segment() {
        let github = Account::new(AccountKind::GitHub, "gh", None);
        assert_eq!(
            OrgKey::of(&github, "Acme/widgets"),
            OrgKey {
                host: "github.com".into(),
                owner: "Acme".into()
            }
        );
        let key = OrgKey::of(&gitlab("https://GitLab.Example.test:8443/"), "acme/web/app");
        assert_eq!(key.host, "gitlab.example.test:8443");
        assert_eq!(key.owner, "acme");
        let default_port = OrgKey::of(&gitlab("https://gitlab.example.test:443/"), "acme/app");
        assert_eq!(default_port.host, "gitlab.example.test");
        let insecure = OrgKey::of(&gitlab("http://gitlab.example.test:8080/"), "acme/app");
        assert_eq!(insecure.host, "gitlab.example.test:8080");
    }

    #[test]
    fn org_keys_match_owners_case_insensitively() {
        let key = OrgKey::of(
            &Account::new(AccountKind::GitHub, "gh", None),
            "ACME/widgets",
        );
        assert!(key.matches("github.com", "acme"));
        assert!(key.matches("GitHub.com", "Acme"));
        assert!(!key.matches("gitlab.example.test", "acme"));
        let organizations = vec![OrgFilters {
            host: "github.com".into(),
            owner: "acme".into(),
            filters: FilterSet {
                include_tags: Some(true),
                ..Default::default()
            },
        }];
        assert_eq!(key.filters_in(&organizations).include_tags, Some(true));
        let other = OrgKey::of(&gitlab("https://gitlab.example.test/"), "acme/widgets");
        assert!(other.filters_in(&organizations).is_empty());
    }

    #[test]
    fn repos_of_unknown_accounts_inherit_every_filter() {
        let config = Config::default();
        let repo = WatchedRepo {
            account_id: "missing".into(),
            repo: RepoInfo {
                id: 1,
                full_name: "acme/widgets".into(),
                web_url: "https://example.test/acme/widgets".into(),
                default_branch: "main".into(),
            },
            branch_patterns: None,
            ignored_workflows: None,
            include_tags: None,
        };
        assert!(config.org_key(&repo).is_none());
        assert!(config.org_filters_of(&repo).is_empty());
    }
}
