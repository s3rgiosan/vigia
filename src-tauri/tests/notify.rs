mod common;

use time::macros::datetime;
use time::{Duration, OffsetDateTime};
use vigia_lib::config::{
    Account, AccountKind, Config, FilterSet, OrgFilters, Settings, WatchedRepo,
};
use vigia_lib::model::{RepoState, RepoStatus, Run, RunState};
use vigia_lib::notify::{
    filter_transitions, identity_of, render, repos_to_rebaseline, ClickTarget, Notifier,
    Transition, COALESCE_ABOVE,
};
use vigia_lib::poller::AccountSnapshot;
use vigia_lib::providers::RepoInfo;

const NOW: OffsetDateTime = datetime!(2026-06-01 12:00 UTC);

fn repo() -> RepoInfo {
    common::repo_info(1, "acme/widgets", "https://github.com/acme/widgets")
}

fn repo_n(id: u64) -> RepoInfo {
    common::repo_info(
        id,
        &format!("acme/widgets-{id}"),
        &format!("https://github.com/acme/widgets-{id}"),
    )
}

fn run(id: u64, attempt: u32, branch: &str, group: &str, state: RunState) -> Run {
    common::run_in(
        id,
        attempt,
        branch,
        group,
        state,
        format!("https://github.com/acme/widgets/actions/runs/{id}"),
        NOW,
    )
}

fn state(groups: Vec<Run>) -> RepoState {
    let status = groups
        .iter()
        .map(|r| RepoStatus::from_run_state(r.state))
        .fold(
            RepoStatus::None,
            |acc, s| if s.is_worse_than(acc) { s } else { acc },
        );
    RepoState {
        status,
        representative: groups.first().cloned(),
        groups,
        last_checked: Some(NOW),
        stale: false,
        note: None,
    }
}

fn error_state() -> RepoState {
    RepoState {
        status: RepoStatus::Error,
        representative: None,
        groups: vec![],
        last_checked: Some(NOW),
        stale: false,
        note: Some("boom".into()),
    }
}

fn observe(n: &mut Notifier, s: &RepoState) -> Vec<Transition> {
    n.observe(AccountKind::GitHub, "a", &repo(), s)
}

#[test]
fn first_observation_is_a_silent_baseline() {
    let mut n = Notifier::default();
    let t = observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Failed)]),
    );
    assert!(t.is_empty());
}

#[test]
fn entering_failed_notifies_once_per_identity() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
    );
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert!(matches!(t.as_slice(), [Transition::Failed { run, .. }] if run.id == 2));
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert!(t.is_empty());
}

#[test]
fn a_new_failed_run_while_failed_notifies_again() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Failed)]),
    );
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert_eq!(t.len(), 1);
}

#[test]
fn a_failed_rerun_with_new_attempt_notifies_again() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Failed)]),
    );
    let t = observe(
        &mut n,
        &state(vec![run(1, 2, "main", "ci", RunState::Failed)]),
    );
    assert_eq!(t.len(), 1);
}

#[test]
fn gitlab_retry_identity_uses_updated_at() {
    let mut n = Notifier::default();
    let mut first = run(10, 1, "main", "push", RunState::Failed);
    first.updated_at = NOW - Duration::minutes(10);
    n.observe(
        AccountKind::GitLab,
        "a",
        &repo(),
        &state(vec![first.clone()]),
    );
    // Same pipeline, same timestamp: nothing.
    let t = n.observe(
        AccountKind::GitLab,
        "a",
        &repo(),
        &state(vec![first.clone()]),
    );
    assert!(t.is_empty());
    // Retried between polls and failed again: updated_at moved.
    let retried = run(10, 1, "main", "push", RunState::Failed);
    let t = n.observe(
        AccountKind::GitLab,
        "a",
        &repo(),
        &state(vec![retried.clone()]),
    );
    assert_eq!(t.len(), 1);
    assert_ne!(
        identity_of(AccountKind::GitLab, &first),
        identity_of(AccountKind::GitLab, &retried)
    );
}

