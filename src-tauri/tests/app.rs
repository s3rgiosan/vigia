//! Runtime orchestration with scripted providers, an injectable clock and paused tokio time.

mod common;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};
use tokio::runtime::Handle;
use vigia_lib::app::{build_provider, Clock, Runtime, PUBLISH_INTERVAL};
use vigia_lib::commands::{
    checked_account_url, reset_secrets_and_restart, retry_secrets_and_restart, settings_view,
};
use vigia_lib::config::{
    Account, AccountKind, Config, ConfigError, ConfigStore, FilterSet, OrgFilters, WatchedRepo,
};
use vigia_lib::model::{RepoStatus, Run, RunState};
use vigia_lib::notes;
use vigia_lib::notify::{ClickTarget, Notification};
use vigia_lib::poller::Snapshot;
use vigia_lib::providers::{
    AccountIdentity, FetchOutcome, FetchRequest, Provider, ProviderError, RepoCache, RepoInfo,
};
use vigia_lib::secrets::{MemoryStore, SecretError, SecretStore, Secrets};

const BASE: OffsetDateTime = datetime!(2026-03-02 09:00 UTC);

type Outcome = Result<Vec<Run>, ProviderError>;

/// Returns scripted fetch outcomes in order; the last one repeats.
struct MockProvider {
    script: Mutex<VecDeque<Outcome>>,
    listed: Mutex<Result<Vec<RepoInfo>, ProviderError>>,
    list_calls: AtomicUsize,
    /// `refresh_groups` of every fetch, in order.
    fetches: Mutex<Vec<bool>>,
    /// The repo ID and the cached runs ETag of every fetch, in order.
    requests: Mutex<Vec<(u64, Option<String>)>>,
    /// How long each fetch takes.
    delay: Mutex<Option<StdDuration>>,
}

impl MockProvider {
    fn new(script: Vec<Outcome>) -> Arc<MockProvider> {
        Arc::new(MockProvider {
            script: Mutex::new(script.into()),
            listed: Mutex::new(Ok(Vec::new())),
            list_calls: AtomicUsize::new(0),
            fetches: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            delay: Mutex::new(None),
        })
    }

    fn requests(&self) -> Vec<(u64, Option<String>)> {
        self.requests.lock().unwrap().clone()
    }

    fn fetch_count(&self) -> usize {
        self.fetches.lock().unwrap().len()
    }

