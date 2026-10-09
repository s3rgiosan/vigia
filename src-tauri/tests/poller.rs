mod common;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};
use vigia_lib::config::{Account, AccountKind, FilterSet, OrgFilters, Settings, WatchedRepo};
use vigia_lib::filters::FilterCompiler;
use vigia_lib::http::RateLimit;
use vigia_lib::model::{RepoStatus, Run, RunState};
use vigia_lib::poller::{
    due_after, poll_repo, repo_backoff_secs, AccountWorker, AuthReason, RepoRuntime,
    BACKOFF_MAX_SECS, DUE_BUCKET_SECS, FAILURES_BEFORE_ERROR,
};
use vigia_lib::pool::FAST_POLL_SECS;
use vigia_lib::pool::{PoolKind, PoolState, RequestClass};
use vigia_lib::providers::{
    AccountIdentity, FetchOutcome, FetchRequest, Membership, Provider, ProviderError, RepoCache,
    RepoInfo,
};

const NOW: OffsetDateTime = datetime!(2026-06-01 12:00 UTC);

type Outcome = Result<Vec<Run>, ProviderError>;

/// Returns scripted outcomes in order; the last one repeats. A repo with its own script uses
/// that one.
struct MockProvider {
    script: Mutex<VecDeque<Outcome>>,
    per_repo: Mutex<HashMap<u64, VecDeque<Outcome>>>,
    rate_limit: Mutex<Option<RateLimit>>,
    member: Mutex<Result<bool, ProviderError>>,
    /// Set on the first fetch, to pause the poller mid-cycle.
    pause_on_fetch: Mutex<Option<Arc<AtomicBool>>>,
    /// Yield once before answering, so concurrent fetches are all in flight together.
    yield_first: AtomicBool,
    calls: Mutex<Vec<u64>>,
    refresh_groups: Mutex<Vec<bool>>,
    include_tags: Mutex<Vec<bool>>,
    /// The branch patterns of every fetch, in order; empty for the default branch.
    branch_patterns: Mutex<Vec<Vec<String>>>,
}

impl MockProvider {
    fn new(script: Vec<Outcome>) -> Arc<MockProvider> {
        Arc::new(MockProvider {
            script: Mutex::new(script.into()),
            per_repo: Mutex::new(HashMap::new()),
            rate_limit: Mutex::new(None),
            member: Mutex::new(Ok(true)),
            pause_on_fetch: Mutex::new(None),
            yield_first: AtomicBool::new(false),
            calls: Mutex::new(Vec::new()),
            refresh_groups: Mutex::new(Vec::new()),
            include_tags: Mutex::new(Vec::new()),
            branch_patterns: Mutex::new(Vec::new()),
        })
    }

    fn script_repo(&self, repo_id: u64, script: Vec<Outcome>) {
        self.per_repo.lock().unwrap().insert(repo_id, script.into());
    }

    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

fn next_outcome(script: &mut VecDeque<Outcome>) -> Outcome {
    if script.len() > 1 {
        script.pop_front().unwrap()
    } else {
        script.front().cloned().unwrap()
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
        on_page(0);
        Ok(vec![])
    }

    async fn is_member(&self, _repo: &RepoInfo) -> Result<bool, ProviderError> {
        self.member.lock().unwrap().clone()
    }

    async fn fetch(
        &self,
        request: FetchRequest<'_>,
        _cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError> {
        if self.yield_first.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        let repo_id = request.repo.id;
        self.calls.lock().unwrap().push(repo_id);
        self.refresh_groups
            .lock()
            .unwrap()
            .push(request.refresh_groups);
        self.include_tags.lock().unwrap().push(request.include_tags);
        self.branch_patterns
            .lock()
            .unwrap()
            .push(request.branch_filter.patterns().to_vec());
        if let Some(flag) = self.pause_on_fetch.lock().unwrap().as_ref() {
            flag.store(true, Ordering::SeqCst);
        }
        let next = match self.per_repo.lock().unwrap().get_mut(&repo_id) {
            Some(script) => next_outcome(script),
            None => next_outcome(&mut self.script.lock().unwrap()),
        };
        let rate_limit = *self.rate_limit.lock().unwrap();
        next.map(|runs| FetchOutcome {
            runs,
            active_groups: None,
            not_modified: false,
            counted_requests: 1,
            rate_limit,
        })
    }
}

fn run(id: u64, state: RunState, updated_at: OffsetDateTime) -> Run {
    Run {
        name: "CI".into(),
        group_name: "CI".into(),
        ..common::run_in(
            id,
            1,
            "main",
            "ci",
            state,
            format!("https://example.test/{id}"),
            updated_at,
        )
    }
}

fn watched(id: u64, account_id: &str) -> WatchedRepo {
    common::watched_repo(account_id, common::acme_repo(id, "example.test"), None)
}

fn worker(kind: AccountKind, provider: Arc<MockProvider>, repos: usize) -> AccountWorker {
    worker_with(kind, provider, repos, |_| {})
}

/// A worker whose watched repos are adjusted by `edit` before their filters are compiled.
fn worker_with(
    kind: AccountKind,
    provider: Arc<MockProvider>,
    repos: usize,
    edit: impl Fn(&mut WatchedRepo),
) -> AccountWorker {
    let account = Account::new(kind, "test", None);
    let pool_kind = match kind {
        AccountKind::GitHub => PoolKind::GitHub,
        AccountKind::GitLab => PoolKind::GitLab,
    };
    let watched: Vec<WatchedRepo> = (1..=repos as u64)
        .map(|i| {
            let mut w = watched(i, &account.id);
            edit(&mut w);
            w
        })
        .collect();
    AccountWorker::new(
        account,
        provider,
        Arc::new(Mutex::new(PoolState::new(pool_kind))),
        watched,
        Settings::default(),
        NOW,
    )
}

#[tokio::test]
async fn successful_cycle_sets_repo_state_and_schedules_base_interval() {
    let provider = MockProvider::new(vec![Ok(vec![run(
        1,
        RunState::Failed,
        NOW - Duration::hours(2),
    )])]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 2);
    let report = w.cycle(NOW, false, false).await;
    assert_eq!(report.polled, 2);
    assert_eq!(provider.calls(), 2);
    assert_eq!(w.repos[0].state.status, RepoStatus::Failed);
    assert!(!w.repos[0].state.stale);
    assert_eq!(w.repos[0].state.last_checked, Some(NOW));
    let due = w.repos[0].next_due - NOW;
    // 60 s with 10 percent jitter, rounded up to a due-time bucket.
    assert!(
        due >= Duration::seconds(54) && due <= Duration::seconds(66 + DUE_BUCKET_SECS),
        "{due}"
    );
    assert_eq!(w.repos[0].next_due.unix_timestamp() % DUE_BUCKET_SECS, 0);
    assert_eq!(w.repos[0].next_due.nanosecond(), 0);
    assert!(!w.repos[0].fast);
}

#[tokio::test]
async fn repos_are_only_polled_when_due_unless_forced() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 1);
    w.cycle(NOW, false, false).await;
    let report = w.cycle(NOW + Duration::seconds(5), false, false).await;
    assert_eq!(report.polled, 0);
    let report = w.cycle(NOW + Duration::seconds(5), true, false).await;
    assert_eq!(report.polled, 1);
    assert_eq!(provider.calls(), 2);
}

