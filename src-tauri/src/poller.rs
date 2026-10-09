//! Per-account polling: scheduling, backoff, pause, refresh, and snapshot assembly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use tokio::sync::{Notify, Semaphore};
use tokio::task::JoinSet;

use crate::aggregate::{overall, repo_state, select_groups, Selection};
use crate::config::{
    Account, AccountKind, OrgFilters, RepoOrder, Settings, WatchedRepo, MIN_POLL_INTERVAL_SECS,
};
use crate::filters::{selection_changed, FilterCompiler, FilterScope, RepoFilters};
use crate::model::{RepoState, RepoStatus, RunState, TrayColor};
use crate::notes;
use crate::pool::{clamp_reset, wait_until, PoolState, RequestClass, FAST_POLL_SECS};
use crate::providers::{FetchRequest, Provider, ProviderError, RepoCache, RepoInfo};
use crate::updates::UpdateInfo;

pub const CONCURRENT_REQUESTS: usize = 6;
/// Network failures in a row before a repo shows Error.
pub const FAILURES_BEFORE_ERROR: u32 = 3;
pub const BACKOFF_MAX_SECS: u64 = 300;
/// A run updated within this window keeps its repo on fast polling.
pub const FAST_POLL_WINDOW: Duration = Duration::hours(1);
pub const JITTER: f64 = 0.1;
/// Heartbeat period used to notice sleep.
pub const HEARTBEAT_SECS: u64 = 5;
/// A wall-clock gap between heartbeats above this means the Mac slept.
pub const WAKE_GAP_SECS: i64 = 35;
/// After a wake, network failures do not count toward Error for this long.
pub const WAKE_GRACE_SECS: i64 = 30;
/// The longest poll interval used for scheduling, whatever the config says.
pub const MAX_POLL_INTERVAL_SECS: u64 = 3600;
/// Due times are rounded up to whole multiples of this many seconds of Unix time, so repos that
/// fall due close together are polled in one wakeup.
pub const DUE_BUCKET_SECS: i64 = 5;
/// Bounds on a server's `Retry-After`.
pub const RETRY_AFTER_MIN_SECS: u64 = 1;
pub const RETRY_AFTER_MAX_SECS: u64 = 3600;

/// The configured poll interval, kept between the minimum and `MAX_POLL_INTERVAL_SECS`.
pub fn base_interval_secs(settings: &Settings) -> u64 {
    settings
        .poll_interval_secs()
        .clamp(MIN_POLL_INTERVAL_SECS, MAX_POLL_INTERVAL_SECS)
}

/// `now` plus `secs`, rounded up to the next `DUE_BUCKET_SECS` boundary.
pub fn due_after(now: OffsetDateTime, secs: u64) -> OffsetDateTime {
    let due = wait_until(now, secs);
    let ts = due.unix_timestamp();
    let rem = ts.rem_euclid(DUE_BUCKET_SECS);
    if rem == 0 && due.nanosecond() == 0 {
        return due;
    }
    OffsetDateTime::from_unix_timestamp(ts - rem + DUE_BUCKET_SECS).unwrap_or(due)
}

/// Whether a gap between two heartbeats means the Mac was asleep.
pub fn is_wake_gap(gap: Duration) -> bool {
    gap > Duration::seconds(WAKE_GAP_SECS)
}

/// Why an account is in an auth error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthReason {
    /// The provider answered 401.
    Rejected,
    /// No token is saved for the account.
    MissingToken,
    /// The Keychain could not be read.
    Keychain,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    pub id: String,
    pub label: String,
    pub kind: AccountKind,
    pub auth_error: bool,
    /// The cause of `auth_error`; `None` while the account has no auth error.
    pub auth_reason: Option<AuthReason>,
    pub unreachable: bool,
    /// Unix time until which the pool is rate limited.
    pub rate_limited_until: Option<i64>,
    pub effective_interval_secs: u64,
    /// The poll interval from settings, before the pool stretches it.
    pub configured_interval_secs: u64,
    /// The account's token could not be read because Keychain access was denied.
    pub keychain_denied: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSnapshot {
    pub account_id: String,
    pub repo: RepoInfo,
    pub state: RepoState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    pub paused: bool,
    /// Keychain access failed; token writes are blocked.
    pub secrets_blocked: bool,
    /// The config was written by a newer Vigia; config writes are disabled.
    pub config_read_only: bool,
    /// The config file could not be read or parsed.
    pub config_error: Option<String>,
    /// A newer release found by an update check.
    pub update: Option<UpdateInfo>,
    pub color: TrayColor,
    pub tooltip: String,
    pub accounts: Vec<AccountSnapshot>,
    pub repos: Vec<RepoSnapshot>,
    pub repo_order: RepoOrder,
    pub group_by_org: bool,
}