    fn last_refresh_groups(&self) -> Option<bool> {
        self.fetches.lock().unwrap().last().copied()
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn validate(&self) -> Result<AccountIdentity, ProviderError> {
        Ok(AccountIdentity {
            user_id: 1,
            login: "mock".into(),
        })
    }

    async fn list_repos(
        &self,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Vec<RepoInfo>, ProviderError> {
        self.list_calls.fetch_add(1, Ordering::SeqCst);
        let listed = self.listed.lock().unwrap().clone();
        if let Ok(list) = &listed {
            on_page(list.len());
        }
        listed
    }

    async fn fetch(
        &self,
        request: FetchRequest<'_>,
        cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError> {
        self.fetches.lock().unwrap().push(request.refresh_groups);
        self.requests
            .lock()
            .unwrap()
            .push((request.repo.id, cache.runs_etag.clone()));
        let delay = *self.delay.lock().unwrap();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
        let next = {
            let mut script = self.script.lock().unwrap();
            if script.len() > 1 {
                script.pop_front().unwrap()
            } else {
                script.front().cloned().unwrap()
            }
        };
        let not_modified = cache.runs_etag.is_some();
        let outcome = next.map(|runs| FetchOutcome {
            runs,
            active_groups: None,
            not_modified,
            counted_requests: u32::from(!not_modified),
            rate_limit: None,
        });
        if let Ok(outcome) = &outcome {
            cache.runs_etag = Some("\"etag-1\"".into());
            cache.runs = outcome.runs.clone();
        }
        outcome
    }
}

/// Test time: `BASE` plus the paused tokio clock's elapsed time plus a jump set by the test.
struct TestClock {
    start: tokio::time::Instant,
    jump_secs: AtomicI64,
}

impl TestClock {
    fn new() -> Arc<TestClock> {
        Arc::new(TestClock {
            start: tokio::time::Instant::now(),
            jump_secs: AtomicI64::new(0),
        })
    }

    fn clock(self: &Arc<Self>) -> Clock {
        let clock = Arc::clone(self);
        Arc::new(move || {
            let elapsed = tokio::time::Instant::now() - clock.start;
            BASE + Duration::try_from(elapsed).unwrap()
                + Duration::seconds(clock.jump_secs.load(Ordering::SeqCst))
        })
    }

    /// Moves the wall clock forward without moving tokio time, as a sleeping Mac does.
    fn jump(&self, secs: i64) {
        self.jump_secs.fetch_add(secs, Ordering::SeqCst);
    }
}

struct Harness {
    runtime: Arc<Runtime>,
    providers: Arc<Mutex<HashMap<String, Arc<MockProvider>>>>,
    builds: Arc<AtomicUsize>,
    published: Arc<Mutex<Vec<Snapshot>>>,
    notified: Arc<Mutex<Vec<Notification>>>,
    clock: Arc<TestClock>,
}

impl Harness {
    fn new(config: Config, secrets: Secrets) -> Harness {
        Harness::with_store(ConfigStore::in_memory(config), secrets)
    }

    fn with_store(store: ConfigStore, secrets: Secrets) -> Harness {
        let providers: Arc<Mutex<HashMap<String, Arc<MockProvider>>>> = Arc::default();
        let builds = Arc::new(AtomicUsize::new(0));
        let published: Arc<Mutex<Vec<Snapshot>>> = Arc::default();
        let notified: Arc<Mutex<Vec<Notification>>> = Arc::default();
        let clock = TestClock::new();
        let factory_providers = Arc::clone(&providers);
        let factory_builds = Arc::clone(&builds);
        let publish_log = Arc::clone(&published);
        let notify_log = Arc::clone(&notified);
        let runtime = Runtime::new(
            store,
            Arc::new(secrets),
            Arc::new(move |s: &Snapshot| publish_log.lock().unwrap().push(s.clone())),
            Arc::new(move |n: &Notification| notify_log.lock().unwrap().push(n.clone())),
        )
        .with_provider_factory(Arc::new(move |account: &Account, _token: &str| {
            factory_builds.fetch_add(1, Ordering::SeqCst);
            match factory_providers.lock().unwrap().get(&account.id) {
                Some(p) => Ok(Arc::clone(p) as Arc<dyn Provider>),
                None => Err(ProviderError::Decode("no scripted provider".into())),
            }
        }))
        .with_clock(clock.clock())
        .with_spawn_handle(Handle::current());
        Harness {
            runtime: Arc::new(runtime),
            providers,
            builds,
            published,
            notified,
            clock,
        }
    }

    fn provide(&self, account: &Account, provider: &Arc<MockProvider>) {
        self.providers
            .lock()
            .unwrap()
            .insert(account.id.clone(), Arc::clone(provider));
        self.runtime
            .secrets
            .set_token(&account.id, "fake-token")
            .unwrap();
    }

    fn notifications(&self) -> Vec<Notification> {
        self.notified.lock().unwrap().clone()
    }
}

fn github(label: &str, user_id: u64) -> Account {
    let mut account = Account::new(AccountKind::GitHub, label, None);
    account.user_id = Some(user_id);
    account
}

fn gitlab(label: &str) -> Account {
    Account::new(
        AccountKind::GitLab,
        label,
        Some("https://gitlab.example.test".into()),
    )
}

fn repo(id: u64) -> RepoInfo {
    common::acme_repo(id, "example.test")
}

fn watched(account: &Account, id: u64) -> WatchedRepo {
    common::watched_repo(&account.id, repo(id), None)
}

fn run(id: u64, state: RunState) -> Run {
    Run {
        name: "CI".into(),
        group_name: "CI".into(),
        ..common::run_in(
            id,
            1,
            "main",
            "ci",
            state,
            format!("https://example.test/acme/r1/runs/{id}"),
            BASE,
        )
    }
}

fn config_with(accounts: &[(&Account, &[u64])]) -> Config {
    let mut config = Config::default();
    for (account, repos) in accounts {
        config.accounts.push((*account).clone());
        for id in *repos {
            config.repos.push(watched(account, *id));
        }
    }
    config
}

/// A store the test keeps a handle to, so it can make reads fail or succeed.
struct SharedStore {
    inner: Arc<MemoryStore>,
    fail_writes: Arc<std::sync::atomic::AtomicBool>,
}

impl SecretStore for SharedStore {
    fn read(&self) -> Result<Option<String>, SecretError> {
        self.inner.read()
    }

    fn write(&self, value: &str) -> Result<(), SecretError> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(SecretError::AccessDenied("denied".into()));
        }
        self.inner.write(value)
    }
}

fn memory_secrets() -> Secrets {
    Secrets::new(Box::new(MemoryStore::default()))
}

/// Lets spawned tasks run for a few milliseconds of paused time until `done` holds.
async fn wait_until(mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(StdDuration::from_millis(5)).await;
    }
    panic!("condition not reached");
}