#[tokio::test]
async fn running_run_turns_on_fast_polling_within_the_cap() {
    let provider = MockProvider::new(vec![Ok(vec![run(
        1,
        RunState::Running,
        NOW - Duration::minutes(5),
    )])]);
    // 8 candidates, GitHub cap is 6.
    let mut w = worker(AccountKind::GitHub, provider, 8);
    w.cycle(NOW, false, false).await;
    let fast = w.repos.iter().filter(|r| r.fast).count();
    assert_eq!(fast, 6);
    let fast_repo = w.repos.iter().find(|r| r.fast).unwrap();
    let due = fast_repo.next_due - NOW;
    assert!(
        due <= Duration::seconds(FAST_POLL_SECS as i64 + 2 + DUE_BUCKET_SECS),
        "{due}"
    );
}

#[tokio::test]
async fn old_running_run_does_not_fast_poll() {
    let provider = MockProvider::new(vec![Ok(vec![run(
        1,
        RunState::Queued,
        NOW - Duration::hours(2),
    )])]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert!(!w.repos[0].fast);
}

#[tokio::test]
async fn network_errors_keep_state_stale_then_error_after_three() {
    let provider = MockProvider::new(vec![
        Ok(vec![run(1, RunState::Success, NOW - Duration::hours(1))]),
        Err(ProviderError::Network("timeout".into())),
    ]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Success);

    for i in 1..FAILURES_BEFORE_ERROR {
        w.cycle(NOW + Duration::minutes(i as i64 * 10), true, false)
            .await;
        assert_eq!(w.repos[0].state.status, RepoStatus::Success, "attempt {i}");
        assert!(w.repos[0].state.stale);
        assert!(w.status.unreachable);
    }
    w.cycle(NOW + Duration::hours(1), true, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    assert_eq!(
        w.repos[0].state.note.as_deref(),
        Some("Can't reach the server")
    );
    assert!(w.status.unreachable);
    assert_eq!(w.status.error.as_deref(), Some("Can't reach the server"));
    assert!(w.backoff_secs > 60);
}

#[tokio::test]
async fn grace_period_does_not_count_failures() {
    let provider = MockProvider::new(vec![Err(ProviderError::Network("down".into()))]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.grace_until = Some(NOW + Duration::seconds(30));
    for i in 0..5 {
        w.cycle(NOW + Duration::seconds(i), true, false).await;
    }
    assert_eq!(w.repos[0].failures, 0);
    assert_ne!(w.repos[0].state.status, RepoStatus::Error);
    // After the grace period failures count again.
    w.cycle(NOW + Duration::seconds(31), true, false).await;
    assert_eq!(w.repos[0].failures, 1);
}

#[tokio::test]
async fn unauthorized_marks_account_and_stops_polling() {
    let provider = MockProvider::new(vec![Err(ProviderError::Unauthorized)]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 2);
    w.cycle(NOW, false, false).await;
    assert!(w.status.auth_error);
    assert_eq!(w.status.auth_reason, Some(AuthReason::Rejected));
    assert!(w.repos.iter().all(|r| r.state.status == RepoStatus::Error));
    assert_eq!(
        w.status.error.as_deref(),
        Some("Token rejected. Replace it in Settings.")
    );
    assert!(w
        .repos
        .iter()
        .all(|r| r.state.note.as_deref() == Some("Token rejected. Replace it in Settings.")));
    let report = w.cycle(NOW + Duration::minutes(5), true, false).await;
    assert_eq!(report.polled, 0);
    assert_eq!(report.skipped_reason.as_deref(), Some("auth error"));
    // The 401 stopped the first cycle before the second repo was requested.
    assert_eq!(provider.calls(), 1);
}

#[tokio::test]
async fn rate_limit_keeps_state_stale_and_pauses_the_pool() {
    let provider = MockProvider::new(vec![
        Ok(vec![run(1, RunState::Success, NOW - Duration::hours(1))]),
        Err(ProviderError::RateLimited {
            retry_after: Some(120),
            reset_at: None,
            remaining: None,
        }),
        Ok(vec![run(1, RunState::Success, NOW - Duration::hours(1))]),
    ]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 1);
    w.cycle(NOW, false, false).await;
    w.cycle(NOW + Duration::minutes(1), true, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Success);
    assert!(w.repos[0].state.stale);
    let until = w.status.rate_limited_until.unwrap();
    assert_eq!(until, (NOW + Duration::minutes(3)).unix_timestamp());

    let report = w.cycle(NOW + Duration::minutes(2), true, false).await;
    assert_eq!(report.skipped_reason.as_deref(), Some("rate limited"));
    assert_eq!(provider.calls(), 2);

    let report = w.cycle(NOW + Duration::minutes(4), true, false).await;
    assert_eq!(report.polled, 1);
    assert!(w.status.rate_limited_until.is_none());
}

#[tokio::test]
async fn gitlab_forbidden_means_ci_disabled() {
    let provider = MockProvider::new(vec![Err(ProviderError::Forbidden)]);
    let mut w = worker(AccountKind::GitLab, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::None);
    assert_eq!(
        w.repos[0].state.note.as_deref(),
        Some("CI/CD is turned off")
    );
}

#[tokio::test]
async fn github_forbidden_and_not_found_are_errors() {
    let provider = MockProvider::new(vec![Err(ProviderError::Forbidden)]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    assert_eq!(
        w.repos[0].state.note.as_deref(),
        Some("Not found, or the token can't access it")
    );

    let provider = MockProvider::new(vec![Err(ProviderError::NotFound)]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    assert_eq!(
        w.repos[0].state.note.as_deref(),
        Some("Not found, or the token can't access it")
    );
}

#[tokio::test]
async fn invalid_branch_filter_is_an_error_without_a_request() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut w = worker_with(AccountKind::GitHub, provider.clone(), 1, |w| {
        w.branch_patterns = Some(vec!["[".into()]);
    });
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    assert_eq!(provider.calls(), 0);
}

#[tokio::test]
async fn invalid_workflow_filter_is_an_error_with_a_note() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut w = worker_with(AccountKind::GitHub, provider.clone(), 1, |w| {
        w.ignored_workflows = Some(vec!["[".into()]);
    });
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    let note = w.repos[0].state.note.clone().unwrap();
    assert!(note.starts_with("Workflow filter isn't valid: "), "{note}");
    assert_eq!(provider.calls(), 0);
}

#[tokio::test]
async fn gitlab_pool_stretches_interval_for_many_repos() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    // 40 repos at 60 s = 2400/h against 40% of 2000 = 800/h → 180 s.
    let mut w = worker(AccountKind::GitLab, provider, 40);
    report_gitlab_limit(&w);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.status.effective_interval_secs, 180);
}

#[tokio::test]
async fn gitlab_without_rate_limit_headers_keeps_the_configured_interval() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut w = worker(AccountKind::GitLab, provider, 40);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.status.effective_interval_secs, 60);
}

