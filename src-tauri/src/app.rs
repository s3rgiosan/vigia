//! Application runtime: builds providers from config and secrets, runs one task per account,
//! and publishes snapshots to the tray and the frontend.
//!
//! Lock order: `tasks`, `publish_state`, `update`, `config`, `applied_config`, `notifier`, `pools`, a pool,
//! a parked worker slot, then the snapshot store's maps. A lock is only taken while holding
//! locks that come before it, and the publisher and notification sink run with no lock held.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use time::OffsetDateTime;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::config::{Account, AccountKind, Config, ConfigError, ConfigStore};
use crate::notify::{filter_transitions, render, Notification, Notifier};
use crate::poller::{
    base_interval_secs, is_wake_gap, AccountWorker, Controls, Snapshot, SnapshotFlags,
    SnapshotStore, HEARTBEAT_SECS,
};
use crate::pool::{PoolKind, PoolState, RequestClass};
use crate::providers::github::GitHubProvider;
use crate::providers::gitlab::GitLabProvider;
use crate::providers::{Provider, ProviderError, RepoInfo};
use crate::secrets::Secrets;
use crate::updates::UpdateInfo;

pub const SNAPSHOT_EVENT: &str = "snapshot-updated";

/// Worker-driven publishes are at most this far apart, across all accounts.
pub const PUBLISH_INTERVAL: StdDuration = StdDuration::from_secs(1);

/// Called with every new snapshot: updates the tray and emits the event.
pub type Publisher = Arc<dyn Fn(&Snapshot) + Send + Sync>;
/// Called with every notification to show.
pub type NotificationSink = Arc<dyn Fn(&Notification) + Send + Sync>;
/// Builds the provider for an account from its token.
pub type ProviderFactory =
    Arc<dyn Fn(&Account, &str) -> Result<Arc<dyn Provider>, ProviderError> + Send + Sync>;
/// Reads the wall clock.
pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;
/// Receives every repo list an account worker fetches, with the account it belongs to.
pub type RepoListSink = Arc<dyn Fn(&Account, &[RepoInfo]) + Send + Sync>;

/// Holds an account's worker while its task waits between cycles, so a restart can take it.
type Parked = Arc<Mutex<Option<AccountWorker>>>;

/// What a worker's poll state depends on: a restart carries the state over only while this
/// stays the same.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderIdentity {
    kind: AccountKind,
    base_url: Option<String>,
    /// A hash of the token, so a replaced token starts fresh without the token being kept.
    token: u64,
}

impl ProviderIdentity {
    fn new(account: &Account, token: &str) -> ProviderIdentity {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        ProviderIdentity {
            kind: account.kind,
            base_url: account.base_url.clone(),
            token: hasher.finish(),
        }
    }
}

struct AccountTask {
    /// Resolves to the worker when the task stops because a restart replaced it.
    handle: JoinHandle<Option<AccountWorker>>,
    parked: Parked,
    identity: ProviderIdentity,
}

/// What the last published snapshot was built from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishKey {
    version: u64,
    paused: bool,
    flags: SnapshotFlags,
}

#[derive(Default)]
struct PublishState {
    last: Option<PublishKey>,
    last_at: Option<tokio::time::Instant>,
    /// A delayed publish is already scheduled and will pick up later changes.
    scheduled: bool,
}

pub struct Runtime {
    /// Read through `with_config` and written through `update_config` or `with_config_mut`.
    config: Mutex<ConfigStore>,
    pub secrets: Arc<Secrets>,
    pub store: Arc<SnapshotStore>,
    pub controls: Controls,
    /// Reached through `with_notifier`.
    notifier: Mutex<Notifier>,
    publisher: Publisher,
    notification_sink: NotificationSink,
    provider_factory: ProviderFactory,
    repo_list_sink: Option<RepoListSink>,
    clock: Clock,
    /// Where account tasks and the heartbeat run. `None` spawns them on Tauri's runtime.
    spawn_handle: Option<Handle>,
    pools: Mutex<HashMap<String, Arc<Mutex<PoolState>>>>,
    tasks: Mutex<HashMap<String, AccountTask>>,
    refresh_generation: Arc<AtomicU64>,
    wake_generation: Arc<AtomicU64>,
    heartbeat: Mutex<Option<JoinHandle<()>>>,
    /// Bumped by every restart. A worker started under an older value no longer publishes or
    /// notifies.
    restart_generation: Arc<AtomicU64>,
    /// The config the running workers were started with. `None` until the first restart.
    applied_config: Mutex<Option<Config>>,
    publish_state: Mutex<PublishState>,
    /// The newer release the last update check found, carried on every snapshot.
    update: Mutex<Option<UpdateInfo>>,
}