/// Lets spawned tasks run without moving far in time.
async fn settle() {
    tokio::time::sleep(StdDuration::from_millis(50)).await;
}

#[tokio::test(start_paused = true)]
async fn restart_starts_one_worker_per_account_and_lists_repos_once() {
    let gh = github("gh", 7);
    let gl = gitlab("gl");
    let h = Harness::new(config_with(&[(&gh, &[1]), (&gl, &[2])]), memory_secrets());
    let gh_provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    let gl_provider = MockProvider::new(vec![Ok(vec![run(2, RunState::Running)])]);
    h.provide(&gh, &gh_provider);
    h.provide(&gl, &gl_provider);

    h.runtime.restart();
    wait_until(|| gh_provider.fetch_count() == 1 && gl_provider.fetch_count() == 1).await;
    settle().await;
    assert_eq!(h.builds.load(Ordering::SeqCst), 2);
    assert_eq!(gh_provider.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(gl_provider.list_calls.load(Ordering::SeqCst), 1);
    let snapshot = h.runtime.snapshot();
    assert_eq!(snapshot.accounts.len(), 2);
    assert_eq!(snapshot.repos[0].state.status, RepoStatus::Success);
    assert_eq!(snapshot.repos[1].state.status, RepoStatus::Running);
    let last = h.published.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.repos.len(), 2);

    // A later restart with the same config rebuilds the providers but keeps each worker's
    // state, so nothing is fetched or listed again until the repos are due.
    h.runtime.restart();
    settle().await;
    assert_eq!(h.builds.load(Ordering::SeqCst), 4);
    assert_eq!(gh_provider.fetch_count(), 1);
    assert_eq!(gl_provider.fetch_count(), 1);
    assert_eq!(gh_provider.list_calls.load(Ordering::SeqCst), 1);
    // The GitLab repo has a running run, so it is on fast polling.
    tokio::time::sleep(StdDuration::from_secs(20)).await;
    assert_eq!(gh_provider.fetch_count(), 1);
    assert_eq!(gl_provider.fetch_count(), 2);
    tokio::time::sleep(StdDuration::from_secs(50)).await;
    assert_eq!(gh_provider.fetch_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn first_start_saves_renamed_repos_to_the_config() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1, 2])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut renamed = repo(1);
    renamed.full_name = "acme/renamed".into();
    *provider.listed.lock().unwrap() = Ok(vec![renamed, repo(2)]);
    h.provide(&gh, &provider);

    h.runtime.restart();
    wait_until(|| provider.list_calls.load(Ordering::SeqCst) == 1).await;
    tokio::time::sleep(PUBLISH_INTERVAL * 2).await;
    // The list is fetched after the first poll, and its page counts toward the pool.
    assert_eq!(provider.fetch_count(), 2);
    let repos = h.runtime.with_config(|c| c.config().repos.clone());
    assert_eq!(repos[0].repo.full_name, "acme/renamed");
    assert_eq!(repos[1].repo, repo(2));
    let names: Vec<String> = h
        .published
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .repos
        .iter()
        .map(|r| r.repo.full_name.clone())
        .collect();
    assert!(names.contains(&"acme/renamed".to_string()), "{names:?}");
    let pool = h.runtime.pool_for(&gh);
    assert_eq!(pool.lock().unwrap().requests_last_hour(h.runtime.now()), 3);
}

#[tokio::test(start_paused = true)]
async fn failed_or_unchanged_repo_list_leaves_the_config_alone() {
    let a = github("a", 7);
    let b = github("b", 8);
    let config = config_with(&[(&a, &[1]), (&b, &[2])]);
    let h = Harness::new(config.clone(), memory_secrets());
    let failing = MockProvider::new(vec![Ok(vec![])]);
    *failing.listed.lock().unwrap() = Err(ProviderError::Network("offline".into()));
    let unchanged = MockProvider::new(vec![Ok(vec![])]);
    *unchanged.listed.lock().unwrap() = Ok(vec![repo(2)]);
    h.provide(&a, &failing);
    h.provide(&b, &unchanged);

    h.runtime.restart();
    wait_until(|| failing.fetch_count() == 1 && unchanged.fetch_count() == 1).await;
    settle().await;
    assert_eq!(failing.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(unchanged.list_calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.runtime.with_config(|c| c.config().clone()), config);
}

#[tokio::test(start_paused = true)]
async fn paused_first_start_skips_the_repo_list() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![])]);
    h.provide(&gh, &provider);

    h.runtime.set_paused(true);
    h.runtime.restart();
    settle().await;
    assert_eq!(provider.list_calls.load(Ordering::SeqCst), 0);
    assert_eq!(provider.fetch_count(), 0);
    assert!(h.runtime.snapshot().paused);
}