/// Records a GitLab limit of 2000 an hour, as a server sending `RateLimit` headers reports.
fn report_gitlab_limit(w: &AccountWorker) {
    w.pool.lock().unwrap().record(
        NOW,
        RequestClass::Base,
        0,
        Some(RateLimit {
            limit: 2000,
            remaining: 2000,
            reset_at: NOW.unix_timestamp() + 60,
        }),
    );
}

#[test]
fn repo_runtime_starts_due_now() {
    let r = RepoRuntime::new(watched(1, "a"), NOW);
    assert_eq!(r.next_due, NOW);
    assert_eq!(r.state.status, RepoStatus::None);
}

#[test]
fn wake_gap_threshold() {
    use vigia_lib::poller::is_wake_gap;
    assert!(!is_wake_gap(Duration::seconds(5)));
    assert!(!is_wake_gap(Duration::seconds(35)));
    assert!(is_wake_gap(Duration::seconds(36)));
    assert!(is_wake_gap(Duration::hours(8)));
}

#[tokio::test]
async fn mark_wake_resets_failures_and_opens_grace() {
    let provider = MockProvider::new(vec![Err(ProviderError::Network("down".into()))]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, true, false).await;
    w.cycle(NOW + Duration::minutes(5), true, false).await;
    assert_eq!(w.repos[0].failures, 2);
    assert!(w.backoff_secs > 0);

    let wake = NOW + Duration::hours(9);
    w.mark_wake(wake);
    assert_eq!(w.repos[0].failures, 0);
    assert_eq!(w.backoff_secs, 0);
    w.cycle(wake, true, false).await;
    assert_eq!(w.repos[0].failures, 0);
    assert_ne!(w.repos[0].state.status, RepoStatus::Error);
}

#[test]
fn apply_repo_list_updates_renamed_repos_only() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut w = worker(AccountKind::GitHub, provider, 2);
    let mut renamed = w.repos[1].watched.repo.clone();
    renamed.full_name = "acme/renamed".into();
    renamed.default_branch = "trunk".into();
    let listed = vec![w.repos[0].watched.repo.clone(), renamed.clone()];
    let update = w.apply_repo_list(&listed, NOW);
    assert_eq!(update.changed, vec![renamed]);
    assert_eq!(update.reselected, vec![2]);
    assert_eq!(w.repos[1].watched.repo.full_name, "acme/renamed");
    assert_eq!(w.repos[1].watched.repo.default_branch, "trunk");
    // A repo missing from the list is left alone.
    let update = w.apply_repo_list(&[], NOW);
    assert!(update.changed.is_empty());
    assert!(update.reselected.is_empty());
}

#[tokio::test]
async fn unpolled_repos_keep_their_schedule_while_another_fast_polls() {
    let provider = MockProvider::new(vec![Ok(vec![run(
        1,
        RunState::Running,
        NOW - Duration::minutes(5),
    )])]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 2);
    w.cycle(NOW, false, false).await;
    // Make only repo 0 fast: give repo 1 a finished run.
    w.repos[1].state.groups[0].state = RunState::Success;
    w.repos[1].fast = false;
    w.repos[1].next_due = NOW + Duration::seconds(60);
    w.repos[0].next_due = NOW + Duration::seconds(15);

    let report = w.cycle(NOW + Duration::seconds(16), false, false).await;
    assert_eq!(report.polled, 1);
    assert_eq!(
        w.repos[1].next_due,
        NOW + Duration::seconds(60),
        "a repo that was not polled keeps its due time"
    );
}

#[tokio::test]
async fn rate_limit_without_headers_doubles_the_wait() {
    let provider = MockProvider::new(vec![Err(ProviderError::RateLimited {
        retry_after: None,
        reset_at: None,
        remaining: None,
    })]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, true, false).await;
    let first = w.status.rate_limited_until.unwrap() - NOW.unix_timestamp();
    assert_eq!(first, 60);
    w.cycle(NOW + Duration::seconds(61), true, false).await;
    let second =
        w.status.rate_limited_until.unwrap() - (NOW + Duration::seconds(61)).unix_timestamp();
    assert_eq!(second, 120);
}

#[test]
fn snapshot_store_retains_only_configured_repos() {
    use vigia_lib::poller::{RepoSnapshot, SnapshotFlags, SnapshotStore};
    let store = SnapshotStore::default();
    let r = RepoRuntime::new(watched(1, "a"), NOW);
    for id in [1, 2] {
        store.set_repo(RepoSnapshot {
            account_id: "a".into(),
            repo: watched(id, "a").repo,
            state: r.state.clone(),
        });
    }
    store.retain_repos(&[("a".to_string(), 2)], &["a".to_string()]);
    let snapshot = store.snapshot(NOW, false, SnapshotFlags::default());
    let ids: Vec<u64> = snapshot.repos.iter().map(|r| r.repo.id).collect();
    assert_eq!(ids, vec![2]);
}

#[test]
fn snapshot_store_drops_removed_accounts_even_with_no_repos_left() {
    use vigia_lib::poller::{AccountSnapshot, SnapshotFlags, SnapshotStore};
    let store = SnapshotStore::default();
    for id in ["kept", "removed"] {
        store.set_account(AccountSnapshot {
            id: id.into(),
            label: id.into(),
            ..Default::default()
        });
    }
    store.retain_repos(&[], &["kept".to_string()]);
    let snapshot = store.snapshot(NOW, false, SnapshotFlags::default());
    let ids: Vec<&str> = snapshot.accounts.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["kept"]);
}

fn ok_running() -> Outcome {
    Ok(vec![run(1, RunState::Running, NOW - Duration::minutes(5))])
}