impl Snapshot {
    pub fn empty(now: OffsetDateTime, paused: bool) -> Snapshot {
        let o = overall(std::iter::empty(), paused);
        Snapshot {
            generated_at: now,
            paused,
            secrets_blocked: false,
            config_read_only: false,
            config_error: None,
            update: None,
            color: o.color(),
            tooltip: o.tooltip(),
            accounts: Vec::new(),
            repos: Vec::new(),
            repo_order: RepoOrder::Name,
            group_by_org: true,
        }
    }
}

/// App-level flags carried on every snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFlags {
    pub secrets_blocked: bool,
    pub config_read_only: bool,
    pub config_error: Option<String>,
    pub update: Option<UpdateInfo>,
    pub repo_order: RepoOrder,
    pub group_by_org: bool,
}

impl Default for SnapshotFlags {
    fn default() -> SnapshotFlags {
        SnapshotFlags {
            secrets_blocked: false,
            config_read_only: false,
            config_error: None,
            update: None,
            repo_order: RepoOrder::Name,
            group_by_org: true,
        }
    }
}

/// Accumulates account and repo states and builds snapshots.
#[derive(Default)]
pub struct SnapshotStore {
    accounts: Mutex<HashMap<String, AccountSnapshot>>,
    repos: Mutex<HashMap<(String, u64), RepoSnapshot>>,
    /// Bumped whenever a stored account or repo actually changes.
    version: AtomicU64,
}

impl SnapshotStore {
    /// Changes whenever the stored accounts or repos change; equal writes leave it alone.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
    }

    pub fn set_account(&self, account: AccountSnapshot) {
        let mut accounts = self.accounts.lock().unwrap();
        if accounts.get(&account.id) != Some(&account) {
            accounts.insert(account.id.clone(), account);
            self.bump();
        }
    }

    pub fn set_repo(&self, repo: RepoSnapshot) {
        let key = (repo.account_id.clone(), repo.repo.id);
        let mut repos = self.repos.lock().unwrap();
        if repos.get(&key) != Some(&repo) {
            repos.insert(key, repo);
            self.bump();
        }
    }

    pub fn account(&self, account_id: &str) -> Option<AccountSnapshot> {
        self.accounts.lock().unwrap().get(account_id).cloned()
    }

    pub fn repo_state(&self, account_id: &str, repo_id: u64) -> Option<RepoState> {
        self.repos
            .lock()
            .unwrap()
            .get(&(account_id.to_string(), repo_id))
            .map(|r| r.state.clone())
    }

    pub fn remove_account(&self, account_id: &str) {
        let removed_account = self.accounts.lock().unwrap().remove(account_id).is_some();
        let removed_repos = {
            let mut repos = self.repos.lock().unwrap();
            let before = repos.len();
            repos.retain(|(id, _), _| id != account_id);
            repos.len() != before
        };
        if removed_account || removed_repos {
            self.bump();
        }
    }

    /// Drops every repo and account that is not in `keep`, which lists `(account_id, repo_id)`.
    /// Drops repos that are no longer watched and accounts that are no longer configured.
    pub fn retain_repos(&self, keep: &[(String, u64)], accounts: &[String]) {
        let removed_repos = {
            let mut repos = self.repos.lock().unwrap();
            let before = repos.len();
            repos.retain(|key, _| keep.iter().any(|k| k == key));
            repos.len() != before
        };
        let removed_accounts = {
            let mut stored = self.accounts.lock().unwrap();
            let before = stored.len();
            stored.retain(|id, _| accounts.iter().any(|a| a == id));
            stored.len() != before
        };
        if removed_repos || removed_accounts {
            self.bump();
        }
    }

    pub fn snapshot(&self, now: OffsetDateTime, paused: bool, flags: SnapshotFlags) -> Snapshot {
        let mut accounts: Vec<AccountSnapshot> =
            self.accounts.lock().unwrap().values().cloned().collect();
        accounts.sort_by(|a, b| a.label.cmp(&b.label));
        let mut repos: Vec<RepoSnapshot> = self.repos.lock().unwrap().values().cloned().collect();
        repos.sort_by(|a, b| a.repo.full_name.cmp(&b.repo.full_name));
        let mut repos_with_stale = repos;
        if paused {
            for r in &mut repos_with_stale {
                r.state.stale = true;
            }
        }
        let o = overall(repos_with_stale.iter().map(|r| r.state.status), paused);
        Snapshot {
            generated_at: now,
            paused,
            secrets_blocked: flags.secrets_blocked,
            config_read_only: flags.config_read_only,
            config_error: flags.config_error,
            update: flags.update,
            color: o.color(),
            tooltip: o.tooltip(),
            accounts,
            repos: repos_with_stale,
            repo_order: flags.repo_order,
            group_by_org: flags.group_by_org,
        }
    }
}

/// Shared controls for every account task.
#[derive(Clone)]
pub struct Controls {
    pub paused: Arc<AtomicBool>,
    /// Wakes every task: pause toggled, refresh requested, or shutdown.
    pub wake: Arc<Notify>,
    pub refresh_requested: Arc<AtomicBool>,
    pub shutdown: Arc<AtomicBool>,
}

