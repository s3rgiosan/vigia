//! Rate limit pools. One GitHub user, or one GitLab account, shares one pool across account tasks.

use std::collections::{HashMap, VecDeque};

use time::{Duration, OffsetDateTime};

use crate::http::RateLimit;

pub const DEFAULT_GITHUB_LIMIT: u64 = 5000;
pub const DEFAULT_GITLAB_LIMIT: u64 = 2000;

/// Share of the hourly limit that base polling may use.
pub const BASE_SHARE: f64 = 0.4;
/// Share of the hourly limit that fast polling may use.
pub const FAST_SHARE: f64 = 0.3;
/// The pool pauses when the server-reported remaining count drops below this share.
pub const PAUSE_SHARE: f64 = 0.15;

pub const FAST_POLL_SECS: u64 = 15;
pub const FAST_POLL_MAX_REPOS: usize = 10;

/// The longest a stretched interval or a rate limit pause lasts, whatever the server reports.
pub const MAX_WAIT_SECS: u64 = 3600;

/// Stretches the configured interval so that `projected_per_hour` requests fit in `budget`,
/// up to `MAX_WAIT_SECS`.
pub fn stretch_interval(
    configured_secs: u64,
    projected_per_hour: f64,
    budget_per_hour: f64,
) -> u64 {
    if budget_per_hour <= 0.0 || projected_per_hour <= budget_per_hour {
        return configured_secs;
    }
    let stretched = configured_secs as f64 * projected_per_hour / budget_per_hour;
    (stretched.ceil() as u64).min(MAX_WAIT_SECS.max(configured_secs))
}

/// `now` plus `secs`, capped at `MAX_WAIT_SECS` and at the largest representable time.
pub fn wait_until(now: OffsetDateTime, secs: u64) -> OffsetDateTime {
    let secs = secs.min(MAX_WAIT_SECS) as i64;
    now.checked_add(Duration::seconds(secs)).unwrap_or(now)
}

/// Requests per hour that `repos` repos make at `interval_secs`.
pub fn projected_per_hour(repos: usize, interval_secs: u64) -> f64 {
    repos as f64 * 3600.0 / interval_secs.max(1) as f64
}