/// Within the jitter of `secs`, plus up to one due-time bucket of rounding up.
fn assert_near(actual: Duration, secs: i64) {
    let spread = (secs as f64 * 0.1).max(1.0).ceil() as i64 + 1;
    let low = Duration::seconds(secs - spread);
    let high = Duration::seconds(secs + spread + DUE_BUCKET_SECS);
    assert!(
        actual >= low && actual <= high,
        "{actual} is not about {secs} s"
    );
}

/// Repos that a stopped cycle skipped were not requested and are still due.
fn assert_skipped_repos_stay_due(w: &AccountWorker, provider: &MockProvider) {
    let called = provider.calls.lock().unwrap().clone();
    for r in w
        .repos
        .iter()
        .filter(|r| !called.contains(&r.watched.repo.id))
    {
        assert!(r.next_due <= NOW, "{} is no longer due", r.watched.repo.id);
        assert_eq!(r.state.last_checked, None);
    }
}

#[tokio::test]
async fn rate_limit_mid_cycle_stops_further_requests() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    provider.script_repo(
        1,
        vec![
            Err(ProviderError::RateLimited {
                retry_after: Some(120),
                reset_at: None,
                remaining: None,
            }),
            Ok(vec![]),
        ],
    );
    let mut w = worker(AccountKind::GitHub, provider.clone(), 10);
    let report = w.cycle(NOW, false, false).await;
    assert_eq!(provider.calls(), 1);
    assert_eq!(report.polled, 1);
    assert_skipped_repos_stay_due(&w, &provider);
    assert_eq!(
        w.status.rate_limited_until,
        Some((NOW + Duration::minutes(2)).unix_timestamp())
    );

    // After the pause every skipped repo is polled.
    let report = w.cycle(NOW + Duration::minutes(3), false, false).await;
    assert_eq!(report.polled, 10);
}

#[tokio::test]
async fn unauthorized_mid_cycle_stops_further_requests_and_releases_the_pool() {
    let provider = MockProvider::new(vec![ok_running()]);
    provider.script_repo(1, vec![Err(ProviderError::Unauthorized)]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 10);
    let report = w.cycle(NOW, false, false).await;
    assert_eq!(provider.calls(), 1);
    assert_eq!(report.next_due, None);
    assert!(w.status.auth_error);
    assert!(w.repos.iter().all(|r| r.state.status == RepoStatus::Error));

    let mut pool = w.pool.lock().unwrap();
    assert_eq!(pool.repo_count(), 0);
    assert_eq!(pool.claim_fast_slots("other", 6), 6);
}

#[tokio::test]
async fn pause_mid_cycle_stops_further_requests() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let paused = Arc::new(AtomicBool::new(false));
    *provider.pause_on_fetch.lock().unwrap() = Some(paused.clone());
    let mut w = worker(AccountKind::GitHub, provider.clone(), 10);
    w.paused = paused;
    let report = w.cycle(NOW, false, false).await;
    assert_eq!(provider.calls(), 1);
    assert_eq!(report.polled, 1);
    assert_skipped_repos_stay_due(&w, &provider);
}

#[tokio::test]
async fn pool_pause_mid_cycle_stops_further_requests() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let reset = NOW + Duration::minutes(10);
    *provider.rate_limit.lock().unwrap() = Some(RateLimit {
        limit: 5000,
        remaining: 100,
        reset_at: reset.unix_timestamp(),
    });
    let mut w = worker(AccountKind::GitHub, provider.clone(), 10);
    w.cycle(NOW, false, false).await;
    assert_eq!(provider.calls(), 1);
    assert_skipped_repos_stay_due(&w, &provider);
    assert_eq!(w.status.rate_limited_until, Some(reset.unix_timestamp()));
}

#[tokio::test]
async fn rate_limit_keeps_the_later_pool_pause() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    provider.yield_first.store(true, Ordering::SeqCst);
    let reset = NOW + Duration::hours(1);
    *provider.rate_limit.lock().unwrap() = Some(RateLimit {
        limit: 5000,
        remaining: 100,
        reset_at: reset.unix_timestamp(),
    });
    // Repo 1 pauses the pool for an hour through its headers; repo 2, already in flight,
    // asks for one minute.
    provider.script_repo(
        2,
        vec![Err(ProviderError::RateLimited {
            retry_after: Some(60),
            reset_at: None,
            remaining: None,
        })],
    );
    let mut w = worker(AccountKind::GitHub, provider.clone(), 2);
    w.cycle(NOW, false, false).await;
    assert_eq!(provider.calls(), 2);
    assert_eq!(w.pool.lock().unwrap().paused_until, Some(reset));
    assert_eq!(w.status.rate_limited_until, Some(reset.unix_timestamp()));
}

#[tokio::test]
async fn secondary_limit_waits_60_seconds_unless_the_primary_window_is_exhausted() {
    let reset = NOW + Duration::hours(1);
    let limited = |remaining| {
        Err(ProviderError::RateLimited {
            retry_after: None,
            reset_at: Some(reset.unix_timestamp()),
            remaining: Some(remaining),
        })
    };

    let provider = MockProvider::new(vec![limited(4000)]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(
        w.status.rate_limited_until,
        Some((NOW + Duration::seconds(60)).unix_timestamp())
    );

    let provider = MockProvider::new(vec![limited(0)]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.status.rate_limited_until, Some(reset.unix_timestamp()));
}

#[tokio::test]
async fn no_fast_poll_while_the_account_is_unreachable() {
    let provider = MockProvider::new(vec![
        ok_running(),
        ok_running(),
        Err(ProviderError::Network("down".into())),
    ]);
    let mut w = worker(AccountKind::GitHub, provider, 2);
    w.cycle(NOW, false, false).await;
    assert!(w.repos.iter().all(|r| r.fast));

    let later = NOW + Duration::seconds(20);
    w.cycle(later, true, false).await;
    assert!(w.status.unreachable);
    for r in &w.repos {
        assert!(!r.fast);
        assert!(
            r.next_due - later >= Duration::seconds(50),
            "{}",
            r.next_due
        );
    }
    // The account holds no fast slots while unreachable.
    assert_eq!(w.pool.lock().unwrap().claim_fast_slots("other", 6), 6);
}

#[tokio::test]
async fn a_repo_whose_last_poll_failed_does_not_fast_poll() {
    let provider = MockProvider::new(vec![ok_running()]);
    provider.script_repo(
        2,
        vec![ok_running(), Err(ProviderError::Server { status: 502 })],
    );
    let mut w = worker(AccountKind::GitHub, provider, 2);
    w.cycle(NOW, false, false).await;
    assert!(w.repos[1].fast);

    w.cycle(NOW + Duration::seconds(20), true, false).await;
    assert!(!w.status.unreachable);
    assert!(w.repos[0].fast);
    assert!(!w.repos[1].fast);
    assert!(w.repos[1].state.stale);
}

#[tokio::test]
async fn refresh_groups_is_passed_only_when_asked() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 1);
    w.cycle(NOW, true, false).await;
    w.cycle(NOW + Duration::seconds(5), true, true).await;
    w.cycle(NOW + Duration::seconds(10), true, false).await;
    assert_eq!(
        *provider.refresh_groups.lock().unwrap(),
        vec![false, true, false]
    );
}