impl Default for Controls {
    fn default() -> Controls {
        Controls {
            paused: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(Notify::new()),
            refresh_requested: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Controls {
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn refresh_now(&self) {
        self.refresh_requested.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.wake.notify_waiters();
    }
}

/// Runtime state of one watched repo inside a worker.
#[derive(Debug, Clone)]
pub struct RepoRuntime {
    pub watched: WatchedRepo,
    /// The compiled branch and workflow filters and the tag choice the repo is selected with.
    pub filters: RepoFilters,
    pub cache: RepoCache,
    pub state: RepoState,
    /// Network or server failures in a row. Drives the Error threshold and per-repo backoff.
    pub failures: u32,
    /// The last poll of this repo returned an error.
    pub last_failed: bool,
    pub next_due: OffsetDateTime,
    pub fast: bool,
    /// The cache holds a successful fetch. Until it does, every request a fetch sends fills it.
    pub primed: bool,
}

impl RepoRuntime {
    /// A repo selected with the default filters: the default branch, no ignored workflows, no
    /// tag runs.
    pub fn new(watched: WatchedRepo, now: OffsetDateTime) -> RepoRuntime {
        RepoRuntime::with_filters(watched, RepoFilters::default(), now)
    }

    /// A repo selected with `filters`, due at `now`.
    pub fn with_filters(
        watched: WatchedRepo,
        filters: RepoFilters,
        now: OffsetDateTime,
    ) -> RepoRuntime {
        RepoRuntime {
            watched,
            filters,
            cache: RepoCache::default(),
            state: RepoState {
                status: RepoStatus::None,
                representative: None,
                groups: Vec::new(),
                last_checked: None,
                stale: false,
                note: None,
            },
            failures: 0,
            last_failed: false,
            next_due: now,
            fast: false,
            primed: false,
        }
    }

    /// Stands in for a repo while its runtime is out being polled. It shares the repo's
    /// identity and filters but holds no cache or runs.
    fn stand_in(&self) -> RepoRuntime {
        let mut stand_in =
            RepoRuntime::with_filters(self.watched.clone(), self.filters.clone(), self.next_due);
        stand_in.failures = self.failures;
        stand_in
    }

    /// Takes over the state, cache and schedule of the same repo from a replaced worker.
    fn carry_over(&mut self, previous: RepoRuntime) {
        self.cache = previous.cache;
        self.state = previous.state;
        self.failures = previous.failures;
        self.last_failed = previous.last_failed;
        self.next_due = previous.next_due;
        self.fast = previous.fast;
        self.primed = previous.primed;
    }

    /// Drops the cached runs and their ETag and makes the repo due, so its next poll fetches
    /// runs again. The workflow and tag lists stay cached.
    fn invalidate_runs(&mut self, now: OffsetDateTime) {
        self.cache.runs_etag = None;
        self.cache.runs.clear();
        self.primed = false;
        self.next_due = now;
    }

    /// A run that is still moving keeps the repo on fast polling for an hour.
    pub fn wants_fast_poll(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        self.state
            .groups
            .iter()
            .filter(|run| {
                matches!(run.state, RunState::Running | RunState::Queued)
                    && now - run.updated_at < FAST_POLL_WINDOW
            })
            .map(|run| run.updated_at)
            .max()
    }
}

/// Outcome of polling one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollResult {
    pub state: RepoState,
    /// Counted requests of steady polling.
    pub counted: u32,
    /// Counted requests that filled an empty cache: a first fetch, a workflow or tag list.
    pub filled: u32,
    pub rate_limit: Option<crate::http::RateLimit>,
    /// An error the account has to act on: 401, rate limit, redirect on account calls.
    pub account_error: Option<ProviderError>,
    pub network_failure: bool,
}

/// Fetches one repo and turns the response, or the error, into a repo state. The repo is
/// selected with its compiled `filters` and tag choice; `settings` supplies the pull request
/// choice.
pub async fn poll_repo(
    provider: &dyn Provider,
    kind: AccountKind,
    runtime: &mut RepoRuntime,
    settings: &Settings,
    now: OffsetDateTime,
    in_grace: bool,
    refresh_groups: bool,
) -> PollResult {
    let filters = runtime.filters.clone();
    let filter = match filters.branch.as_ref() {
        Ok(f) => f,
        Err(e) => {
            log::warn!(
                "{}: branch filter does not compile: {e}",
                runtime.watched.repo.full_name
            );
            return invalid_filter(runtime, notes::invalid_branch_filter(&filter_detail(e)));
        }
    };
    let workflow_filter = match filters.workflow.as_ref() {
        Ok(f) => f,
        Err(e) => {
            log::warn!(
                "{}: workflow filter does not compile: {e}",
                runtime.watched.repo.full_name
            );
            return invalid_filter(runtime, notes::invalid_workflow_filter(&filter_detail(e)));
        }
    };

    let include_tags = filters.include_tags;
    let request = FetchRequest {
        repo: &runtime.watched.repo,
        branch_filter: filter,
        exclude_pull_requests: settings.exclude_pull_requests,
        now,
        refresh_groups,
        include_tags,
    };
    let previous = runtime.state.clone();
    let stale = |mut state: RepoState| {
        state.stale = true;
        state
    };

    let result = provider.fetch(request, &mut runtime.cache).await;
    runtime.last_failed = result.is_err();
    let err = match result {
        Ok(outcome) => {
            runtime.failures = 0;
            // The runs request is the steady cost: on GitHub only when it is not a 304, on
            // GitLab always. Everything else a fetch sends fills a list cache, and a fetch into
            // an empty cache fills it entirely.
            let steady = if runtime.primed {
                match kind {
                    AccountKind::GitHub => u32::from(!outcome.not_modified),
                    AccountKind::GitLab => 1,
                }
            } else {
                0
            };
            let counted = steady.min(outcome.counted_requests);
            runtime.primed = true;
            let selection = Selection {
                default_branch: &runtime.watched.repo.default_branch,
                branch_filter: filter,
                workflow_filter,
                exclude_pull_requests: settings.exclude_pull_requests,
                include_tags,
                active_groups: outcome.active_groups.as_ref(),
                now,
            };
            let groups = select_groups(&outcome.runs, &selection);
            return PollResult {
                state: repo_state(groups, now),
                counted,
                filled: outcome.counted_requests - counted,
                rate_limit: outcome.rate_limit,
                account_error: None,
                network_failure: false,
            };
        }
        Err(err) => err,
    };

    let mut result = PollResult {
        state: previous.clone(),
        counted: 1,
        filled: 0,
        rate_limit: None,
        account_error: None,
        network_failure: false,
    };
    // A GitLab 403 means CI/CD is disabled only while the account is still a project member.
    let err = match (err, kind) {
        (ProviderError::Forbidden, AccountKind::GitLab) => {
            // A cached answer sends no request; a failed check sent one.
            match provider.membership(&runtime.watched.repo).await {
                Ok(membership) => {
                    result.counted += membership.requests;
                    if membership.member {
                        runtime.failures = 0;
                        result.state = ci_disabled_state(previous, now);
                        return result;
                    }
                    ProviderError::NotFound
                }
                Err(e) => {
                    result.counted += 1;
                    e
                }
            }
        }
        (err, _) => err,
    };
    log::info!("{} poll failed: {err:?}", runtime.watched.repo.full_name);
    match err {
        ProviderError::Unauthorized | ProviderError::RateLimited { .. } => {
            result.state = stale(previous);
            result.account_error = Some(err);
        }
        ProviderError::NotFound | ProviderError::Forbidden => {
            runtime.failures = 0;
            result.state = error_state(previous, notes::NO_ACCESS);
        }
        ProviderError::Redirected { .. } => {
            runtime.failures = 0;
            result.state = error_state(previous, notes::REDIRECTED);
        }
        ProviderError::Decode(_) => {
            runtime.failures = 0;
            result.state = error_state(previous, notes::UNEXPECTED_RESPONSE);
        }
        ProviderError::Network(_) | ProviderError::Server { .. } => {
            result.network_failure = true;
            if !in_grace {
                runtime.failures += 1;
            }
            result.state = if runtime.failures >= FAILURES_BEFORE_ERROR {
                error_state(previous, notes::UNREACHABLE)
            } else {
                stale(previous)
            };
        }
    }
    result
}

/// The result for a repo whose filter does not compile: Error with `note`, without a request.
fn invalid_filter(runtime: &mut RepoRuntime, note: String) -> PollResult {
    runtime.last_failed = true;
    let mut state = runtime.state.clone();
    state.status = RepoStatus::Error;
    state.note = Some(note);
    PollResult {
        state,
        counted: 0,
        filled: 0,
        rate_limit: None,
        account_error: None,
        network_failure: false,
    }
}

fn ci_disabled_state(mut state: RepoState, now: OffsetDateTime) -> RepoState {
    state.status = RepoStatus::None;
    state.representative = None;
    state.groups.clear();
    state.stale = false;
    state.note = Some(notes::CI_DISABLED.into());
    state.last_checked = Some(now);
    state
}

/// The offending pattern and what is wrong with it, without the parser's wording around them.
fn filter_detail(e: &globset::Error) -> String {
    match e.glob() {
        Some(glob) => format!("{glob} ({})", e.kind()),
        None => e.kind().to_string(),
    }
}

fn error_state(mut state: RepoState, note: &str) -> RepoState {
    state.status = RepoStatus::Error;
    state.stale = false;
    state.note = Some(note.to_string());
    state
}

/// One account's polling loop state. The spawned task calls `cycle` repeatedly.
pub struct AccountWorker {
    pub account: Account,
    pub provider: Arc<dyn Provider>,
    pub pool: Arc<Mutex<PoolState>>,
    pub repos: Vec<RepoRuntime>,
    pub settings: Arc<Settings>,
    /// Organization-level filter overrides of the config the worker was built from.
    pub organizations: Arc<Vec<OrgFilters>>,
    pub status: AccountSnapshot,
    pub backoff_secs: u64,
    /// Wait used when a rate limit response carries no reset or retry-after header.
    pub rate_limit_backoff_secs: u64,
    pub grace_until: Option<OffsetDateTime>,
    /// Shared pause flag. A cycle sends no further requests once it is set.
    pub paused: Arc<AtomicBool>,
    semaphore: Arc<Semaphore>,
}

/// What a fresh repo list changed in a worker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoListUpdate {
    /// The repos whose name, URL or default branch changed, as listed.
    pub changed: Vec<RepoInfo>,
    /// IDs of the changed repos that now select their runs differently.
    pub reselected: Vec<u64>,
}

/// What a cycle decided, for logging and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleReport {
    pub polled: usize,
    /// Indexes of the repos whose state the cycle may have changed: the ones it polled, or
    /// every repo when it marked them all stale or rejected.
    pub touched: Vec<usize>,
    pub skipped_reason: Option<String>,
    pub next_due: Option<OffsetDateTime>,
}