#[tokio::test(start_paused = true)]
async fn account_without_token_shows_every_repo_in_error() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1, 2])]), memory_secrets());

    h.runtime.restart();
    settle().await;
    assert_eq!(h.builds.load(Ordering::SeqCst), 0);
    let snapshot = h.runtime.snapshot();
    let account = &snapshot.accounts[0];
    assert!(account.auth_error);
    assert!(!account.keychain_denied);
    assert_eq!(account.error.as_deref(), Some(notes::NO_TOKEN));
    assert_eq!(snapshot.repos.len(), 2);
    for r in &snapshot.repos {
        assert_eq!(r.state.status, RepoStatus::Error);
        assert_eq!(r.state.note.as_deref(), Some(notes::NO_TOKEN));
    }
    assert!(h.notifications().is_empty());
    assert!(!h.runtime.with_notifier(|n| n.auth_notified(&gh.id)));
}

#[tokio::test(start_paused = true)]
async fn blocked_keychain_marks_accounts_as_denied() {
    let gl = gitlab("gl");
    let h = Harness::new(
        config_with(&[(&gl, &[3])]),
        Secrets::new(Box::new(MemoryStore::failing())),
    );

    h.runtime.restart();
    let snapshot = h.runtime.snapshot();
    assert!(snapshot.secrets_blocked);
    let account = &snapshot.accounts[0];
    assert!(account.keychain_denied);
    assert_eq!(account.error.as_deref(), Some(notes::KEYCHAIN_DENIED));
    assert_eq!(
        snapshot.repos[0].state.note.as_deref(),
        Some(notes::KEYCHAIN_DENIED)
    );
}

#[tokio::test(start_paused = true)]
async fn provider_build_error_is_treated_like_a_missing_token() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    h.runtime.secrets.set_token(&gh.id, "fake-token").unwrap();

    h.runtime.restart();
    assert_eq!(h.builds.load(Ordering::SeqCst), 1);
    assert!(h.runtime.snapshot().accounts[0].auth_error);
}

#[tokio::test(start_paused = true)]
async fn pools_are_shared_per_github_user_and_separate_per_gitlab_account() {
    let a = github("a", 7);
    let b = github("b", 7);
    let c = github("c", 8);
    let gl1 = gitlab("gl1");
    let gl2 = gitlab("gl2");
    let h = Harness::new(Config::default(), memory_secrets());
    let r = &h.runtime;

    assert!(Arc::ptr_eq(&r.pool_for(&a), &r.pool_for(&b)));
    assert!(!Arc::ptr_eq(&r.pool_for(&a), &r.pool_for(&c)));
    assert!(!Arc::ptr_eq(&r.pool_for(&gl1), &r.pool_for(&gl2)));
    assert!(Arc::ptr_eq(&r.pool_for(&gl1), &r.pool_for(&gl1)));
}

#[tokio::test(start_paused = true)]
async fn restart_recounts_pool_repos_and_drops_pools_of_removed_accounts() {
    let a = github("a", 7);
    let b = github("b", 7);
    let h = Harness::new(config_with(&[(&a, &[1, 2]), (&b, &[3])]), memory_secrets());
    h.provide(&a, &MockProvider::new(vec![Ok(vec![])]));
    h.provide(&b, &MockProvider::new(vec![Ok(vec![])]));

    h.runtime.restart();
    let pool = h.runtime.pool_for(&a);
    assert_eq!(pool.lock().unwrap().repo_count(), 3);

    let b_id = b.id.clone();
    h.runtime
        .update_config(|c| c.remove_account(&b_id))
        .unwrap();
    h.runtime.restart();
    assert!(Arc::ptr_eq(&pool, &h.runtime.pool_for(&a)));
    assert_eq!(pool.lock().unwrap().repo_count(), 2);

    let a_id = a.id.clone();
    h.runtime
        .update_config(|c| c.remove_account(&a_id))
        .unwrap();
    h.runtime.restart();
    assert!(h.runtime.snapshot().repos.is_empty());
    assert!(!Arc::ptr_eq(&pool, &h.runtime.pool_for(&a)));
}