#[test]
fn recovery_is_reported_once() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Failed)]),
    );
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Success)]),
    );
    assert!(matches!(t.as_slice(), [Transition::Recovered { .. }]));
    // Staying green is quiet.
    let t = observe(
        &mut n,
        &state(vec![run(3, 1, "main", "ci", RunState::Success)]),
    );
    assert!(t.is_empty());
}

#[test]
fn recovery_from_error_resets_the_baseline() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
    );
    assert!(observe(&mut n, &error_state()).is_empty());
    // A failure that happened while the repo was in Error does not notify.
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert!(t.is_empty());
    // The baseline now includes that failure; the next one notifies.
    let t = observe(
        &mut n,
        &state(vec![run(3, 1, "main", "ci", RunState::Failed)]),
    );
    assert_eq!(t.len(), 1);
}

#[test]
fn reset_clears_baselines() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
    );
    n.reset();
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert!(t.is_empty());
}

#[test]
fn a_new_group_in_a_baselined_repo_notifies() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
    );
    let t = observe(
        &mut n,
        &state(vec![
            run(1, 1, "main", "ci", RunState::Success),
            run(2, 1, "feature/x", "ci", RunState::Failed),
        ]),
    );
    assert!(matches!(t.as_slice(), [Transition::Failed { run, .. }] if run.branch == "feature/x"));
}

#[test]
fn auth_error_notifies_once_per_account_until_recovery() {
    let mut n = Notifier::default();
    let mut account = AccountSnapshot {
        id: "a".into(),
        label: "Work".into(),
        auth_error: true,
        ..Default::default()
    };
    let first = n
        .observe_account(&account)
        .expect("first auth error notifies");
    assert_eq!(first.title, "Work: token rejected");
    assert_eq!(first.body, "Open Settings to replace the token.");
    assert_eq!(first.target, ClickTarget::Popup);
    assert!(n.observe_account(&account).is_none());
    account.auth_error = false;
    assert!(n.observe_account(&account).is_none());
    account.auth_error = true;
    assert!(n.observe_account(&account).is_some());
}

#[test]
fn render_coalesces_bursts_into_one_summary() {
    let transitions: Vec<Transition> = (1..=COALESCE_ABOVE as u64 + 1)
        .map(|i| Transition::Failed {
            repo: repo(),
            run: run(i, 1, "main", "ci", RunState::Failed),
        })
        .collect();
    let rendered = render("a", "Work", &transitions);
    assert_eq!(rendered.len(), 1);
    // Every transition belongs to the same repository.
    assert_eq!(rendered[0].title, "Work: 1 repository failed");
    assert_eq!(rendered[0].body, "Open Vigia for the list.");
    assert_eq!(rendered[0].target, ClickTarget::Popup);

    let few = render("a", "Work", &transitions[..2]);
    assert_eq!(few.len(), 2);
    assert_eq!(few[0].title, "widgets failed");
    assert_eq!(few[0].body, "main · ci");
    assert_eq!(
        few[0].target,
        ClickTarget::Run {
            account_id: "a".into(),
            url: "https://github.com/acme/widgets/actions/runs/1".into(),
        }
    );
}

#[test]
fn coalesced_groups_do_not_repeat_next_cycle() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
    );
    let burst: Vec<Run> = (1..=5)
        .map(|i| run(100 + i, 1, &format!("b{i}"), "ci", RunState::Failed))
        .collect();
    let t = observe(&mut n, &state(burst.clone()));
    assert_eq!(t.len(), 5);
    assert_eq!(render("a", "Work", &t).len(), 1);
    let t = observe(&mut n, &state(burst));
    assert!(t.is_empty());
}

fn failed(repo: RepoInfo, id: u64) -> Transition {
    Transition::Failed {
        repo,
        run: run(id, 1, "main", "ci", RunState::Failed),
    }
}

fn recovered(repo: RepoInfo, id: u64) -> Transition {
    Transition::Recovered {
        repo,
        run: run(id, 1, "main", "ci", RunState::Success),
    }
}