impl AccountWorker {
    /// Builds the worker with every repo due at `now` and no organization-level filters.
    pub fn new(
        account: Account,
        provider: Arc<dyn Provider>,
        pool: Arc<Mutex<PoolState>>,
        watched: Vec<WatchedRepo>,
        settings: impl Into<Arc<Settings>>,
        now: OffsetDateTime,
    ) -> AccountWorker {
        AccountWorker::with_organizations(
            account,
            provider,
            pool,
            watched,
            settings,
            Arc::default(),
            now,
        )
    }

    /// Builds the worker with every repo due at `now`. The branch and workflow filters are
    /// compiled here, once: repos that inherit their organization's patterns share one compiled
    /// set.
    pub fn with_organizations(
        account: Account,
        provider: Arc<dyn Provider>,
        pool: Arc<Mutex<PoolState>>,
        watched: Vec<WatchedRepo>,
        settings: impl Into<Arc<Settings>>,
        organizations: Arc<Vec<OrgFilters>>,
        now: OffsetDateTime,
    ) -> AccountWorker {
        let settings = settings.into();
        let mut compiler = FilterCompiler::new(&settings, &organizations, &account);
        let repos: Vec<RepoRuntime> = watched
            .into_iter()
            .map(|w| {
                let filters = compiler.for_repo(&w);
                RepoRuntime::with_filters(w, filters, now)
            })
            .collect();
        pool.lock()
            .unwrap()
            .set_repo_count(&account.id, repos.len());
        let interval = base_interval_secs(&settings);
        let status = AccountSnapshot {
            id: account.id.clone(),
            label: account.label.clone(),
            kind: account.kind,
            effective_interval_secs: interval,
            configured_interval_secs: interval,
            ..Default::default()
        };
        AccountWorker {
            account,
            provider,
            pool,
            repos,
            settings,
            organizations,
            status,
            backoff_secs: 0,
            rate_limit_backoff_secs: 0,
            grace_until: None,
            paused: Arc::new(AtomicBool::new(false)),
            semaphore: Arc::new(Semaphore::new(CONCURRENT_REQUESTS)),
        }
    }