#[tokio::test(start_paused = true)]
async fn pause_stops_polling_and_resume_polls_at_once() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;

    h.runtime.set_paused(true);
    assert!(h.published.lock().unwrap().last().unwrap().paused);
    tokio::time::sleep(StdDuration::from_secs(600)).await;
    assert_eq!(provider.fetch_count(), 1);

    // Refresh is ignored while paused, so the resume poll does not refresh groups.
    h.runtime.refresh_now();
    h.runtime.set_paused(false);
    wait_until(|| provider.fetch_count() == 2).await;
    assert_eq!(provider.last_refresh_groups(), Some(false));
}

#[tokio::test(start_paused = true)]
async fn refresh_now_forces_a_poll_that_refreshes_groups() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;
    assert_eq!(provider.last_refresh_groups(), Some(false));

    h.runtime.refresh_now();
    wait_until(|| provider.fetch_count() == 2).await;
    assert_eq!(provider.last_refresh_groups(), Some(true));
}

#[tokio::test(start_paused = true)]
async fn workers_poll_again_when_the_next_repo_is_due() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;

    // The default interval is 60 s with up to 10% jitter.
    tokio::time::sleep(StdDuration::from_secs(50)).await;
    assert_eq!(provider.fetch_count(), 1);
    tokio::time::sleep(StdDuration::from_secs(20)).await;
    assert_eq!(provider.fetch_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn heartbeat_gap_counts_as_a_wake_and_forces_a_poll() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    h.provide(&gh, &provider);
    h.runtime.start_heartbeat();
    h.runtime.start_heartbeat();
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;

    // An ordinary heartbeat gap is not a wake.
    tokio::time::sleep(StdDuration::from_secs(12)).await;
    assert_eq!(provider.fetch_count(), 1);

    // The wall clock jumps past the wake gap but short of the repo's next poll, which without
    // a wake would come about a minute of tokio time after the first one.
    h.clock.jump(36);
    tokio::time::sleep(StdDuration::from_secs(4)).await;
    assert_eq!(provider.fetch_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn new_failure_after_the_baseline_notifies_through_the_sink() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![
        Ok(vec![run(1, RunState::Success)]),
        Ok(vec![run(2, RunState::Failed)]),
    ]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;
    assert!(h.notifications().is_empty());

    h.runtime.refresh_now();
    wait_until(|| provider.fetch_count() == 2).await;
    settle().await;
    let sent = h.notifications();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].target,
        ClickTarget::Run {
            account_id: gh.id.clone(),
            url: "https://example.test/acme/r1/runs/2".into(),
        }
    );

    // A restart with the same config keeps the baseline, so the same failure stays quiet.
    h.runtime.restart();
    tokio::time::sleep(StdDuration::from_secs(70)).await;
    assert_eq!(provider.fetch_count(), 3);
    assert_eq!(h.notifications().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn rejected_token_notifies_once_and_waits_for_a_wake() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Err(ProviderError::Unauthorized)]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;
    settle().await;
    let sent = h.notifications();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, ClickTarget::Popup);
    assert!(h.runtime.snapshot().accounts[0].auth_error);

    tokio::time::sleep(StdDuration::from_secs(3600)).await;
    h.runtime.refresh_now();
    settle().await;
    assert_eq!(provider.fetch_count(), 1);
    assert_eq!(h.notifications().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_workers() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![])]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;

    h.runtime.controls.shutdown();
    h.runtime.refresh_now();
    tokio::time::sleep(StdDuration::from_secs(600)).await;
    assert_eq!(provider.fetch_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn snapshot_carries_config_and_secret_flags() {
    let h = Harness::with_store(
        ConfigStore::unavailable(&ConfigError::ReadOnly),
        Secrets::new(Box::new(MemoryStore::failing())),
    );
    let snapshot = h.runtime.snapshot();
    assert!(snapshot.config_read_only);
    assert!(snapshot.secrets_blocked);
    assert!(snapshot.config_error.is_some());
    assert_eq!(snapshot.generated_at, BASE);
}

#[test]
fn default_factory_builds_each_provider_kind() {
    let gh = github("gh", 7);
    assert!(build_provider(&gh, "fake-token").is_ok());
    assert!(build_provider(&gitlab("gl"), "fake-token").is_ok());
    let no_base = Account::new(AccountKind::GitLab, "gl", None);
    assert!(build_provider(&no_base, "fake-token").is_err());

    let runtime = Runtime::new(
        ConfigStore::in_memory(Config::default()),
        Arc::new(memory_secrets()),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    );
    assert!(matches!(
        runtime.provider_for(&gh),
        Err(ProviderError::Unauthorized)
    ));
    runtime.secrets.set_token(&gh.id, "fake-token").unwrap();
    assert!(runtime.provider_for(&gh).is_ok());
}

#[tokio::test(start_paused = true)]
async fn settings_view_lists_config_without_tokens() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    h.runtime.secrets.set_token(&gh.id, "fake-token").unwrap();
    let view = settings_view(&h.runtime);
    assert_eq!(view.accounts, vec![gh.clone()]);
    assert_eq!(view.repos.len(), 1);
    assert!(!view.read_only);
    assert!(!view.secrets_blocked);
    assert!(!serde_json::to_string(&view).unwrap().contains("fake-token"));
}