#[test]
fn repo_backoff_doubles_from_base_up_to_the_cap() {
    assert_eq!(repo_backoff_secs(60, 0), 60);
    assert_eq!(repo_backoff_secs(60, 1), 60);
    assert_eq!(repo_backoff_secs(60, 2), 120);
    assert_eq!(repo_backoff_secs(60, 3), 240);
    assert_eq!(repo_backoff_secs(60, 4), BACKOFF_MAX_SECS);
    assert_eq!(repo_backoff_secs(60, 40), BACKOFF_MAX_SECS);
    // A base above the cap is never shortened.
    assert_eq!(repo_backoff_secs(600, 3), 600);
}

#[tokio::test]
async fn failing_repo_backs_off_while_others_keep_the_base_interval() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    provider.script_repo(1, vec![Err(ProviderError::Network("timeout".into()))]);
    let mut w = worker(AccountKind::GitHub, provider, 2);

    let mut at = NOW;
    for expected in [60, 120, 240, 300] {
        w.cycle(at, true, false).await;
        assert!(!w.status.unreachable);
        assert_near(w.repos[0].next_due - at, expected);
        assert_near(w.repos[1].next_due - at, 60);
        at += Duration::minutes(10);
    }
}

#[tokio::test]
async fn gitlab_forbidden_without_membership_is_not_found() {
    let provider = MockProvider::new(vec![Err(ProviderError::Forbidden)]);
    *provider.member.lock().unwrap() = Ok(false);
    let mut w = worker(AccountKind::GitLab, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    assert_eq!(
        w.repos[0].state.note.as_deref(),
        Some("Not found, or the token can't access it")
    );
}

#[tokio::test]
async fn unreachable_interval_uses_the_stretched_base() {
    let provider = MockProvider::new(vec![Err(ProviderError::Network("down".into()))]);
    // 40 GitLab repos stretch the base interval to 180 s.
    let mut w = worker(AccountKind::GitLab, provider, 40);
    report_gitlab_limit(&w);
    w.mark_wake(NOW - Duration::minutes(1));
    w.cycle(NOW, false, false).await;
    assert!(w.status.unreachable);
    assert_eq!(w.status.effective_interval_secs, 180);
    assert_eq!(w.status.configured_interval_secs, 60);
    for r in &w.repos {
        assert!(r.next_due - NOW >= Duration::seconds(160), "{}", r.next_due);
    }
}

#[tokio::test]
async fn tag_runs_follow_the_repo_then_account_then_global_choice() {
    let tag_run = Run {
        branch: "1.0.0".into(),
        tag: true,
        ..run(1, RunState::Failed, NOW)
    };
    let cases = [
        (false, None, None, false),
        (true, None, None, true),
        (false, Some(true), None, true),
        (true, Some(false), None, false),
        (false, None, Some(true), true),
        (true, None, Some(false), false),
        (false, Some(false), Some(true), true),
        (true, Some(true), Some(false), false),
    ];
    for (global, org, choice, expected) in cases {
        let provider = MockProvider::new(vec![Ok(vec![tag_run.clone()])]);
        let settings = Settings {
            include_tags: global,
            ..Default::default()
        };
        let organizations = vec![acme_org(FilterSet {
            include_tags: org,
            ..Default::default()
        })];
        let watched = WatchedRepo {
            include_tags: choice,
            ..watched(1, "a")
        };
        let github = Account::new(AccountKind::GitHub, "acme", None);
        let filters = FilterCompiler::new(&settings, &organizations, &github).for_repo(&watched);
        let mut repo = RepoRuntime::with_filters(watched, filters, NOW);
        let result = poll_repo(
            provider.as_ref(),
            AccountKind::GitHub,
            &mut repo,
            &settings,
            NOW,
            false,
            false,
        )
        .await;
        let case = format!("global {global}, org {org:?}, repo {choice:?}");
        assert_eq!(
            *provider.include_tags.lock().unwrap(),
            vec![expected],
            "{case}"
        );
        let status = if expected {
            RepoStatus::Failed
        } else {
            RepoStatus::None
        };
        assert_eq!(result.state.status, status, "{case}");
    }
}

/// The `acme` organization on github.com with `filters`.
fn acme_org(filters: FilterSet) -> OrgFilters {
    OrgFilters {
        host: "github.com".into(),
        owner: "acme".into(),
        filters,
    }
}

/// A worker for `account` watching repos 1 and 2, the second adjusted by `edit`.
fn account_worker(
    account: &Account,
    provider: Arc<MockProvider>,
    pool: Arc<Mutex<PoolState>>,
    organizations: &[OrgFilters],
    edit: impl Fn(&mut WatchedRepo),
    now: OffsetDateTime,
) -> AccountWorker {
    let mut second = watched(2, &account.id);
    edit(&mut second);
    AccountWorker::with_organizations(
        account.clone(),
        provider,
        pool,
        vec![watched(1, &account.id), second],
        Settings {
            branch_patterns: vec!["develop".into()],
            ..Default::default()
        },
        Arc::new(organizations.to_vec()),
        now,
    )
}

#[tokio::test]
async fn the_fetch_uses_the_effective_branch_filter() {
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success, NOW)])]);
    let account = Account::new(AccountKind::GitHub, "acme", None);
    let organizations = vec![acme_org(FilterSet {
        branch_patterns: Some(vec!["release/*".into()]),
        ..Default::default()
    })];
    let pool = Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub)));
    let mut w = account_worker(
        &account,
        provider.clone(),
        pool,
        &organizations,
        |r| r.branch_patterns = Some(vec![]),
        NOW,
    );
    w.cycle(NOW, false, false).await;
    let mut seen = provider.branch_patterns.lock().unwrap().clone();
    seen.sort();
    // Repo 1 inherits the organization patterns; repo 2's empty override means the default
    // branch.
    assert_eq!(seen, vec![vec![], vec!["release/*".to_string()]]);
}