impl Runtime {
    pub fn new(
        config: ConfigStore,
        secrets: Arc<Secrets>,
        publisher: Publisher,
        notification_sink: NotificationSink,
    ) -> Runtime {
        Runtime {
            config: Mutex::new(config),
            secrets,
            store: Arc::new(SnapshotStore::default()),
            controls: Controls::default(),
            notifier: Mutex::new(Notifier::default()),
            publisher,
            notification_sink,
            provider_factory: Arc::new(build_provider),
            repo_list_sink: None,
            clock: Arc::new(OffsetDateTime::now_utc),
            spawn_handle: None,
            pools: Mutex::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
            refresh_generation: Arc::new(AtomicU64::new(0)),
            wake_generation: Arc::new(AtomicU64::new(0)),
            heartbeat: Mutex::new(None),
            restart_generation: Arc::new(AtomicU64::new(0)),
            applied_config: Mutex::new(None),
            publish_state: Mutex::new(PublishState::default()),
            update: Mutex::new(None),
        }
    }

    /// Builds account providers with `factory`.
    #[doc(hidden)]
    pub fn with_provider_factory(mut self, factory: ProviderFactory) -> Runtime {
        self.provider_factory = factory;
        self
    }

    /// Reads the wall clock from `clock`.
    #[doc(hidden)]
    pub fn with_clock(mut self, clock: Clock) -> Runtime {
        self.clock = clock;
        self
    }

    /// Runs account tasks, the heartbeat and delayed publishes on `handle`.
    #[doc(hidden)]
    pub fn with_spawn_handle(mut self, handle: Handle) -> Runtime {
        self.spawn_handle = Some(handle);
        self
    }

    /// Hands every repo list a worker fetches to `sink`, such as a cache the settings window
    /// reads.
    pub fn with_repo_list_sink(mut self, sink: RepoListSink) -> Runtime {
        self.repo_list_sink = Some(sink);
        self
    }

    pub fn now(&self) -> OffsetDateTime {
        (self.clock)()
    }

    /// Runs `f` with the config store locked.
    pub fn with_config<R>(&self, f: impl FnOnce(&ConfigStore) -> R) -> R {
        f(&self.config.lock().unwrap())
    }

    /// Runs `f` with the config store locked for writing, to check and update in one step.
    pub fn with_config_mut<R>(&self, f: impl FnOnce(&mut ConfigStore) -> R) -> R {
        f(&mut self.config.lock().unwrap())
    }

    /// Applies a change to the config and writes the file. Nothing changes when the write fails.
    pub fn update_config(&self, change: impl FnOnce(&mut Config)) -> Result<(), ConfigError> {
        self.config.lock().unwrap().update(change)
    }

    /// Runs `f` with the notification memory locked.
    pub fn with_notifier<R>(&self, f: impl FnOnce(&mut Notifier) -> R) -> R {
        f(&mut self.notifier.lock().unwrap())
    }