/// How many repos a pool may fast-poll at once.
pub fn fast_poll_cap(limit: u64) -> usize {
    let cap = (FAST_SHARE * limit as f64 * FAST_POLL_SECS as f64 / 3600.0).floor() as usize;
    cap.min(FAST_POLL_MAX_REPOS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolKind {
    /// 304 responses are free, so the observed count of counted responses drives the interval.
    GitHub,
    /// Every request counts, so the projected count drives the interval once the server has
    /// reported a limit. An instance that sends no `RateLimit` headers may have no limit at all,
    /// so base polling is not stretched from the assumed one; a 429 still pauses the pool.
    GitLab,
}

/// Why a request was sent, which decides whether it weighs on the base interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestClass {
    /// Steady polling at the base interval. Only these stretch the base interval.
    Base,
    /// Polling of a repo with a moving run, bounded by the fast-poll slots.
    Fast,
    /// Requests that fill an empty cache: a repo's first fetch, workflow and tag lists, and
    /// repo lists.
    Fill,
}

#[derive(Debug, Clone)]
pub struct PoolState {
    pub kind: PoolKind,
    pub limit: u64,
    pub last_rate_limit: Option<RateLimit>,
    /// Counted base polling requests in the last hour.
    observed: VecDeque<(OffsetDateTime, u32)>,
    /// Counted fast polling and cache filling requests in the last hour.
    other: VecDeque<(OffsetDateTime, u32)>,
    /// Repos per account in this pool.
    repo_counts: HashMap<String, usize>,
    /// Fast-poll slots held per account.
    fast_slots: HashMap<String, usize>,
    pub paused_until: Option<OffsetDateTime>,
    /// Interval the pool last computed, used to normalize observed counts back to the configured rate.
    last_effective_secs: Option<u64>,
}

impl PoolState {
    pub fn new(kind: PoolKind) -> PoolState {
        let limit = match kind {
            PoolKind::GitHub => DEFAULT_GITHUB_LIMIT,
            PoolKind::GitLab => DEFAULT_GITLAB_LIMIT,
        };
        PoolState {
            kind,
            limit,
            last_rate_limit: None,
            observed: VecDeque::new(),
            other: VecDeque::new(),
            repo_counts: HashMap::new(),
            fast_slots: HashMap::new(),
            paused_until: None,
            last_effective_secs: None,
        }
    }

    pub fn set_repo_count(&mut self, account_id: &str, count: usize) {
        self.repo_counts.insert(account_id.to_string(), count);
    }

    pub fn repo_count(&self) -> usize {
        self.repo_counts.values().sum()
    }

    /// Records a response: its counted requests and the limit headers. A reported reset time
    /// pauses the pool for at most `MAX_WAIT_SECS`.
    pub fn record(
        &mut self,
        now: OffsetDateTime,
        class: RequestClass,
        counted: u32,
        rate_limit: Option<RateLimit>,
    ) {
        if counted > 0 {
            let window = match class {
                RequestClass::Base => &mut self.observed,
                RequestClass::Fast | RequestClass::Fill => &mut self.other,
            };
            window.push_back((now, counted));
        }
        self.prune(now);
        if let Some(rl) = rate_limit {
            self.last_rate_limit = Some(rl);
            if rl.limit > 0 {
                self.limit = rl.limit;
            }
            let floor = (PAUSE_SHARE * self.limit as f64) as u64;
            if rl.remaining < floor {
                self.paused_until = Some(clamp_reset(now, rl.reset_at));
            }
        }
    }

    /// Counted requests of every class in the last hour.
    pub fn requests_last_hour(&self, now: OffsetDateTime) -> u32 {
        let cutoff = now - Duration::hours(1);
        self.observed
            .iter()
            .chain(self.other.iter())
            .filter(|(at, _)| *at >= cutoff)
            .map(|(_, n)| n)
            .sum()
    }

    pub fn is_paused(&self, now: OffsetDateTime) -> bool {
        self.paused_until.is_some_and(|until| until > now)
    }

    fn prune(&mut self, now: OffsetDateTime) {
        let cutoff = now - Duration::hours(1);
        for window in [&mut self.observed, &mut self.other] {
            while window.front().is_some_and(|(at, _)| *at < cutoff) {
                window.pop_front();
            }
        }
    }

    /// Counted base polling requests observed in the last hour, scaled to a full hour.
    ///
    /// The span between the first and last observation misses the interval that produced the
    /// last batch, so one `interval_secs` is added to it. A single startup burst then reads as
    /// one interval's worth of requests, not as an infinite rate.
    pub fn observed_per_hour(&self, now: OffsetDateTime, interval_secs: u64) -> f64 {
        let Some((first, _)) = self.observed.front() else {
            return 0.0;
        };
        let total: u32 = self.observed.iter().map(|(_, n)| n).sum();
        let span = (now - *first).as_seconds_f64().max(0.0);
        let window = (span + interval_secs.max(1) as f64).min(3600.0);
        total as f64 * 3600.0 / window
    }

    /// Effective base interval for this pool.
    ///
    /// Observed GitHub counts were measured at the previous effective interval, so they are
    /// scaled back to what they would be at the configured interval before stretching.
    pub fn effective_interval(&mut self, configured_secs: u64, now: OffsetDateTime) -> u64 {
        let budget = BASE_SHARE * self.limit as f64;
        let projected = match self.kind {
            PoolKind::GitHub => {
                let previous = self.last_effective_secs.unwrap_or(configured_secs).max(1);
                self.observed_per_hour(now, previous) * previous as f64
                    / configured_secs.max(1) as f64
            }
            PoolKind::GitLab if self.last_rate_limit.is_some() => {
                projected_per_hour(self.repo_count(), configured_secs)
            }
            PoolKind::GitLab => 0.0,
        };
        let effective = stretch_interval(configured_secs, projected, budget);
        self.last_effective_secs = Some(effective);
        effective
    }

    /// Claims up to `wanted` fast-poll slots for an account and returns how many it got.
    pub fn claim_fast_slots(&mut self, account_id: &str, wanted: usize) -> usize {
        let cap = fast_poll_cap(self.limit);
        let used_by_others: usize = self
            .fast_slots
            .iter()
            .filter(|(id, _)| id.as_str() != account_id)
            .map(|(_, n)| n)
            .sum();
        let granted = wanted.min(cap.saturating_sub(used_by_others));
        self.fast_slots.insert(account_id.to_string(), granted);
        granted
    }

    /// Drops an account's repo count and fast-poll slots, so they stop weighing on the pool.
    pub fn release_account(&mut self, account_id: &str) {
        self.repo_counts.remove(account_id);
        self.fast_slots.remove(account_id);
    }

    /// Drops repo counts and fast-poll slots of every account outside `keep`.
    pub fn retain_accounts(&mut self, keep: &std::collections::HashSet<String>) {
        self.repo_counts.retain(|id, _| keep.contains(id));
        self.fast_slots.retain(|id, _| keep.contains(id));
    }
}

/// A server-reported reset time, kept between `now` and `MAX_WAIT_SECS` from now.
pub fn clamp_reset(now: OffsetDateTime, reset_at: i64) -> OffsetDateTime {
    let latest = wait_until(now, MAX_WAIT_SECS);
    OffsetDateTime::from_unix_timestamp(reset_at).map_or(latest, |reset| reset.clamp(now, latest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-06-01 12:00 UTC);

    #[test]
    fn interval_is_configured_when_within_budget() {
        // 30 repos at 60 s = 1800/h, under 40% of 5000.
        assert_eq!(stretch_interval(60, projected_per_hour(30, 60), 2000.0), 60);
    }

    #[test]
    fn interval_stretches_when_over_budget() {
        // 80 repos at 60 s = 4800/h against 2000/h → 144 s.
        assert_eq!(
            stretch_interval(60, projected_per_hour(80, 60), 2000.0),
            144
        );
    }

    #[test]
    fn fast_poll_caps() {
        assert_eq!(fast_poll_cap(5000), 6);
        assert_eq!(fast_poll_cap(2000), 2);
        assert_eq!(fast_poll_cap(100000), FAST_POLL_MAX_REPOS);
    }

    #[test]
    fn gitlab_pool_uses_projected_count_across_accounts() {
        let mut pool = PoolState::new(PoolKind::GitLab);
        pool.set_repo_count("a", 20);
        pool.set_repo_count("b", 20);
        pool.record(NOW, RequestClass::Base, 1, Some(gitlab_limit(2000)));
        // 40 repos at 60 s = 2400/h against 800/h → 180 s.
        assert_eq!(pool.effective_interval(60, NOW), 180);
    }

    #[test]
    fn github_pool_uses_observed_non_304_count() {
        let mut pool = PoolState::new(PoolKind::GitHub);
        pool.set_repo_count("a", 80);
        // Nothing observed yet: quiet repos keep the configured interval.
        assert_eq!(pool.effective_interval(60, NOW), 60);

        // 80 counted requests in one cycle project to 4800/h against 2000/h.
        pool.record(NOW, RequestClass::Base, 80, None);
        assert_eq!(pool.effective_interval(60, NOW), 144);

        // Observations older than an hour fall out of the window.
        let later = NOW + Duration::hours(2);
        pool.record(later, RequestClass::Base, 0, None);
        assert_eq!(pool.effective_interval(60, later), 60);
    }

    #[test]
    fn github_stretch_is_stable_at_the_stretched_interval() {
        let mut pool = PoolState::new(PoolKind::GitHub);
        pool.set_repo_count("a", 80);
        pool.record(NOW, RequestClass::Base, 80, None);
        assert_eq!(pool.effective_interval(60, NOW), 144);
        // Polling at 144 s, 80 repos produce 80 requests per 144 s = 2000/h, which is the
        // budget at the stretched rate. The interval must hold at 144, not decay toward 60.
        let later = NOW + Duration::hours(2);
        pool.record(later - Duration::seconds(144), RequestClass::Base, 80, None);
        pool.record(later, RequestClass::Base, 80, None);
        assert_eq!(pool.effective_interval(60, later), 144);
    }

    #[test]
    fn server_limit_replaces_default_and_low_remaining_pauses() {
        let mut pool = PoolState::new(PoolKind::GitHub);
        pool.record(
            NOW,
            RequestClass::Base,
            1,
            Some(RateLimit {
                limit: 15000,
                remaining: 14000,
                reset_at: NOW.unix_timestamp() + 600,
            }),
        );
        assert_eq!(pool.limit, 15000);
        assert!(!pool.is_paused(NOW));

        pool.record(
            NOW,
            RequestClass::Base,
            1,
            Some(RateLimit {
                limit: 15000,
                remaining: 100,
                reset_at: NOW.unix_timestamp() + 600,
            }),
        );
        assert!(pool.is_paused(NOW));
        assert!(!pool.is_paused(NOW + Duration::seconds(601)));
    }

    #[test]
    fn fast_slots_are_shared_across_accounts() {
        let mut pool = PoolState::new(PoolKind::GitHub);
        assert_eq!(pool.claim_fast_slots("a", 4), 4);
        assert_eq!(pool.claim_fast_slots("b", 4), 2);
        assert_eq!(pool.claim_fast_slots("a", 1), 1);
        assert_eq!(pool.claim_fast_slots("b", 10), 5);
    }

    #[test]
    fn release_account_frees_slots_and_repo_count() {
        let mut pool = PoolState::new(PoolKind::GitLab);
        pool.set_repo_count("a", 20);
        pool.set_repo_count("b", 20);
        assert_eq!(pool.claim_fast_slots("a", 2), 2);
        assert_eq!(pool.claim_fast_slots("b", 2), 0);

        pool.release_account("a");
        assert_eq!(pool.repo_count(), 20);
        assert_eq!(pool.claim_fast_slots("b", 2), 2);
    }

    #[test]
    fn retain_accounts_drops_unlisted_accounts() {
        let mut pool = PoolState::new(PoolKind::GitLab);
        pool.set_repo_count("a", 20);
        pool.set_repo_count("b", 20);
        assert_eq!(pool.claim_fast_slots("a", 2), 2);

        let keep: std::collections::HashSet<String> = ["b".to_string()].into();
        pool.retain_accounts(&keep);
        assert_eq!(pool.repo_count(), 20);
        assert_eq!(pool.claim_fast_slots("b", 2), 2);
    }

    fn gitlab_limit(limit: u64) -> RateLimit {
        RateLimit {
            limit,
            remaining: limit,
            reset_at: NOW.unix_timestamp() + 60,
        }
    }

    #[test]
    fn gitlab_without_reported_limits_does_not_stretch_from_the_assumed_one() {
        let mut pool = PoolState::new(PoolKind::GitLab);
        pool.set_repo_count("a", 40);
        pool.record(NOW, RequestClass::Base, 40, None);
        assert_eq!(pool.effective_interval(60, NOW), 60);
        assert_eq!(fast_poll_cap(pool.limit), 2);
    }

    #[test]
    fn fill_and_fast_requests_do_not_stretch_base_polling() {
        let mut pool = PoolState::new(PoolKind::GitHub);
        pool.set_repo_count("a", 80);
        pool.record(NOW, RequestClass::Fill, 400, None);
        pool.record(NOW, RequestClass::Fast, 400, None);
        assert_eq!(pool.effective_interval(60, NOW), 60);
        assert_eq!(pool.requests_last_hour(NOW), 800);
        assert_eq!(pool.requests_last_hour(NOW + Duration::minutes(61)), 0);
    }

    #[test]
    fn a_far_reset_time_pauses_for_at_most_an_hour() {
        let mut pool = PoolState::new(PoolKind::GitHub);
        for reset_at in [i64::MAX, i64::MIN, NOW.unix_timestamp() + 86_400] {
            pool.paused_until = None;
            pool.record(
                NOW,
                RequestClass::Base,
                1,
                Some(RateLimit {
                    limit: 5000,
                    remaining: 0,
                    reset_at,
                }),
            );
            let until = pool.paused_until.unwrap();
            assert!(until >= NOW && until <= NOW + Duration::hours(1), "{until}");
        }
    }

    #[test]
    fn stretching_and_waits_stop_at_the_cap() {
        assert_eq!(stretch_interval(60, f64::INFINITY, 2000.0), MAX_WAIT_SECS);
        assert_eq!(wait_until(NOW, u64::MAX), NOW + Duration::hours(1));
        let end = time::PrimitiveDateTime::MAX.assume_utc();
        assert_eq!(wait_until(end, 60), end);
    }
}