#[tokio::test]
async fn an_org_filter_change_refetches_only_repos_whose_effective_values_change() {
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Failed, NOW)])]);
    let account = Account::new(AccountKind::GitHub, "acme", None);
    let pool = Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub)));
    // Repo 2 ignores nothing on its own, so an organization ignore list does not reach it.
    let keep_all = |r: &mut WatchedRepo| r.ignored_workflows = Some(vec![]);
    let mut old = account_worker(&account, provider.clone(), pool.clone(), &[], keep_all, NOW);
    old.cycle(NOW, false, false).await;
    assert_eq!(provider.calls(), 2);

    let changed = vec![acme_org(FilterSet {
        ignored_workflows: Some(vec!["Nightly*".into()]),
        ..Default::default()
    })];
    let later = NOW + Duration::seconds(10);
    let mut new = account_worker(&account, provider.clone(), pool, &changed, keep_all, later);
    new.adopt(old, later);
    let report = new.cycle(later, false, false).await;
    assert_eq!(report.polled, 1);
    let calls = provider.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[2], 1);
}

#[tokio::test]
async fn another_orgs_filter_change_refetches_nothing() {
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success, NOW)])]);
    let account = Account::new(AccountKind::GitHub, "acme", None);
    let pool = Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub)));
    let mut old = account_worker(&account, provider.clone(), pool.clone(), &[], |_| {}, NOW);
    old.cycle(NOW, false, false).await;
    assert_eq!(provider.calls(), 2);

    let other = vec![OrgFilters {
        host: "github.com".into(),
        owner: "example".into(),
        filters: FilterSet {
            include_tags: Some(true),
            ..Default::default()
        },
    }];
    let later = NOW + Duration::seconds(10);
    let mut new = account_worker(&account, provider.clone(), pool, &other, |_| {}, later);
    new.adopt(old, later);
    let report = new.cycle(later, false, false).await;
    assert_eq!(report.polled, 0);
    assert_eq!(provider.calls(), 2);
}

#[tokio::test]
async fn a_repo_moved_to_another_org_takes_that_orgs_filters() {
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Success, NOW)])]);
    let account = Account::new(AccountKind::GitHub, "acme", None);
    let organizations = vec![OrgFilters {
        host: "github.com".into(),
        owner: "Example".into(),
        filters: FilterSet {
            branch_patterns: Some(vec!["release/*".into()]),
            ..Default::default()
        },
    }];
    let pool = Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub)));
    let mut w = account_worker(
        &account,
        provider.clone(),
        pool,
        &organizations,
        |_| {},
        NOW,
    );
    w.cycle(NOW, false, false).await;
    assert_eq!(
        w.repos[1]
            .filters
            .branch
            .as_ref()
            .as_ref()
            .unwrap()
            .patterns(),
        ["develop"]
    );

    let mut moved = w.repos[1].watched.repo.clone();
    moved.full_name = "example/r2".into();
    let later = NOW + Duration::seconds(10);
    let listed = vec![w.repos[0].watched.repo.clone(), moved];
    let update = w.apply_repo_list(&listed, later);
    assert_eq!(update.reselected, vec![2]);
    assert_eq!(
        w.repos[1]
            .filters
            .branch
            .as_ref()
            .as_ref()
            .unwrap()
            .patterns(),
        ["release/*"]
    );
    assert!(!w.repos[1].primed);
    assert_eq!(w.repos[1].next_due, later);

    // A rename within the same organization keeps the selection and the cache.
    let mut renamed = w.repos[0].watched.repo.clone();
    renamed.full_name = "acme/renamed".into();
    let update = w.apply_repo_list(&[renamed], later);
    assert_eq!(update.changed.len(), 1);
    assert!(update.reselected.is_empty());
    assert!(w.repos[0].primed);
}

#[tokio::test]
async fn accounts_on_the_same_org_share_its_filters() {
    let organizations = vec![acme_org(FilterSet {
        branch_patterns: Some(vec!["release/*".into()]),
        ..Default::default()
    })];
    for label in ["first", "second"] {
        let provider = MockProvider::new(vec![Ok(vec![])]);
        let account = Account::new(AccountKind::GitHub, label, None);
        let pool = Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub)));
        let mut w = account_worker(
            &account,
            provider.clone(),
            pool,
            &organizations,
            |_| {},
            NOW,
        );
        w.cycle(NOW, false, false).await;
        let seen = provider.branch_patterns.lock().unwrap().clone();
        assert_eq!(seen, vec![vec!["release/*".to_string()]; 2], "{label}");
    }
}

#[test]
fn an_empty_snapshot_is_idle_or_paused() {
    use vigia_lib::poller::Snapshot;
    let idle = Snapshot::empty(NOW, false);
    assert!(!idle.paused);
    assert!(idle.accounts.is_empty() && idle.repos.is_empty());
    assert!(!idle.secrets_blocked && !idle.config_read_only);
    assert!(idle.config_error.is_none());
    let paused = Snapshot::empty(NOW, true);
    assert!(paused.paused);
    assert_ne!(idle.tooltip, paused.tooltip);
}

#[test]
fn controls_flip_their_flags() {
    use vigia_lib::poller::Controls;
    let controls = Controls::default();
    assert!(!controls.is_paused());
    controls.set_paused(true);
    assert!(controls.is_paused());
    controls.set_paused(false);
    assert!(!controls.is_paused());

    assert!(!controls.refresh_requested.load(Ordering::SeqCst));
    controls.refresh_now();
    assert!(controls.refresh_requested.load(Ordering::SeqCst));

    assert!(!controls.shutdown.load(Ordering::SeqCst));
    controls.shutdown();
    assert!(controls.shutdown.load(Ordering::SeqCst));
}