#[test]
fn failed_running_failed_with_the_same_identity_does_not_renotify() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
    );
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert_eq!(t.len(), 1);
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Running)]),
    );
    assert!(t.is_empty());
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert!(t.is_empty());
}

#[test]
fn failed_then_canceled_then_success_is_a_recovery() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
    );
    let t = observe(
        &mut n,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    assert!(matches!(t.as_slice(), [Transition::Failed { .. }]));
    // A canceled run neither fails nor recovers the group.
    let t = observe(
        &mut n,
        &state(vec![run(3, 1, "main", "ci", RunState::Canceled)]),
    );
    assert!(t.is_empty());
    let t = observe(
        &mut n,
        &state(vec![run(4, 1, "main", "ci", RunState::Success)]),
    );
    assert!(matches!(t.as_slice(), [Transition::Recovered { run, .. }] if run.id == 4));
}

#[test]
fn render_summarizes_mixed_failures_and_recoveries() {
    let transitions = vec![
        failed(repo_n(1), 1),
        failed(repo_n(2), 2),
        recovered(repo_n(3), 3),
        recovered(repo_n(4), 4),
        recovered(repo_n(5), 5),
    ];
    let rendered = render("a", "Work", &transitions);
    assert_eq!(rendered.len(), 1);
    assert_eq!(
        rendered[0].title,
        "Work: 2 repositories failed, 3 repositories recovered"
    );
}

#[test]
fn render_coalesces_only_above_the_threshold() {
    let transitions: Vec<Transition> = (1..=COALESCE_ABOVE as u64 + 1)
        .map(|i| failed(repo_n(i), i))
        .collect();
    let at_threshold = render("a", "Work", &transitions[..COALESCE_ABOVE]);
    assert_eq!(at_threshold.len(), COALESCE_ABOVE);
    assert!(at_threshold.iter().all(|n| n.title.ends_with(" failed")));

    let above = render("a", "Work", &transitions);
    assert_eq!(above.len(), 1);
    assert_eq!(
        above[0].title,
        format!("Work: {} repositories failed", COALESCE_ABOVE + 1)
    );
}

#[test]
fn render_shows_the_last_segment_of_a_nested_gitlab_path() {
    let nested = RepoInfo {
        id: 7,
        full_name: "acme/platform/backend/api".into(),
        web_url: "https://gitlab.example.test/acme/platform/backend/api".into(),
        default_branch: "main".into(),
    };
    let rendered = render(
        "g",
        "GitLab",
        &[failed(nested.clone(), 1), recovered(nested, 2)],
    );
    assert_eq!(rendered[0].title, "api failed");
    assert_eq!(rendered[1].title, "api recovered");
}

#[test]
fn the_same_repo_id_under_two_accounts_keeps_separate_memory() {
    let mut n = Notifier::default();
    let green = state(vec![run(1, 1, "main", "ci", RunState::Success)]);
    let red = state(vec![run(2, 1, "main", "ci", RunState::Failed)]);
    n.observe(AccountKind::GitHub, "a", &repo(), &green);
    // Account b has no baseline yet: its first state is silent.
    assert!(n
        .observe(AccountKind::GitHub, "b", &repo(), &red)
        .is_empty());
    // Account a has a baseline and notifies for the same run.
    assert_eq!(n.observe(AccountKind::GitHub, "a", &repo(), &red).len(), 1);
    // Resetting one account's repo leaves the other alone.
    n.reset_repo("a", 1);
    let next = state(vec![run(3, 1, "main", "ci", RunState::Failed)]);
    assert!(n
        .observe(AccountKind::GitHub, "a", &repo(), &next)
        .is_empty());
    assert_eq!(n.observe(AccountKind::GitHub, "b", &repo(), &next).len(), 1);
}

fn settings(notify_failures: bool, notify_recoveries: bool) -> Settings {
    Settings {
        notify_failures,
        notify_recoveries,
        ..Default::default()
    }
}