    /// Takes over the poll state of the worker this one replaces, for the same account and
    /// provider identity.
    ///
    /// Repos watched by both keep their state, cache, schedule and failure count. A repo whose
    /// effective selection changed (branch patterns, ignored workflows or tag choice from the
    /// repo, organization or global settings, pull request exclusion or default branch) drops its
    /// cached runs and is due at once.
    /// A changed poll interval brings later due times forward to the new interval. The account's
    /// reachability, backoff and rate limit state carry over unless the token was rejected.
    pub fn adopt(&mut self, previous: AccountWorker, now: OffsetDateTime) {
        let base = base_interval_secs(&self.settings);
        let interval_changed = base_interval_secs(&previous.settings) != base;
        let mut carried: HashMap<u64, RepoRuntime> = previous
            .repos
            .into_iter()
            .map(|r| (r.watched.repo.id, r))
            .collect();
        for r in &mut self.repos {
            let Some(old) = carried.remove(&r.watched.repo.id) else {
                continue;
            };
            let before = FilterScope::new(
                &previous.settings,
                &previous.organizations,
                &previous.account,
                &old.watched,
            );
            let after = FilterScope::new(
                &self.settings,
                &self.organizations,
                &self.account,
                &r.watched,
            );
            let reselected = selection_changed(before, after)
                || old.watched.repo.default_branch != r.watched.repo.default_branch;
            r.carry_over(old);
            if reselected {
                r.invalidate_runs(now);
            } else if interval_changed && !r.fast {
                r.next_due = r.next_due.min(due_after(now, jitter(base)));
            }
        }

        if !previous.status.auth_error {
            self.status.unreachable = previous.status.unreachable;
            self.status.error = previous.status.error;
            self.status.rate_limited_until = previous.status.rate_limited_until;
            if !interval_changed {
                self.status.effective_interval_secs = previous.status.effective_interval_secs;
            }
            self.backoff_secs = previous.backoff_secs;
        }
        self.rate_limit_backoff_secs = previous.rate_limit_backoff_secs;
        self.grace_until = previous.grace_until;

        // The replaced worker gave its fast slots back; claim them again, newest runs first.
        let mut fast: Vec<(usize, OffsetDateTime)> = self
            .repos
            .iter()
            .enumerate()
            .filter(|(_, r)| r.fast)
            .map(|(i, r)| (i, r.wants_fast_poll(now).unwrap_or(now)))
            .collect();
        fast.sort_by_key(|a| std::cmp::Reverse(a.1));
        let granted = self
            .pool
            .lock()
            .unwrap()
            .claim_fast_slots(&self.account.id, fast.len());
        for &(i, _) in fast.iter().skip(granted) {
            self.repos[i].fast = false;
        }
    }