#[tokio::test]
async fn controls_wake_a_waiting_task() {
    use vigia_lib::poller::Controls;
    let controls = Controls::default();
    let wake = controls.wake.clone();
    let waiter = tokio::spawn(async move { wake.notified().await });
    // The waiter must be registered before the notification, or the wake-up is lost.
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    controls.refresh_now();
    tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn a_paused_snapshot_marks_every_repo_stale() {
    use vigia_lib::poller::{RepoSnapshot, SnapshotFlags, SnapshotStore};
    let store = SnapshotStore::default();
    let r = RepoRuntime::new(watched(1, "a"), NOW);
    store.set_repo(RepoSnapshot {
        account_id: "a".into(),
        repo: watched(1, "a").repo,
        state: r.state.clone(),
    });
    let live = store.snapshot(NOW, false, SnapshotFlags::default());
    assert!(!live.repos[0].state.stale);
    let paused = store.snapshot(NOW, true, SnapshotFlags::default());
    assert!(paused.paused);
    assert!(paused.repos[0].state.stale);
}

#[test]
fn snapshot_store_sorts_by_name_and_carries_flags() {
    use vigia_lib::poller::{AccountSnapshot, RepoSnapshot, SnapshotFlags, SnapshotStore};
    let store = SnapshotStore::default();
    for label in ["zeta", "alpha"] {
        store.set_account(AccountSnapshot {
            id: label.into(),
            label: label.into(),
            ..Default::default()
        });
    }
    let r = RepoRuntime::new(watched(1, "alpha"), NOW);
    for id in [2, 1] {
        store.set_repo(RepoSnapshot {
            account_id: "alpha".into(),
            repo: watched(id, "alpha").repo,
            state: r.state.clone(),
        });
    }
    let snapshot = store.snapshot(
        NOW,
        false,
        vigia_lib::poller::SnapshotFlags {
            secrets_blocked: true,
            config_read_only: true,
            config_error: Some("broken".into()),
            update: None,
            ..Default::default()
        },
    );
    let labels: Vec<&str> = snapshot.accounts.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(labels, vec!["alpha", "zeta"]);
    let names: Vec<&str> = snapshot
        .repos
        .iter()
        .map(|r| r.repo.full_name.as_str())
        .collect();
    assert_eq!(names, vec!["acme/r1", "acme/r2"]);
    assert!(snapshot.secrets_blocked && snapshot.config_read_only);
    assert_eq!(snapshot.config_error.as_deref(), Some("broken"));
    store.remove_account("alpha");
    let after = store.snapshot(NOW, false, SnapshotFlags::default());
    assert_eq!(after.accounts.len(), 1);
    assert!(after.repos.is_empty());
}

#[tokio::test]
async fn repo_snapshots_report_each_repo_with_its_account() {
    let provider = MockProvider::new(vec![Ok(vec![run(
        1,
        RunState::Failed,
        NOW - Duration::hours(2),
    )])]);
    let mut w = worker(AccountKind::GitHub, provider, 2);
    w.cycle(NOW, false, false).await;
    let snapshots = w.repo_snapshots();
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].account_id, w.account.id);
    assert_eq!(snapshots[1].repo.id, 2);
    assert_eq!(snapshots[0].state.status, RepoStatus::Failed);
}

#[tokio::test]
async fn gitlab_membership_check_failure_is_handled_like_any_poll_error() {
    let provider = MockProvider::new(vec![Err(ProviderError::Forbidden)]);
    *provider.member.lock().unwrap() = Err(ProviderError::Network("down".into()));
    let mut w = worker(AccountKind::GitLab, provider, 1);
    w.cycle(NOW, false, false).await;
    assert!(w.repos[0].state.stale);
    assert!(w.repos[0].last_failed);
}

#[tokio::test]
async fn redirect_and_decode_errors_become_repo_errors() {
    let provider = MockProvider::new(vec![Err(ProviderError::Redirected {
        location: "https://other.test/".into(),
    })]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    let redirected = w.repos[0].state.note.clone();

    *provider.script.lock().unwrap() =
        VecDeque::from(vec![Err(ProviderError::Decode("bad".into()))]);
    w.cycle(NOW + Duration::hours(1), true, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    assert_ne!(w.repos[0].state.note, redirected);
}

struct PanickingProvider;

#[async_trait]
impl Provider for PanickingProvider {
    async fn validate(&self) -> Result<AccountIdentity, ProviderError> {
        unreachable!()
    }

    async fn list_repos(
        &self,
        _on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Vec<RepoInfo>, ProviderError> {
        unreachable!()
    }

    async fn is_member(&self, _repo: &RepoInfo) -> Result<bool, ProviderError> {
        unreachable!()
    }

    async fn fetch(
        &self,
        _request: FetchRequest<'_>,
        _cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError> {
        panic!("provider bug");
    }
}

#[tokio::test]
async fn a_panicking_poll_marks_only_that_repo_as_an_internal_error() {
    let account = Account::new(AccountKind::GitHub, "test", None);
    let mut w = AccountWorker::new(
        account.clone(),
        Arc::new(PanickingProvider),
        Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub))),
        vec![watched(1, &account.id)],
        Settings::default(),
        NOW,
    );
    w.cycle(NOW, false, false).await;
    assert_eq!(w.repos[0].state.status, RepoStatus::Error);
    assert!(w.repos[0].last_failed);
    assert!(w.repos[0].state.note.is_some());
}

fn rate_limited(
    retry_after: Option<u64>,
    reset_at: Option<i64>,
    remaining: Option<u64>,
) -> Outcome {
    Err(ProviderError::RateLimited {
        retry_after,
        reset_at,
        remaining,
    })
}

fn paused_until(w: &AccountWorker) -> OffsetDateTime {
    w.pool.lock().unwrap().paused_until.unwrap()
}

#[tokio::test]
async fn a_huge_retry_after_is_clamped_to_an_hour_and_zero_to_a_second() {
    let provider = MockProvider::new(vec![rate_limited(Some(u64::MAX), None, None)]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(paused_until(&w), NOW + Duration::hours(1));

    let provider = MockProvider::new(vec![rate_limited(Some(0), None, None)]);
    let mut w = worker(AccountKind::GitHub, provider, 1);
    w.cycle(NOW, false, false).await;
    assert_eq!(paused_until(&w), NOW + Duration::seconds(1));
}

#[tokio::test]
async fn a_far_or_invalid_reset_time_waits_at_most_an_hour() {
    for reset_at in [i64::MAX, i64::MIN, NOW.unix_timestamp() + 7 * 86_400] {
        let provider = MockProvider::new(vec![rate_limited(None, Some(reset_at), Some(0))]);
        let mut w = worker(AccountKind::GitHub, provider, 1);
        w.cycle(NOW, false, false).await;
        let until = paused_until(&w);
        assert!(
            until >= NOW && until <= NOW + Duration::hours(1),
            "{reset_at}: {until}"
        );
    }
}

#[tokio::test]
async fn a_huge_poll_interval_is_scheduled_at_most_an_hour_ahead() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let account = Account::new(AccountKind::GitHub, "test", None);
    let settings = Settings {
        poll_interval_secs: u64::MAX,
        ..Default::default()
    };
    let mut w = AccountWorker::new(
        account.clone(),
        provider,
        Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub))),
        vec![watched(1, &account.id)],
        settings,
        NOW,
    );
    let report = w.cycle(NOW, false, false).await;
    let next = report.next_due.unwrap();
    assert!(next > NOW && next <= NOW + Duration::hours(1), "{next}");
    assert_eq!(w.status.configured_interval_secs, 3600);
}

#[test]
fn due_times_round_up_to_bucket_boundaries_without_overflowing() {
    assert_eq!(due_after(NOW, 60), NOW + Duration::seconds(60));
    assert_eq!(due_after(NOW, 61), NOW + Duration::seconds(65));
    let odd = NOW + Duration::milliseconds(1);
    assert_eq!(due_after(odd, 0), NOW + Duration::seconds(5));
    assert_eq!(due_after(NOW, u64::MAX), NOW + Duration::hours(1));
}