#[tokio::test(start_paused = true)]
async fn retry_and_reset_secrets_restart_only_when_the_store_recovers() {
    let gh = github("gh", 7);
    let store = Arc::new(MemoryStore::failing());
    let fail_writes = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let h = Harness::new(
        config_with(&[(&gh, &[1])]),
        Secrets::new(Box::new(SharedStore {
            inner: Arc::clone(&store),
            fail_writes: Arc::clone(&fail_writes),
        })),
    );
    let published = || h.published.lock().unwrap().len();

    assert!(!retry_secrets_and_restart(&h.runtime));
    assert!(!reset_secrets_and_restart(&h.runtime));
    assert_eq!(published(), 0);

    *store.fail_reads.lock().unwrap() = false;
    assert!(retry_secrets_and_restart(&h.runtime));
    assert_eq!(published(), 1);

    // The restart shows the renamed account; a restart with nothing changed publishes nothing.
    h.runtime
        .update_config(|c| c.accounts[0].label = "renamed".into())
        .unwrap();
    fail_writes.store(false, Ordering::SeqCst);
    assert!(reset_secrets_and_restart(&h.runtime));
    assert_eq!(published(), 2);
    assert_eq!(
        h.published.lock().unwrap().last().unwrap().accounts[0].label,
        "renamed"
    );
    h.runtime.restart();
    assert_eq!(published(), 2);
    assert_eq!(store.value().as_deref(), Some("{}"));
}

#[tokio::test(start_paused = true)]
async fn account_urls_are_checked_against_the_account() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let ok = checked_account_url(&h.runtime, &gh.id, "https://github.com/acme/r1/actions");
    assert_eq!(ok.unwrap().as_str(), "https://github.com/acme/r1/actions");
    let missing = checked_account_url(&h.runtime, "missing", "https://github.com/acme/r1");
    assert_eq!(
        missing.unwrap_err().kind,
        vigia_lib::commands::ErrorKind::UnknownAccount
    );
    let foreign = checked_account_url(&h.runtime, &gh.id, "https://evil.example.test/x");
    assert_eq!(
        foreign.unwrap_err().kind,
        vigia_lib::commands::ErrorKind::InvalidInput
    );
}

#[tokio::test(start_paused = true)]
async fn a_restart_after_a_rename_keeps_cached_state_and_shows_it_at_once() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Failed)])]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;
    settle().await;
    tokio::time::sleep(StdDuration::from_secs(2)).await;
    let before = h.published.lock().unwrap().len();

    h.runtime
        .update_config(|c| c.accounts[0].label = "work".into())
        .unwrap();
    h.runtime.restart();
    settle().await;
    let published: Vec<Snapshot> = h.published.lock().unwrap()[before..].to_vec();
    assert!(!published.is_empty());
    for snapshot in &published {
        assert_eq!(snapshot.repos[0].state.status, RepoStatus::Failed);
        assert_eq!(snapshot.accounts[0].label, "work");
    }
    assert_eq!(provider.fetch_count(), 1);

    // The next poll sends the cached ETag: the repo is not fetched cold.
    tokio::time::sleep(StdDuration::from_secs(70)).await;
    assert_eq!(
        provider.requests(),
        vec![(1, None), (1, Some("\"etag-1\"".into()))]
    );
    assert_eq!(
        h.runtime.snapshot().repos[0].state.status,
        RepoStatus::Failed
    );
}

#[tokio::test(start_paused = true)]
async fn a_filter_change_on_one_repo_refetches_only_that_repo() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1, 2])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 2).await;

    h.runtime
        .update_config(|c| c.repos[0].branch_patterns = Some(vec!["release/*".into()]))
        .unwrap();
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 3).await;
    settle().await;
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2], (1, None));
}