    fn spawn<T: Send + 'static>(
        &self,
        task: impl Future<Output = T> + Send + 'static,
    ) -> JoinHandle<T> {
        match &self.spawn_handle {
            Some(handle) => handle.spawn(task),
            None => tauri::async_runtime::spawn(task).inner_handle(),
        }
    }

    /// Ticks every few seconds and treats a long wall-clock gap as a wake from sleep.
    pub fn start_heartbeat(self: &Arc<Self>) {
        let mut slot = self.heartbeat.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let runtime = Arc::clone(self);
        let handle = self.spawn(async move {
            let mut last = runtime.now();
            loop {
                tokio::time::sleep(StdDuration::from_secs(HEARTBEAT_SECS)).await;
                let now = runtime.now();
                if is_wake_gap(now - last) {
                    log::info!("wake detected after a gap of {}", now - last);
                    runtime.wake_generation.fetch_add(1, Ordering::SeqCst);
                    runtime.controls.wake.notify_waiters();
                }
                last = now;
            }
        });
        *slot = Some(handle);
    }

    pub fn snapshot(&self) -> Snapshot {
        let update = self.update.lock().unwrap().clone();
        let flags = self.with_config(|config| SnapshotFlags {
            secrets_blocked: self.secrets.is_blocked(),
            config_read_only: config.read_only(),
            config_error: config.load_error().map(str::to_string),
            update,
        });
        self.store
            .snapshot(self.now(), self.controls.is_paused(), flags)
    }

    /// Publishes the current snapshot at once, unless nothing in it changed since the last
    /// publish. The generation time alone is not a change.
    pub fn publish(&self) {
        let snapshot = {
            let mut state = self.publish_state.lock().unwrap();
            // Read before the snapshot is built, so a change landing in between is published
            // again rather than lost.
            let version = self.store.version();
            let snapshot = self.snapshot();
            let key = PublishKey {
                version,
                paused: snapshot.paused,
                flags: SnapshotFlags {
                    secrets_blocked: snapshot.secrets_blocked,
                    config_read_only: snapshot.config_read_only,
                    config_error: snapshot.config_error.clone(),
                    update: snapshot.update.clone(),
                },
            };
            if state.last.as_ref() == Some(&key) {
                return;
            }
            state.last = Some(key);
            state.last_at = Some(tokio::time::Instant::now());
            snapshot
        };
        (self.publisher)(&snapshot);
    }

    /// Publishes like `publish`, at most once per `PUBLISH_INTERVAL`. A change within the
    /// interval is published when it ends.
    fn publish_soon(self: &Arc<Self>) {
        let delay = {
            let mut state = self.publish_state.lock().unwrap();
            if state.scheduled {
                return;
            }
            let now = tokio::time::Instant::now();
            match state.last_at {
                Some(at) if now < at + PUBLISH_INTERVAL => {
                    state.scheduled = true;
                    at + PUBLISH_INTERVAL - now
                }
                _ => StdDuration::ZERO,
            }
        };
        if delay.is_zero() {
            self.publish();
            return;
        }
        let runtime = Arc::clone(self);
        self.spawn(async move {
            tokio::time::sleep(delay).await;
            runtime.publish_state.lock().unwrap().scheduled = false;
            runtime.publish();
        });
    }

    /// Records the update the last check found and publishes when it differs from the one
    /// before.
    pub fn set_update(&self, update: Option<UpdateInfo>) {
        *self.update.lock().unwrap() = update;
        self.publish();
    }

    pub fn set_paused(&self, paused: bool) {
        self.controls.set_paused(paused);
        self.publish();
    }

    pub fn refresh_now(&self) {
        if self.controls.is_paused() {
            return;
        }
        self.refresh_generation.fetch_add(1, Ordering::SeqCst);
        self.controls.refresh_now();
    }

    /// Builds the provider for one account from its stored token.
    pub fn provider_for(&self, account: &Account) -> Result<Arc<dyn Provider>, ProviderError> {
        self.provider_and_identity(account).map(|(p, _)| p)
    }

    fn provider_and_identity(
        &self,
        account: &Account,
    ) -> Result<(Arc<dyn Provider>, ProviderIdentity), ProviderError> {
        let token = self
            .secrets
            .token(&account.id)
            .ok_or(ProviderError::Unauthorized)?;
        let provider = (self.provider_factory)(account, &token)?;
        Ok((provider, ProviderIdentity::new(account, &token)))
    }

    /// The rate limit pool the account polls under, created on first use.
    pub fn pool_for(&self, account: &Account) -> Arc<Mutex<PoolState>> {
        let key = pool_key(account);
        let kind = match account.kind {
            AccountKind::GitHub => PoolKind::GitHub,
            AccountKind::GitLab => PoolKind::GitLab,
        };
        self.pools
            .lock()
            .unwrap()
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(PoolState::new(kind))))
            .clone()
    }

    /// Replaces every account task with one per configured account.
    ///
    /// The `tasks` lock is held for the whole restart so two concurrent restarts cannot leak a
    /// set of workers. Pools survive a restart, so rate limit state and observed counts persist.
    /// Notification memory survives too, except where `Notifier::apply_config` drops it.
    ///
    /// An account whose kind, base URL and token are unchanged keeps its poll state: the new
    /// worker adopts the old one's repo states, caches and schedule (see `AccountWorker::adopt`)
    /// and the popup keeps showing them. A worker parked between cycles is taken over at once;
    /// one in the middle of a cycle finishes it and hands itself to its successor, which shows
    /// the last published states meanwhile.
    pub fn restart(self: &Arc<Self>) {
        let mut tasks = self.tasks.lock().unwrap();
        let config = self.with_config(|c| c.config().clone());
        let previous = self.applied_config.lock().unwrap().replace(config.clone());
        let generation = self.with_notifier(|notifier| {
            // Bumped under the notifier lock, which a worker holds while it checks its
            // generation and stores its results, so a stopped worker cannot write past here.
            notifier.apply_config(previous.as_ref(), &config);
            self.restart_generation.fetch_add(1, Ordering::SeqCst) + 1
        });

        let keep: Vec<(String, u64)> = config
            .repos
            .iter()
            .map(|r| (r.account_id.clone(), r.repo.id))
            .collect();
        let account_ids: Vec<String> = config.accounts.iter().map(|a| a.id.clone()).collect();
        self.store.retain_repos(&keep, &account_ids);
        {
            let live: HashSet<String> = config.accounts.iter().map(pool_key).collect();
            let mut pools = self.pools.lock().unwrap();
            pools.retain(|k, _| live.contains(k));
            // Stopped workers give back their repo counts and fast poll slots; new workers
            // claim theirs again.
            if let Some(previous) = &previous {
                let live_accounts: HashSet<String> =
                    config.accounts.iter().map(|a| a.id.clone()).collect();
                for pool in pools.values() {
                    let mut pool = pool.lock().unwrap();
                    for account in &previous.accounts {
                        pool.release_account(&account.id);
                    }
                    pool.retain_accounts(&live_accounts);
                }
            }
        }
        let first_start = previous.is_none();

        let now = self.now();
        let settings = Arc::new(config.settings.clone());
        let organizations = Arc::new(config.organizations.clone());
        let mut replaced = std::mem::take(&mut *tasks);
        for account in &config.accounts {
            let watched: Vec<_> = config.repos_of(&account.id).cloned().collect();
            let (provider, identity) = match self.provider_and_identity(account) {
                Ok(built) => built,
                Err(e) => {
                    log::warn!("account {} has no usable token: {e}", account.label);
                    self.mark_without_token(account, &config, watched, now);
                    continue;
                }
            };
            let pool = self.pool_for(account);
            let mut worker = AccountWorker::with_organizations(
                account.clone(),
                provider,
                pool,
                watched,
                Arc::clone(&settings),
                Arc::clone(&organizations),
                now,
            );
            let mut handoff = None;
            if let Some(old) = replaced
                .remove(&account.id)
                .filter(|old| old.identity == identity)
            {
                let parked = old.parked.lock().unwrap().take();
                match parked {
                    Some(previous) => {
                        old.handle.abort();
                        worker.adopt(previous, now);
                    }
                    None => {
                        self.seed_from_store(&mut worker);
                        handoff = Some(old.handle);
                    }
                }
            }
            self.store.set_account(worker.status.clone());
            for repo in worker.repo_snapshots() {
                self.store.set_repo(repo);
            }
            let parked: Parked = Arc::new(Mutex::new(None));
            let handle = self.spawn(account_loop(
                Arc::clone(self),
                worker,
                Arc::clone(&parked),
                handoff,
                first_start,
                generation,
            ));
            tasks.insert(
                account.id.clone(),
                AccountTask {
                    handle,
                    parked,
                    identity,
                },
            );
        }
        for old in replaced.into_values() {
            old.handle.abort();
        }
        drop(tasks);
        self.publish();
    }

    /// Shows the last stored states on a worker whose predecessor has not handed itself over
    /// yet. A rejected token is not carried, so the new worker polls again.
    fn seed_from_store(&self, worker: &mut AccountWorker) {
        let account_id = worker.account.id.clone();
        if let Some(stored) = self.store.account(&account_id).filter(|s| !s.auth_error) {
            worker.status.unreachable = stored.unreachable;
            worker.status.error = stored.error;
            worker.status.rate_limited_until = stored.rate_limited_until;
        }
        for r in &mut worker.repos {
            if let Some(state) = self.store.repo_state(&account_id, r.watched.repo.id) {
                r.state = state;
            }
        }
    }

    /// Shows an account whose token is missing or unreadable as an auth error with every repo
    /// in Error. No notification is sent, and the account's auth notification memory is left
    /// untouched, so a later rejected token still notifies once.
    fn mark_without_token(
        &self,
        account: &Account,
        config: &Config,
        watched: Vec<crate::config::WatchedRepo>,
        now: OffsetDateTime,
    ) {
        let keychain_denied = self.secrets.is_blocked();
        let note = if keychain_denied {
            crate::notes::KEYCHAIN_DENIED
        } else {
            crate::notes::NO_TOKEN
        };
        let interval = base_interval_secs(&config.settings);
        let status = crate::poller::AccountSnapshot {
            id: account.id.clone(),
            label: account.label.clone(),
            kind: account.kind,
            auth_error: true,
            effective_interval_secs: interval,
            configured_interval_secs: interval,
            keychain_denied,
            error: Some(note.into()),
            ..Default::default()
        };
        let mut notifier = self.notifier.lock().unwrap();
        self.store.set_account(status);
        for w in watched {
            let mut runtime = crate::poller::RepoRuntime::new(w, now);
            runtime.state.status = crate::model::RepoStatus::Error;
            runtime.state.note = Some(note.into());
            // Records the Error, so the repo's first successful state later is a new baseline.
            notifier.observe(
                account.kind,
                &account.id,
                &runtime.watched.repo,
                &runtime.state,
            );
            self.store.set_repo(crate::poller::RepoSnapshot {
                account_id: account.id.clone(),
                repo: runtime.watched.repo,
                state: runtime.state,
            });
        }
    }
}