    /// Starts the post-wake grace period.
    pub fn mark_wake(&mut self, now: OffsetDateTime) {
        self.grace_until = Some(wait_until(now, WAKE_GRACE_SECS.unsigned_abs()));
        self.backoff_secs = 0;
        for r in &mut self.repos {
            r.failures = 0;
        }
    }

    /// Updates names, URLs and default branches from a fresh repo list. A repo that moved to
    /// another owner resolves its filters from the new owner's organization; when that or a new
    /// default branch changes its selection, it drops its cached runs and is due at `now`.
    pub fn apply_repo_list(&mut self, listed: &[RepoInfo], now: OffsetDateTime) -> RepoListUpdate {
        let mut update = RepoListUpdate::default();
        let mut compiler = FilterCompiler::new(&self.settings, &self.organizations, &self.account);
        for r in &mut self.repos {
            let Some(fresh) = listed.iter().find(|l| l.id == r.watched.repo.id) else {
                continue;
            };
            if &r.watched.repo == fresh {
                continue;
            }
            let previous = r.watched.clone();
            r.watched.repo = fresh.clone();
            update.changed.push(fresh.clone());
            let before = FilterScope::new(
                &self.settings,
                &self.organizations,
                &self.account,
                &previous,
            );
            let after = FilterScope::new(
                &self.settings,
                &self.organizations,
                &self.account,
                &r.watched,
            );
            let reselected = selection_changed(before, after)
                || previous.repo.default_branch != fresh.default_branch;
            if reselected {
                r.filters = compiler.for_repo(&r.watched);
                r.invalidate_runs(now);
                update.reselected.push(fresh.id);
            }
        }
        update
    }

    pub fn repo_snapshot(&self, index: usize) -> RepoSnapshot {
        let r = &self.repos[index];
        RepoSnapshot {
            account_id: self.account.id.clone(),
            repo: r.watched.repo.clone(),
            state: r.state.clone(),
        }
    }

    pub fn repo_snapshots(&self) -> Vec<RepoSnapshot> {
        (0..self.repos.len())
            .map(|i| self.repo_snapshot(i))
            .collect()
    }