#[tokio::test(start_paused = true)]
async fn a_restart_mid_cycle_hands_the_worker_over_when_the_cycle_ends() {
    let gh = github("gh", 7);
    let h = Harness::new(config_with(&[(&gh, &[1])]), memory_secrets());
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    h.provide(&gh, &provider);
    h.runtime.restart();
    wait_until(|| provider.fetch_count() == 1).await;

    // A slow response keeps the next cycle in flight while the restart happens.
    *provider.delay.lock().unwrap() = Some(StdDuration::from_secs(3));
    h.runtime.refresh_now();
    wait_until(|| provider.fetch_count() == 2).await;
    h.runtime.restart();
    settle().await;
    assert_eq!(
        h.runtime.snapshot().repos[0].state.status,
        RepoStatus::Success
    );
    tokio::time::sleep(StdDuration::from_secs(4)).await;
    *provider.delay.lock().unwrap() = None;

    // The successor adopted the cache filled before the restart.
    tokio::time::sleep(StdDuration::from_secs(70)).await;
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].1.is_some());
}

#[tokio::test(start_paused = true)]
async fn worker_publishes_are_throttled_across_accounts() {
    let a = github("a", 7);
    let b = github("b", 8);
    let h = Harness::new(config_with(&[(&a, &[1]), (&b, &[2])]), memory_secrets());
    let pa = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    let pb = MockProvider::new(vec![Ok(vec![run(2, RunState::Failed)])]);
    h.provide(&a, &pa);
    h.provide(&b, &pb);

    h.runtime.restart();
    wait_until(|| pa.fetch_count() == 1 && pb.fetch_count() == 1).await;
    // The restart published; both cycles landed within the same second, so one more publish
    // follows when the second ends.
    assert_eq!(h.published.lock().unwrap().len(), 1);
    tokio::time::sleep(StdDuration::from_millis(1100)).await;
    let published = h.published.lock().unwrap().clone();
    assert_eq!(published.len(), 2);
    let statuses: Vec<RepoStatus> = published[1].repos.iter().map(|r| r.state.status).collect();
    assert_eq!(statuses, vec![RepoStatus::Success, RepoStatus::Failed]);
}

fn org(host: &str, owner: &str, filters: FilterSet) -> OrgFilters {
    OrgFilters {
        host: host.into(),
        owner: owner.into(),
        filters,
    }
}

#[tokio::test(start_paused = true)]
async fn settings_view_lists_the_orgs_of_watched_repos() {
    let gh = github("gh", 7);
    let gl = gitlab("gl");
    let mut config = config_with(&[(&gh, &[1, 2, 4, 5]), (&gl, &[3])]);
    config.repos[2].repo.full_name = "Org-10/tools".into();
    config.repos[3].repo.full_name = "org-2/tools".into();
    config.repos[4].repo.full_name = "acme/web/app".into();
    config.organizations = vec![
        org(
            "github.com",
            "ACME",
            FilterSet {
                branch_patterns: Some(vec![]),
                ignored_workflows: Some(vec!["Dependabot*".into()]),
                include_tags: Some(true),
            },
        ),
        org(
            "github.com",
            "example",
            FilterSet {
                include_tags: Some(true),
                ..Default::default()
            },
        ),
    ];
    let h = Harness::new(config, memory_secrets());
    let json = serde_json::to_value(settings_view(&h.runtime)).unwrap();
    assert_eq!(
        json["organizations"],
        serde_json::json!([
            {
                "host": "github.com",
                "owner": "acme",
                "filters": {
                    "branch_patterns": [],
                    "ignored_workflows": ["Dependabot*"],
                    "include_tags": true,
                },
                "repo_count": 2,
            },
            {
                "host": "github.com",
                "owner": "org-2",
                "filters": { "branch_patterns": null, "ignored_workflows": null, "include_tags": null },
                "repo_count": 1,
            },
            {
                "host": "github.com",
                "owner": "Org-10",
                "filters": { "branch_patterns": null, "ignored_workflows": null, "include_tags": null },
                "repo_count": 1,
            },
            {
                "host": "gitlab.example.test",
                "owner": "acme",
                "filters": { "branch_patterns": null, "ignored_workflows": null, "include_tags": null },
                "repo_count": 1,
            },
        ])
    );
    assert_eq!(json["repos"][0]["host"], "github.com");
    assert_eq!(json["repos"][0]["owner"], "acme");
    assert_eq!(json["repos"][0]["repo"]["full_name"], "acme/r1");
    assert_eq!(json["repos"][2]["owner"], "Org-10");
    assert_eq!(json["repos"][4]["host"], "gitlab.example.test");
    assert_eq!(json["repos"][4]["owner"], "acme");
    assert!(json["accounts"][0].get("filters").is_none());
    assert_eq!(
        json["accounts"][1]["base_url"],
        "https://gitlab.example.test"
    );
}