/// Builds a GitHub or GitLab provider for the account from its token.
pub fn build_provider(account: &Account, token: &str) -> Result<Arc<dyn Provider>, ProviderError> {
    let provider: Arc<dyn Provider> = match account.kind {
        AccountKind::GitHub => Arc::new(GitHubProvider::new(token)?),
        AccountKind::GitLab => {
            let base = account.base_url.as_deref().unwrap_or_default();
            Arc::new(GitLabProvider::new(base, token)?)
        }
    };
    Ok(provider)
}

/// Pool key: GitHub accounts of one user share a pool; each GitLab account has its own.
fn pool_key(account: &Account) -> String {
    match account.kind {
        AccountKind::GitHub => format!("github:{}", account.user_id.unwrap_or(0)),
        AccountKind::GitLab => format!("gitlab:{}", account.id),
    }
}

/// Wrapper so `tauri::async_runtime::JoinHandle` can be aborted through the tokio handle.
trait IntoTokioHandle<T> {
    fn inner_handle(self) -> JoinHandle<T>;
}

impl<T> IntoTokioHandle<T> for tauri::async_runtime::JoinHandle<T> {
    fn inner_handle(self) -> JoinHandle<T> {
        match self {
            tauri::async_runtime::JoinHandle::Tokio(h) => h,
        }
    }
}