#[test]
fn filter_transitions_follows_both_settings() {
    let all = vec![failed(repo(), 1), recovered(repo(), 2)];
    let kinds = |t: Vec<Transition>| -> Vec<&'static str> {
        t.iter()
            .map(|t| match t {
                Transition::Failed { .. } => "failed",
                Transition::Recovered { .. } => "recovered",
            })
            .collect()
    };
    assert_eq!(
        kinds(filter_transitions(all.clone(), &settings(true, true))),
        vec!["failed", "recovered"]
    );
    assert_eq!(
        kinds(filter_transitions(all.clone(), &settings(true, false))),
        vec!["failed"]
    );
    assert_eq!(
        kinds(filter_transitions(all.clone(), &settings(false, true))),
        vec!["recovered"]
    );
    assert!(filter_transitions(all, &settings(false, false)).is_empty());
}

fn watched(account: &Account, id: u64, patterns: Option<Vec<String>>) -> WatchedRepo {
    common::watched_repo(&account.id, common::acme_repo(id, "github.com"), patterns)
}

/// One account watching repos 1 and 2.
fn two_repo_config() -> (Account, Config) {
    let account = Account::new(AccountKind::GitHub, "Work", None);
    let mut config = Config::default();
    config.accounts.push(account.clone());
    config.repos.push(watched(&account, 1, None));
    config.repos.push(watched(&account, 2, None));
    (account, config)
}

/// Gives each repo of `config` a green baseline.
fn baseline_all(n: &mut Notifier, config: &Config) {
    for w in &config.repos {
        n.observe(
            AccountKind::GitHub,
            &w.account_id,
            &w.repo,
            &state(vec![run(1, 1, "main", "ci", RunState::Success)]),
        );
    }
}

/// Whether a new failure in the repo notifies, which means its baseline survived.
fn keeps_baseline(n: &mut Notifier, account: &Account, repo_id: u64) -> bool {
    let info = watched(account, repo_id, None).repo;
    let t = n.observe(
        AccountKind::GitHub,
        &account.id,
        &info,
        &state(vec![run(2, 1, "main", "ci", RunState::Failed)]),
    );
    !t.is_empty()
}

#[test]
fn renaming_an_account_or_changing_the_interval_keeps_baselines() {
    let (account, before) = two_repo_config();
    let mut n = Notifier::default();
    n.apply_config(None, &before);
    baseline_all(&mut n, &before);

    let mut after = before.clone();
    after.accounts[0].label = "Renamed".into();
    after.settings.poll_interval_secs = 300;
    after.settings.notify_recoveries = true;
    assert!(repos_to_rebaseline(&before, &after).is_empty());
    n.apply_config(Some(&before), &after);
    assert!(keeps_baseline(&mut n, &account, 1));
    assert!(keeps_baseline(&mut n, &account, 2));
}

#[test]
fn a_branch_filter_change_resets_only_that_repo() {
    let (account, before) = two_repo_config();
    let mut n = Notifier::default();
    n.apply_config(None, &before);
    baseline_all(&mut n, &before);

    let mut after = before.clone();
    after.repos[0].branch_patterns = Some(vec!["release/*".into()]);
    assert_eq!(
        repos_to_rebaseline(&before, &after),
        vec![(account.id.clone(), 1)]
    );
    n.apply_config(Some(&before), &after);
    assert!(!keeps_baseline(&mut n, &account, 1));
    assert!(keeps_baseline(&mut n, &account, 2));
}

#[test]
fn a_global_pattern_change_resets_repos_that_inherit_it() {
    let (account, mut before) = two_repo_config();
    before.repos[1].branch_patterns = Some(vec!["main".into()]);
    let mut after = before.clone();
    after.settings.branch_patterns = vec!["develop".into()];
    // Repo 2 has its own override, so the global change does not affect it.
    assert_eq!(repos_to_rebaseline(&before, &after), vec![(account.id, 1)]);
}

#[test]
fn a_repo_workflow_override_change_resets_only_that_repo() {
    let (account, before) = two_repo_config();
    let mut after = before.clone();
    after.repos[0].ignored_workflows = Some(vec!["Dependabot*".into()]);
    assert_eq!(repos_to_rebaseline(&before, &after), vec![(account.id, 1)]);
}