#[tokio::test(start_paused = true)]
async fn an_org_filter_change_refetches_and_rebaselines_only_affected_repos() {
    let gh = github("gh", 7);
    let second = github("second", 8);
    let gl = gitlab("gl");
    let mut config = config_with(&[(&gh, &[1, 2]), (&second, &[4]), (&gl, &[3])]);
    // Repo 2 ignores nothing on its own, so an organization ignore list does not reach it.
    config.repos[1].ignored_workflows = Some(vec![]);
    let h = Harness::new(config, memory_secrets());
    let provider = MockProvider::new(vec![
        Ok(vec![run(1, RunState::Success)]),
        Ok(vec![run(1, RunState::Success)]),
        Ok(vec![run(2, RunState::Failed)]),
    ]);
    let shared = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    let other = MockProvider::new(vec![Ok(vec![run(1, RunState::Success)])]);
    h.provide(&gh, &provider);
    h.provide(&second, &shared);
    h.provide(&gl, &other);
    h.runtime.restart();
    wait_until(|| {
        provider.fetch_count() == 2 && shared.fetch_count() == 1 && other.fetch_count() == 1
    })
    .await;

    vigia_lib::settings::set_org_filters(
        &h.runtime,
        "github.com",
        "acme",
        FilterSet {
            ignored_workflows: Some(vec!["Nightly*".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    // Repo 1 and the second account's acme repo refetch; the GitLab acme group is another org.
    wait_until(|| provider.fetch_count() == 3 && shared.fetch_count() == 2).await;
    settle().await;
    assert_eq!(provider.requests()[2], (1, None));
    assert_eq!(shared.requests()[1], (4, None));
    assert_eq!(other.fetch_count(), 1);
    // Repo 1's new failure only sets its fresh baseline.
    assert!(h.notifications().is_empty());

    // Repo 2 kept its baseline, so the same failure notifies there; repo 1 stays quiet.
    h.runtime.refresh_now();
    wait_until(|| provider.fetch_count() == 5).await;
    settle().await;
    let sent = h.notifications();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].title.starts_with("r2"), "{}", sent[0].title);
}

#[tokio::test(start_paused = true)]
async fn a_repo_moved_to_another_org_takes_its_filters_after_the_repo_list_refresh() {
    let gh = github("gh", 7);
    let mut config = config_with(&[(&gh, &[1])]);
    config.organizations = vec![org(
        "github.com",
        "example",
        FilterSet {
            ignored_workflows: Some(vec!["Nightly*".into()]),
            ..Default::default()
        },
    )];
    let h = Harness::new(config, memory_secrets());
    let provider = MockProvider::new(vec![
        Ok(vec![run(1, RunState::Success)]),
        Ok(vec![run(1, RunState::Success)]),
        Ok(vec![run(2, RunState::Failed)]),
    ]);
    let mut moved = repo(1);
    moved.full_name = "example/r1".into();
    *provider.listed.lock().unwrap() = Ok(vec![moved]);
    h.provide(&gh, &provider);

    h.runtime.restart();
    // The first poll, then a refetch once the list shows the repo under its new owner.
    wait_until(|| provider.list_calls.load(Ordering::SeqCst) == 1).await;
    tokio::time::sleep(StdDuration::from_secs(2)).await;
    assert_eq!(provider.fetch_count(), 2);
    assert_eq!(provider.requests()[1], (1, None));
    let saved = h
        .runtime
        .with_config(|c| c.config().repos[0].repo.full_name.clone());
    assert_eq!(saved, "example/r1");
    let json = serde_json::to_value(settings_view(&h.runtime)).unwrap();
    assert_eq!(json["organizations"][0]["owner"], "example");

    // A later restart compares against the new name, so the repo keeps its baseline and the
    // next failure notifies.
    vigia_lib::settings::update_settings(
        &h.runtime,
        h.runtime.with_config(|c| c.config().settings.clone()),
    )
    .unwrap();
    settle().await;
    assert_eq!(provider.fetch_count(), 2);
    h.runtime.refresh_now();
    wait_until(|| provider.fetch_count() == 3).await;
    settle().await;
    let sent = h.notifications();
    assert_eq!(sent.len(), 1, "{sent:?}");
}