/// Refreshes names, URLs and default branches of the worker's repos, persists changes, and
/// shows renamed repos. A repo whose selection changed with it, such as one moved to an
/// organization with other filters, starts a new notification baseline. Its requests fill
/// caches, so the pool records them as such.
///
/// Returns whether any repo now selects its runs differently.
async fn refresh_repo_list(
    runtime: &Arc<Runtime>,
    worker: &mut AccountWorker,
    generation: u64,
) -> bool {
    let pages = AtomicU32::new(0);
    let listed = worker
        .provider
        .list_repos(&|_| {
            pages.fetch_add(1, Ordering::Relaxed);
        })
        .await;
    let sent = pages.load(Ordering::Relaxed) + u32::from(listed.is_err());
    worker
        .pool
        .lock()
        .unwrap()
        .record(runtime.now(), RequestClass::Fill, sent, None);
    let listed = match listed {
        Ok(list) => list,
        Err(e) => {
            log::warn!("repo list for {} failed: {e}", worker.account.label);
            return false;
        }
    };
    if let Some(sink) = &runtime.repo_list_sink {
        sink(&worker.account, &listed);
    }
    let update = worker.apply_repo_list(&listed, runtime.now());
    let changed = update.changed;
    if changed.is_empty() {
        return false;
    }
    log::info!(
        "repo list for {}: {} repos changed",
        worker.account.label,
        changed.len()
    );
    let account_id = worker.account.id.clone();
    let apply = |c: &mut Config| {
        for r in c.repos.iter_mut().filter(|r| r.account_id == account_id) {
            if let Some(fresh) = changed.iter().find(|f| f.id == r.repo.id) {
                r.repo = fresh.clone();
            }
        }
    };
    let result = runtime.update_config(apply);
    if let Err(e) = result {
        log::warn!("could not save refreshed repo list: {e}");
    }
    // The worker already selects with the new names, so the next restart compares against them.
    if let Some(applied) = runtime.applied_config.lock().unwrap().as_mut() {
        apply(applied);
    }
    let renamed: Vec<usize> = (0..worker.repos.len())
        .filter(|&i| {
            changed
                .iter()
                .any(|c| c.id == worker.repos[i].watched.repo.id)
        })
        .collect();
    let stored = runtime.with_notifier(|notifier| {
        if runtime.restart_generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        for &repo_id in &update.reselected {
            notifier.reset_repo(&account_id, repo_id);
        }
        for &i in &renamed {
            runtime.store.set_repo(worker.repo_snapshot(i));
        }
        true
    });
    if stored {
        runtime.publish_soon();
    }
    !update.reselected.is_empty()
}