    /// Polls every repo that is due. `force` makes every repo due, as after Refresh now or a
    /// wake. `refresh_groups` fetches GitHub workflow lists again, as on Refresh now.
    ///
    /// A 401, a rate limit, a pool pause or the pause flag stops the cycle from sending further
    /// requests. Repos it skips keep their state and stay due.
    pub async fn cycle(
        &mut self,
        now: OffsetDateTime,
        force: bool,
        refresh_groups: bool,
    ) -> CycleReport {
        if self.status.auth_error {
            return CycleReport {
                polled: 0,
                touched: Vec::new(),
                skipped_reason: Some("auth error".into()),
                next_due: None,
            };
        }
        let pool_paused_until = {
            let pool = self.pool.lock().unwrap();
            pool.paused_until.filter(|_| pool.is_paused(now))
        };
        if let Some(until) = pool_paused_until {
            self.status.rate_limited_until = Some(until.unix_timestamp());
            for r in &mut self.repos {
                r.state.stale = true;
            }
            return CycleReport {
                polled: 0,
                touched: (0..self.repos.len()).collect(),
                skipped_reason: Some("rate limited".into()),
                next_due: Some(until),
            };
        }
        self.status.rate_limited_until = None;
        if let Some(until) = self.grace_until {
            if until <= now {
                self.grace_until = None;
            }
        }
        let in_grace = self.grace_until.is_some();

        let due: Vec<usize> = self
            .repos
            .iter()
            .enumerate()
            .filter(|(_, r)| force || r.next_due <= now)
            .map(|(i, _)| i)
            .collect();

        // Each due repo's runtime moves into its poll task, leaving a stand-in behind, and
        // comes back with the result. Results are applied by repo index.
        let stop = StopSignals {
            cancel: Arc::new(AtomicBool::new(false)),
            paused: self.paused.clone(),
            pool: self.pool.clone(),
        };
        let mut tasks = JoinSet::new();
        let mut task_repos = HashMap::new();
        for &index in &due {
            if stop.is_set(now) {
                break;
            }
            let permit = self
                .semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("the semaphore is never closed");
            let stop = stop.clone();
            let provider = self.provider.clone();
            let kind = self.account.kind;
            let settings = Arc::clone(&self.settings);
            let class = if self.repos[index].fast {
                RequestClass::Fast
            } else {
                RequestClass::Base
            };
            let stand_in = self.repos[index].stand_in();
            let mut runtime = std::mem::replace(&mut self.repos[index], stand_in);
            let handle = tasks.spawn(async move {
                let _permit = permit;
                if stop.is_set(now) {
                    return (runtime, None);
                }
                let result = poll_repo(
                    &*provider,
                    kind,
                    &mut runtime,
                    &settings,
                    now,
                    in_grace,
                    refresh_groups,
                )
                .await;
                stop.observe(&result, class, now);
                (runtime, Some(result))
            });
            task_repos.insert(handle.id(), index);
        }

        let mut polled = Vec::with_capacity(due.len());
        let mut network_failures = 0usize;
        let mut account_error: Option<ProviderError> = None;
        while let Some(joined) = tasks.join_next_with_id().await {
            match joined {
                Ok((id, (runtime, result))) => {
                    let index = task_repos[&id];
                    self.repos[index] = runtime;
                    let Some(result) = result else {
                        continue;
                    };
                    self.repos[index].state = result.state;
                    polled.push(index);
                    if result.network_failure {
                        network_failures += 1;
                    }
                    if let Some(err) = result.account_error {
                        // A 401 outranks a rate limit seen in the same cycle.
                        if account_error.is_none() || err == ProviderError::Unauthorized {
                            account_error = Some(err);
                        }
                    }
                }
                Err(e) => {
                    let Some(&index) = task_repos.get(&e.id()) else {
                        continue;
                    };
                    // The stand-in keeps the repo's identity; its cache and runs are lost.
                    let repo = &mut self.repos[index];
                    log::error!("poll of {} failed: {e}", repo.watched.repo.full_name);
                    repo.state = error_state(repo.state.clone(), notes::INTERNAL_ERROR);
                    repo.last_failed = true;
                    polled.push(index);
                }
            }
        }
        polled.sort_unstable();
        // Responses were recorded as they arrived; this prunes the window on quiet cycles.
        self.pool
            .lock()
            .unwrap()
            .record(now, RequestClass::Base, 0, None);

        match account_error {
            Some(ProviderError::Unauthorized) => {
                self.status.auth_error = true;
                self.status.auth_reason = Some(AuthReason::Rejected);
                self.status.error = Some(notes::TOKEN_REJECTED.into());
                for r in &mut self.repos {
                    r.state = error_state(r.state.clone(), notes::TOKEN_REJECTED);
                    r.fast = false;
                }
                self.pool.lock().unwrap().release_account(&self.account.id);
                return CycleReport {
                    polled: polled.len(),
                    touched: (0..self.repos.len()).collect(),
                    skipped_reason: Some("auth error".into()),
                    next_due: None,
                };
            }
            Some(ProviderError::RateLimited {
                retry_after,
                reset_at,
                remaining,
            }) => {
                let wait = self.rate_limit_wait(now, retry_after, reset_at, remaining);
                let mut pool = self.pool.lock().unwrap();
                let until = pool.paused_until.map_or(wait, |current| current.max(wait));
                pool.paused_until = Some(until);
            }
            _ => {
                self.rate_limit_backoff_secs = 0;
            }
        }
        let pool_paused_until = {
            let pool = self.pool.lock().unwrap();
            pool.paused_until.filter(|_| pool.is_paused(now))
        };
        if let Some(until) = pool_paused_until {
            self.status.rate_limited_until = Some(until.unix_timestamp());
        }

        // A cycle that polled nothing says nothing about reachability.
        if !polled.is_empty() {
            let all_failed = network_failures == polled.len();
            self.status.unreachable = all_failed;
            if all_failed {
                self.backoff_secs = if self.backoff_secs == 0 {
                    base_interval_secs(&self.settings)
                } else {
                    (self.backoff_secs * 2).min(BACKOFF_MAX_SECS)
                };
                self.status.error = Some(notes::UNREACHABLE.into());
            } else if network_failures == 0 {
                self.backoff_secs = 0;
                self.status.error = None;
            }
        }

        let next_due = self.schedule(now, &polled);
        CycleReport {
            polled: polled.len(),
            touched: polled,
            skipped_reason: None,
            next_due,
        }
    }