#[tokio::test]
async fn the_cold_start_burst_does_not_stretch_the_interval() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let mut w = worker(AccountKind::GitHub, provider, 80);
    // The first poll of 80 repos fills empty caches: counted, but not steady polling.
    w.cycle(NOW, false, false).await;
    assert_eq!(w.status.effective_interval_secs, 60);
    assert_eq!(w.pool.lock().unwrap().requests_last_hour(NOW), 80);
    assert!(w.repos.iter().all(|r| r.primed));

    // The same 80 counted requests from primed repos are steady polling and stretch it.
    let later = NOW + Duration::seconds(60);
    w.cycle(later, true, false).await;
    assert_eq!(w.status.effective_interval_secs, 144);
}

#[tokio::test]
async fn fast_poll_requests_do_not_count_as_base_polling() {
    let provider = MockProvider::new(vec![ok_running()]);
    let mut w = worker(AccountKind::GitHub, provider.clone(), 1);
    w.cycle(NOW, false, false).await;
    assert!(w.repos[0].fast);
    let mut at = NOW;
    for _ in 0..10 {
        at = w.repos[0].next_due;
        w.cycle(at, false, false).await;
    }
    assert_eq!(provider.calls(), 11);
    let pool = w.pool.lock().unwrap();
    assert_eq!(pool.observed_per_hour(at, 60), 0.0);
    assert_eq!(pool.requests_last_hour(at), 11);
}

#[tokio::test]
async fn filters_are_compiled_once_and_shared_by_repos_that_inherit_them() {
    let provider = MockProvider::new(vec![Ok(vec![])]);
    let account = Account::new(AccountKind::GitHub, "test", None);
    let settings = Settings {
        branch_patterns: vec!["main".into(), "release/*".into()],
        ignored_workflows: vec!["nightly*".into()],
        ..Default::default()
    };
    let mut own = watched(3, &account.id);
    own.branch_patterns = Some(vec!["dev".into()]);
    let mut w = AccountWorker::new(
        account.clone(),
        provider,
        Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub))),
        vec![watched(1, &account.id), watched(2, &account.id), own],
        settings,
        NOW,
    );
    let shared = Arc::clone(&w.repos[0].filters.branch);
    assert!(Arc::ptr_eq(&shared, &w.repos[1].filters.branch));
    assert!(!Arc::ptr_eq(&shared, &w.repos[2].filters.branch));
    assert!(Arc::ptr_eq(
        &w.repos[0].filters.workflow,
        &w.repos[2].filters.workflow
    ));

    w.cycle(NOW, false, false).await;
    w.cycle(NOW + Duration::minutes(5), true, false).await;
    assert!(Arc::ptr_eq(&shared, &w.repos[0].filters.branch));
    assert!(Arc::ptr_eq(&shared, &w.repos[1].filters.branch));
}

#[tokio::test]
async fn adopt_keeps_poll_state_and_refetches_only_reselected_repos() {
    let provider = MockProvider::new(vec![Ok(vec![run(1, RunState::Failed, NOW)])]);
    let account = Account::new(AccountKind::GitHub, "test", None);
    let pool = Arc::new(Mutex::new(PoolState::new(PoolKind::GitHub)));
    let repos = vec![watched(1, &account.id), watched(2, &account.id)];
    let mut old = AccountWorker::new(
        account.clone(),
        provider.clone(),
        pool.clone(),
        repos.clone(),
        Settings::default(),
        NOW,
    );
    old.cycle(NOW, false, false).await;
    old.repos[0].cache.runs_etag = Some("\"etag-1\"".into());
    old.repos[1].cache.runs_etag = Some("\"etag-2\"".into());
    let due = old.repos[0].next_due;

    let mut changed = repos;
    changed[1].ignored_workflows = Some(vec!["CI".into()]);
    let later = NOW + Duration::seconds(10);
    let mut new = AccountWorker::new(account, provider, pool, changed, Settings::default(), later);
    new.adopt(old, later);

    let kept = &new.repos[0];
    assert_eq!(kept.state.status, RepoStatus::Failed);
    assert_eq!(kept.cache.runs_etag.as_deref(), Some("\"etag-1\""));
    assert_eq!(kept.next_due, due);
    assert!(kept.primed);

    let reselected = &new.repos[1];
    assert_eq!(reselected.state.status, RepoStatus::Failed);
    assert_eq!(reselected.cache.runs_etag, None);
    assert!(reselected.cache.runs.is_empty());
    assert_eq!(reselected.next_due, later);
    assert!(!reselected.primed);
}

/// Answers every fetch with a 403 and membership from a cache, without a request.
struct CachedMemberProvider;

#[async_trait]
impl Provider for CachedMemberProvider {
    async fn validate(&self) -> Result<AccountIdentity, ProviderError> {
        unreachable!()
    }

    async fn list_repos(
        &self,
        _on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Vec<RepoInfo>, ProviderError> {
        unreachable!()
    }

    async fn membership(&self, _repo: &RepoInfo) -> Result<Membership, ProviderError> {
        Ok(Membership {
            member: true,
            requests: 0,
        })
    }

    async fn fetch(
        &self,
        _request: FetchRequest<'_>,
        _cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError> {
        Err(ProviderError::Forbidden)
    }
}

#[tokio::test]
async fn a_cached_membership_answer_counts_no_request() {
    let mut repo = RepoRuntime::new(watched(1, "a"), NOW);
    let result = poll_repo(
        &CachedMemberProvider,
        AccountKind::GitLab,
        &mut repo,
        &Settings::default(),
        NOW,
        false,
        false,
    )
    .await;
    assert_eq!(
        result.state.note.as_deref(),
        Some(vigia_lib::notes::CI_DISABLED)
    );
    assert_eq!(result.counted, 1);

    // Without an override, a membership answer costs one request.
    let provider = MockProvider::new(vec![Err(ProviderError::Forbidden)]);
    let result = poll_repo(
        provider.as_ref(),
        AccountKind::GitLab,
        &mut repo,
        &Settings::default(),
        NOW,
        false,
        false,
    )
    .await;
    assert_eq!(result.counted, 2);
}

#[test]
fn auth_reason_serializes_as_snake_case_or_null() {
    use vigia_lib::poller::AccountSnapshot;
    let json = |reason| {
        let account = AccountSnapshot {
            auth_reason: reason,
            ..Default::default()
        };
        serde_json::to_value(account).unwrap()["auth_reason"].clone()
    };
    assert_eq!(json(None), serde_json::Value::Null);
    assert_eq!(json(Some(AuthReason::Rejected)), "rejected");
    assert_eq!(json(Some(AuthReason::MissingToken)), "missing_token");
    assert_eq!(json(Some(AuthReason::Keychain)), "keychain");
}