/// Stores the results of the repos a cycle `touched`, publishes them, and sends the
/// notifications they produce. Repos the cycle left alone are neither stored nor observed.
///
/// Returns false, having done nothing, when a restart replaced this worker.
fn apply_cycle(
    runtime: &Arc<Runtime>,
    worker: &AccountWorker,
    generation: u64,
    touched: &[usize],
) -> bool {
    let mut notifications = Vec::new();
    let mut transitions = Vec::new();
    {
        let mut notifier = runtime.notifier.lock().unwrap();
        let current = runtime.restart_generation.load(Ordering::SeqCst);
        if current != generation {
            return false;
        }
        runtime.store.set_account(worker.status.clone());
        for &i in touched {
            runtime.store.set_repo(worker.repo_snapshot(i));
        }
        if let Some(n) = notifier.observe_account(&worker.status) {
            notifications.push(n);
        }
        for &i in touched {
            let repo = &worker.repos[i];
            transitions.extend(notifier.observe(
                worker.account.kind,
                &worker.account.id,
                &repo.watched.repo,
                &repo.state,
            ));
        }
    }
    let wanted = filter_transitions(transitions, &worker.settings);
    notifications.extend(render(&worker.account.id, &worker.account.label, &wanted));
    runtime.publish_soon();
    for n in &notifications {
        (runtime.notification_sink)(n);
    }
    true
}

/// Stores every repo of a worker that just adopted its predecessor. Returns false when a
/// restart replaced it meanwhile.
fn store_adopted(runtime: &Arc<Runtime>, worker: &AccountWorker, generation: u64) -> bool {
    let stored = runtime.with_notifier(|_| {
        if runtime.restart_generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        runtime.store.set_account(worker.status.clone());
        for repo in worker.repo_snapshots() {
            runtime.store.set_repo(repo);
        }
        true
    });
    if stored {
        runtime.publish_soon();
    }
    stored
}