    /// Sets `next_due` on the repos just polled and on repos whose fast flag changed, then
    /// returns the earliest due time. Repos that were not polled keep their schedule. Due times
    /// fall on `DUE_BUCKET_SECS` boundaries.
    fn schedule(&mut self, now: OffsetDateTime, polled: &[usize]) -> Option<OffsetDateTime> {
        let configured = base_interval_secs(&self.settings);
        let base = {
            let mut pool = self.pool.lock().unwrap();
            pool.effective_interval(configured, now)
        };
        self.status.effective_interval_secs = base;
        self.status.configured_interval_secs = configured;

        // Fast polling is for healthy repos only: not while the account is unreachable or
        // backing off, and not for a repo whose last poll failed.
        let account_failing = self.status.unreachable || self.backoff_secs > 0;
        let mut fast_candidates: Vec<(usize, OffsetDateTime)> = if account_failing {
            Vec::new()
        } else {
            self.repos
                .iter()
                .enumerate()
                .filter(|(_, r)| !r.last_failed)
                .filter_map(|(i, r)| r.wants_fast_poll(now).map(|at| (i, at)))
                .collect()
        };
        fast_candidates.sort_by_key(|a| std::cmp::Reverse(a.1));
        let granted = self
            .pool
            .lock()
            .unwrap()
            .claim_fast_slots(&self.account.id, fast_candidates.len());
        let fast: std::collections::HashSet<usize> = fast_candidates
            .iter()
            .take(granted)
            .map(|(i, _)| *i)
            .collect();

        let interval_secs = if self.status.unreachable {
            self.backoff_secs.max(base)
        } else {
            base
        };
        for (i, r) in self.repos.iter_mut().enumerate() {
            let was_fast = r.fast;
            r.fast = fast.contains(&i);
            if !polled.contains(&i) && was_fast == r.fast {
                continue;
            }
            let secs = if r.fast {
                FAST_POLL_SECS
            } else {
                repo_backoff_secs(interval_secs, r.failures)
            };
            r.next_due = due_after(now, jitter(secs));
        }
        self.repos.iter().map(|r| r.next_due).min()
    }

    /// When a rate limited pool may poll again. `retry-after` wins, kept between
    /// `RETRY_AFTER_MIN_SECS` and `RETRY_AFTER_MAX_SECS`. The reset time applies only when the
    /// primary window is exhausted, and at most an hour ahead; otherwise, as for a secondary
    /// limit, the wait starts at 60 seconds and doubles on each repeat.
    fn rate_limit_wait(
        &mut self,
        now: OffsetDateTime,
        retry_after: Option<u64>,
        reset_at: Option<i64>,
        remaining: Option<u64>,
    ) -> OffsetDateTime {
        if let Some(secs) = retry_after {
            return wait_until(now, secs.clamp(RETRY_AFTER_MIN_SECS, RETRY_AFTER_MAX_SECS));
        }
        if let (Some(0), Some(reset_at)) = (remaining, reset_at) {
            return clamp_reset(now, reset_at);
        }
        let secs = self.rate_limit_backoff_secs.max(60);
        self.rate_limit_backoff_secs = (secs * 2).min(BACKOFF_MAX_SECS);
        wait_until(now, secs)
    }
}

/// Interval for a repo after `failures` network or server failures in a row: the base
/// interval, doubled per further failure, capped at `BACKOFF_MAX_SECS` but never below base.
pub fn repo_backoff_secs(base_secs: u64, failures: u32) -> u64 {
    if failures <= 1 {
        return base_secs;
    }
    let factor = 1u64 << (failures - 1).min(16);
    base_secs
        .saturating_mul(factor)
        .min(BACKOFF_MAX_SECS)
        .max(base_secs)
}

/// Signals that stop a cycle from sending further requests.
#[derive(Clone)]
struct StopSignals {
    /// Set by a task that got a 401 or a rate limit.
    cancel: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    pool: Arc<Mutex<PoolState>>,
}

impl StopSignals {
    fn is_set(&self, now: OffsetDateTime) -> bool {
        self.cancel.load(Ordering::SeqCst)
            || self.paused.load(Ordering::SeqCst)
            || self.pool.lock().unwrap().is_paused(now)
    }

    /// Records a response in the pool, which may pause it, and cancels the cycle after a 401
    /// or a rate limit.
    fn observe(&self, result: &PollResult, class: RequestClass, now: OffsetDateTime) {
        {
            let mut pool = self.pool.lock().unwrap();
            pool.record(now, class, result.counted, result.rate_limit);
            pool.record(now, RequestClass::Fill, result.filled, None);
        }
        if result.account_error.is_some() {
            self.cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// Adds about 10 percent of random spread so requests do not align.
pub fn jitter(secs: u64) -> u64 {
    let spread = (secs as f64 * JITTER).max(1.0);
    let delta: f64 = rand::random_range(-spread..=spread);
    ((secs as f64) + delta).max(1.0) as u64
}