#[test]
fn a_global_workflow_list_change_resets_repos_that_inherit_it() {
    let (account, mut before) = two_repo_config();
    before.repos[1].ignored_workflows = Some(vec![]);
    let mut after = before.clone();
    after.settings.ignored_workflows = vec!["Dependabot*".into()];
    assert_eq!(repos_to_rebaseline(&before, &after), vec![(account.id, 1)]);
}

#[test]
fn the_exclude_pull_requests_setting_resets_every_repo() {
    let (account, before) = two_repo_config();
    let mut n = Notifier::default();
    n.apply_config(None, &before);
    baseline_all(&mut n, &before);

    let mut after = before.clone();
    after.settings.exclude_pull_requests = !before.settings.exclude_pull_requests;
    assert_eq!(repos_to_rebaseline(&before, &after).len(), 2);
    n.apply_config(Some(&before), &after);
    assert!(!keeps_baseline(&mut n, &account, 1));
    assert!(!keeps_baseline(&mut n, &account, 2));
}

#[test]
fn unwatching_one_repo_keeps_the_other_and_rewatching_starts_fresh() {
    let (account, before) = two_repo_config();
    let mut n = Notifier::default();
    n.apply_config(None, &before);
    baseline_all(&mut n, &before);

    let mut unwatched = before.clone();
    unwatched.repos.remove(0);
    n.apply_config(Some(&before), &unwatched);
    assert!(keeps_baseline(&mut n, &account, 2));

    // Watching repo 1 again is a new watch with a silent first state.
    assert_eq!(
        repos_to_rebaseline(&unwatched, &before),
        vec![(account.id.clone(), 1)]
    );
    n.apply_config(Some(&unwatched), &before);
    assert!(!keeps_baseline(&mut n, &account, 1));
}

#[test]
fn auth_memory_survives_a_restart_and_is_dropped_with_the_account() {
    let (account, before) = two_repo_config();
    let mut n = Notifier::default();
    n.apply_config(None, &before);
    let status = AccountSnapshot {
        id: account.id.clone(),
        label: account.label.clone(),
        auth_error: true,
        ..Default::default()
    };
    assert!(n.observe_account(&status).is_some());

    let mut renamed = before.clone();
    renamed.accounts[0].label = "Renamed".into();
    n.apply_config(Some(&before), &renamed);
    assert!(n.auth_notified(&account.id));
    assert!(n.observe_account(&status).is_none());

    let mut removed = renamed.clone();
    removed.remove_account(&account.id);
    n.apply_config(Some(&renamed), &removed);
    assert!(!n.auth_notified(&account.id));
}

#[test]
fn forget_auth_lets_a_replaced_token_notify_again() {
    let mut n = Notifier::default();
    let status = AccountSnapshot {
        id: "a".into(),
        label: "Work".into(),
        auth_error: true,
        ..Default::default()
    };
    assert!(n.observe_account(&status).is_some());
    n.forget_auth("a");
    assert!(n.observe_account(&status).is_some());
}

#[test]
fn a_repo_tag_choice_change_resets_only_that_repo() {
    let (account, before) = two_repo_config();
    let mut n = Notifier::default();
    n.apply_config(None, &before);
    baseline_all(&mut n, &before);

    let mut after = before.clone();
    after.repos[0].include_tags = Some(true);
    assert_eq!(
        repos_to_rebaseline(&before, &after),
        vec![(account.id.clone(), 1)]
    );
    n.apply_config(Some(&before), &after);
    assert!(!keeps_baseline(&mut n, &account, 1));
    assert!(keeps_baseline(&mut n, &account, 2));
}