/// Polls one account until shutdown or until a restart replaces it, then returns the worker
/// for its successor to adopt. Between cycles the worker waits in `parked`, where a restart
/// takes it directly. `handoff` is the task this one replaces while that task was mid-cycle.
async fn account_loop(
    runtime: Arc<Runtime>,
    mut worker: AccountWorker,
    parked: Parked,
    handoff: Option<JoinHandle<Option<AccountWorker>>>,
    mut refresh_list: bool,
    generation: u64,
) -> Option<AccountWorker> {
    if let Some(previous) = handoff {
        if let Ok(Some(previous)) = previous.await {
            worker.adopt(previous, runtime.now());
            if !store_adopted(&runtime, &worker, generation) {
                return Some(worker);
            }
        }
    }

    let controls = runtime.controls.clone();
    let mut seen_refresh = runtime.refresh_generation.load(Ordering::SeqCst);
    let mut seen_wake = runtime.wake_generation.load(Ordering::SeqCst);
    let mut force = false;
    let mut refresh_groups = false;
    worker.paused = controls.paused.clone();

    loop {
        // Arm the wakeup before reading any state, so a notify that lands while this iteration
        // inspects flags or runs a cycle is not lost.
        let notified = controls.wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        if controls.shutdown.load(Ordering::SeqCst) {
            return None;
        }
        if runtime.restart_generation.load(Ordering::SeqCst) != generation {
            return Some(worker);
        }
        if controls.is_paused() {
            *parked.lock().unwrap() = Some(worker);
            notified.await;
            worker = parked.lock().unwrap().take()?;
            // Resuming polls everything at once.
            force = true;
            continue;
        }
        let refresh = runtime.refresh_generation.load(Ordering::SeqCst);
        if refresh != seen_refresh {
            seen_refresh = refresh;
            force = true;
            refresh_groups = true;
        }
        let wake = runtime.wake_generation.load(Ordering::SeqCst);
        if wake != seen_wake {
            seen_wake = wake;
            worker.mark_wake(runtime.now());
            force = true;
        }

        let now = runtime.now();
        let report = worker.cycle(now, force, refresh_groups).await;
        force = false;
        refresh_groups = false;
        if !apply_cycle(&runtime, &worker, generation, &report.touched) {
            return Some(worker);
        }
        log::debug!(
            "account {}: polled {} ({:?}), next {:?}",
            worker.account.label,
            report.polled,
            report.skipped_reason,
            report.next_due
        );
        // At the first start the repo list is refreshed after the first poll, so the popup
        // fills before the list's pages are loaded.
        let mut next_due = report.next_due;
        if refresh_list {
            refresh_list = false;
            let reselected = refresh_repo_list(&runtime, &mut worker, generation).await;
            // Repos whose selection changed with the list are due at once.
            if reselected {
                next_due = next_due.map(|due| due.min(runtime.now()));
            }
        }

        let wait = next_due.map(|due| {
            (due - runtime.now())
                .max(time::Duration::seconds(1))
                .unsigned_abs()
        });
        *parked.lock().unwrap() = Some(worker);
        tokio::select! {
            _ = async {
                match wait {
                    Some(wait) => tokio::time::sleep(wait).await,
                    // Auth error: wait for an external wake such as a token change.
                    None => std::future::pending().await,
                }
            } => {}
            _ = &mut notified => {}
        }
        worker = parked.lock().unwrap().take()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Settings, WatchedRepo};
    use crate::pool::PoolState;
    use crate::providers::{AccountIdentity, FetchOutcome, FetchRequest, RepoCache};
    use crate::secrets::MemoryStore;

    struct NullProvider;

    #[async_trait::async_trait]
    impl Provider for NullProvider {
        async fn validate(&self) -> Result<AccountIdentity, ProviderError> {
            Err(ProviderError::Unauthorized)
        }

        async fn list_repos(
            &self,
            _on_page: &(dyn Fn(usize) + Send + Sync),
        ) -> Result<Vec<RepoInfo>, ProviderError> {
            Ok(Vec::new())
        }

        async fn fetch(
            &self,
            _request: FetchRequest<'_>,
            _cache: &mut RepoCache,
        ) -> Result<FetchOutcome, ProviderError> {
            Err(ProviderError::Unauthorized)
        }
    }

    fn counting_runtime(published: &Arc<AtomicU64>) -> Arc<Runtime> {
        let count = Arc::clone(published);
        Arc::new(
            Runtime::new(
                ConfigStore::in_memory(Config::default()),
                Arc::new(Secrets::new(Box::new(MemoryStore::default()))),
                Arc::new(move |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                }),
                Arc::new(|_| {}),
            )
            .with_spawn_handle(Handle::current()),
        )
    }

    fn watched(account: &Account, id: u64) -> WatchedRepo {
        WatchedRepo {
            account_id: account.id.clone(),
            repo: RepoInfo {
                id,
                full_name: format!("acme/r{id}"),
                web_url: format!("https://example.test/acme/r{id}"),
                default_branch: "main".into(),
            },
            branch_patterns: None,
            ignored_workflows: None,
            include_tags: None,
        }
    }

    fn worker(runtime: &Runtime, account: &Account, repos: &[u64]) -> AccountWorker {
        AccountWorker::new(
            account.clone(),
            Arc::new(NullProvider),
            Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub))),
            repos.iter().map(|&id| watched(account, id)).collect(),
            Settings::default(),
            runtime.now(),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn a_worker_from_before_a_restart_stores_and_publishes_nothing() {
        let published = Arc::new(AtomicU64::new(0));
        let runtime = counting_runtime(&published);
        let account = Account::new(AccountKind::GitHub, "a", None);
        let worker = worker(&runtime, &account, &[1]);
        // With no accounts a restart spawns nothing and moves to generation 1.
        runtime.restart();
        let after_restart = published.load(Ordering::SeqCst);

        assert!(!apply_cycle(&runtime, &worker, 0, &[0]));
        assert!(runtime.snapshot().accounts.is_empty());
        tokio::time::sleep(PUBLISH_INTERVAL * 2).await;
        assert_eq!(published.load(Ordering::SeqCst), after_restart);

        assert!(apply_cycle(&runtime, &worker, 1, &[0]));
        assert_eq!(runtime.snapshot().repos.len(), 1);
        tokio::time::sleep(PUBLISH_INTERVAL * 2).await;
        assert_eq!(published.load(Ordering::SeqCst), after_restart + 1);
    }

    #[tokio::test(start_paused = true)]
    async fn apply_cycle_stores_and_observes_only_the_touched_repos() {
        let published = Arc::new(AtomicU64::new(0));
        let runtime = counting_runtime(&published);
        let account = Account::new(AccountKind::GitHub, "a", None);
        let mut worker = worker(&runtime, &account, &[1, 2]);
        runtime.restart();
        let run = crate::model::Run {
            id: 9,
            attempt: 1,
            state: crate::model::RunState::Success,
            branch: "main".into(),
            group: "ci".into(),
            name: "CI".into(),
            group_name: "CI".into(),
            url: "https://example.test/acme/runs/9".into(),
            updated_at: runtime.now(),
            pull_request: false,
            fork: false,
            tag: false,
        };
        for r in &mut worker.repos {
            r.state.status = crate::model::RepoStatus::Success;
            r.state.groups = vec![run.clone()];
            r.state.last_checked = Some(runtime.now());
        }

        assert!(apply_cycle(&runtime, &worker, 1, &[1]));
        let snapshot = runtime.snapshot();
        let stored: Vec<u64> = snapshot.repos.iter().map(|r| r.repo.id).collect();
        assert_eq!(stored, vec![2]);
        let remembered = runtime.with_notifier(|n| {
            (
                n.remembered_groups(&account.id, 1),
                n.remembered_groups(&account.id, 2),
            )
        });
        assert_eq!(remembered, (0, 1));
    }

    #[tokio::test(start_paused = true)]
    async fn publishing_skips_unchanged_snapshots_and_throttles_worker_updates() {
        let published = Arc::new(AtomicU64::new(0));
        let runtime = counting_runtime(&published);
        let account = Account::new(AccountKind::GitHub, "a", None);
        let mut worker = worker(&runtime, &account, &[1]);
        runtime.publish();
        runtime.publish();
        assert_eq!(published.load(Ordering::SeqCst), 1);

        // Three cycles within one interval produce one publish, at the end of the interval.
        for status in [
            crate::model::RepoStatus::Running,
            crate::model::RepoStatus::Failed,
            crate::model::RepoStatus::Success,
        ] {
            worker.repos[0].state.status = status;
            assert!(apply_cycle(&runtime, &worker, 0, &[0]));
        }
        assert_eq!(published.load(Ordering::SeqCst), 1);
        tokio::time::sleep(PUBLISH_INTERVAL + StdDuration::from_millis(10)).await;
        assert_eq!(published.load(Ordering::SeqCst), 2);
        assert_eq!(
            runtime.snapshot().repos[0].state.status,
            crate::model::RepoStatus::Success
        );

        // A cycle that changes nothing publishes nothing, however long after.
        tokio::time::sleep(PUBLISH_INTERVAL * 5).await;
        assert!(apply_cycle(&runtime, &worker, 0, &[0]));
        tokio::time::sleep(PUBLISH_INTERVAL * 2).await;
        assert_eq!(published.load(Ordering::SeqCst), 2);

        // Past the interval a change is published at once.
        worker.repos[0].state.status = crate::model::RepoStatus::Failed;
        assert!(apply_cycle(&runtime, &worker, 0, &[0]));
        assert_eq!(published.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_found_update_is_published_once_and_carried_on_the_snapshot() {
        let published = Arc::new(AtomicU64::new(0));
        let runtime = counting_runtime(&published);
        runtime.publish();
        assert_eq!(published.load(Ordering::SeqCst), 1);

        let update = UpdateInfo {
            version: "9.9.9".into(),
            notes: Some("Example notes".into()),
        };
        runtime.set_update(Some(update.clone()));
        runtime.set_update(Some(update.clone()));
        assert_eq!(published.load(Ordering::SeqCst), 2);
        assert_eq!(runtime.snapshot().update, Some(update));

        runtime.set_update(None);
        runtime.set_update(None);
        assert_eq!(published.load(Ordering::SeqCst), 3);
        assert_eq!(runtime.snapshot().update, None);
    }

    #[test]
    fn pool_keys_follow_the_github_user_and_the_gitlab_account() {
        let mut github = Account::new(AccountKind::GitHub, "a", None);
        assert_eq!(pool_key(&github), "github:0");
        github.user_id = Some(42);
        assert_eq!(pool_key(&github), "github:42");
        let gitlab = Account::new(AccountKind::GitLab, "b", Some("https://gl.test".into()));
        assert_eq!(pool_key(&gitlab), format!("gitlab:{}", gitlab.id));
    }
}