#[test]
fn a_global_tag_change_resets_repos_that_inherit_it() {
    let (account, mut before) = two_repo_config();
    before.repos[1].include_tags = Some(false);
    let mut after = before.clone();
    after.settings.include_tags = true;
    assert_eq!(
        repos_to_rebaseline(&before, &after),
        vec![(account.id.clone(), 1)]
    );

    // An override that matches the new global value leaves the effective setting unchanged.
    let mut before = after.clone();
    before.repos[0].include_tags = Some(true);
    let mut after = before.clone();
    after.repos[0].include_tags = None;
    assert!(repos_to_rebaseline(&before, &after).is_empty());
}

#[test]
fn tag_runs_of_one_workflow_share_their_notification_memory() {
    let mut n = Notifier::default();
    let tagged = |id: u64, tag: &str, state: RunState| Run {
        tag: true,
        ..run(id, 1, tag, "release", state)
    };
    n.observe(
        AccountKind::GitHub,
        "a",
        &repo(),
        &state(vec![tagged(1, "1.5.0", RunState::Success)]),
    );
    let failed = n.observe(
        AccountKind::GitHub,
        "a",
        &repo(),
        &state(vec![tagged(2, "1.6.0", RunState::Failed)]),
    );
    assert_eq!(failed.len(), 1);
    let recovered = n.observe(
        AccountKind::GitHub,
        "a",
        &repo(),
        &state(vec![tagged(3, "1.6.1", RunState::Success)]),
    );
    assert!(matches!(recovered[..], [Transition::Recovered { .. }]));
}

#[test]
fn groups_that_leave_the_selection_are_forgotten() {
    let mut n = Notifier::default();
    observe(
        &mut n,
        &state(vec![
            run(1, 1, "main", "ci", RunState::Success),
            run(2, 1, "feature-a", "ci", RunState::Failed),
            run(3, 1, "feature-b", "ci", RunState::Success),
        ]),
    );
    assert_eq!(n.remembered_groups("a", 1), 3);

    // The feature branches were merged and their groups are no longer selected.
    let t = observe(
        &mut n,
        &state(vec![run(4, 1, "main", "ci", RunState::Success)]),
    );
    assert!(t.is_empty());
    assert_eq!(n.remembered_groups("a", 1), 1);
    assert_eq!(n.remembered_groups("a", 99), 0);
}

fn acme_org(filters: FilterSet) -> OrgFilters {
    OrgFilters {
        host: "github.com".into(),
        owner: "acme".into(),
        filters,
    }
}

#[test]
fn an_org_filter_change_resets_only_that_orgs_affected_repos() {
    let (account, mut before) = two_repo_config();
    // A second account watches another acme repo and a repo of another organization.
    let second = Account::new(AccountKind::GitHub, "Second", None);
    before.accounts.push(second.clone());
    before.repos.push(watched(&second, 3, None));
    let mut elsewhere = watched(&second, 4, None);
    elsewhere.repo.full_name = "example/r4".into();
    before.repos.push(elsewhere);
    before.settings.ignored_workflows = vec!["Dependabot*".into()];
    // Repo 2 keeps the global list through its own override.
    before.repos[1].ignored_workflows = Some(vec!["Dependabot*".into()]);
    let mut n = Notifier::default();
    n.apply_config(None, &before);
    baseline_all(&mut n, &before);

    let mut after = before.clone();
    after.organizations.push(acme_org(FilterSet {
        ignored_workflows: Some(vec![]),
        ..Default::default()
    }));
    assert_eq!(
        repos_to_rebaseline(&before, &after),
        vec![(account.id.clone(), 1), (second.id.clone(), 3)]
    );
    n.apply_config(Some(&before), &after);
    assert!(!keeps_baseline(&mut n, &account, 1));
    assert!(keeps_baseline(&mut n, &account, 2));
    assert!(!keeps_baseline(&mut n, &second, 3));
    assert!(keeps_baseline(&mut n, &second, 4));

    // An organization value equal to the global one changes nothing effective.
    let mut same = before.clone();
    same.organizations.push(acme_org(FilterSet {
        ignored_workflows: Some(vec!["Dependabot*".into()]),
        include_tags: Some(false),
        ..Default::default()
    }));
    assert!(repos_to_rebaseline(&before, &same).is_empty());
}
