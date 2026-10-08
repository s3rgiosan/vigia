mod common;

use std::collections::HashSet;

use time::macros::datetime;
use time::{Duration, OffsetDateTime};
use vigia_lib::aggregate::{
    overall, select_groups, Overall, Selection, STALE_BRANCH_CUTOFF, TAG_GROUP_BRANCH,
};
use vigia_lib::filters::{BranchFilter, WorkflowFilter};
use vigia_lib::model::{is_github_pull_request_event, RepoStatus, Run, RunState, TrayColor};

static NO_IGNORES: std::sync::LazyLock<WorkflowFilter> =
    std::sync::LazyLock::new(WorkflowFilter::default);

const NOW: OffsetDateTime = datetime!(2026-06-01 12:00 UTC);

fn run(id: u64, branch: &str, group: &str, state: RunState) -> Run {
    common::run_in(
        id,
        1,
        branch,
        group,
        state,
        format!("https://example.test/runs/{id}"),
        NOW - Duration::hours(1),
    )
}

fn selection<'a>(filter: &'a BranchFilter) -> Selection<'a> {
    Selection {
        default_branch: "main",
        branch_filter: filter,
        workflow_filter: &NO_IGNORES,
        exclude_pull_requests: true,
        include_tags: false,
        active_groups: None,
        now: NOW,
    }
}

// --- status mapping -------------------------------------------------------

#[test]
fn github_status_mapping_covers_every_value() {
    let cases = [
        (("queued", None), RunState::Queued),
        (("requested", None), RunState::Queued),
        (("pending", None), RunState::Queued),
        (("waiting", None), RunState::Neutral),
        (("in_progress", None), RunState::Running),
        (("completed", Some("success")), RunState::Success),
        (("completed", Some("failure")), RunState::Failed),
        (("completed", Some("timed_out")), RunState::Failed),
        (("completed", Some("startup_failure")), RunState::Failed),
        (("completed", Some("cancelled")), RunState::Canceled),
        (("completed", Some("skipped")), RunState::Neutral),
        (("completed", Some("neutral")), RunState::Neutral),
        (("completed", Some("stale")), RunState::Neutral),
        (("completed", Some("action_required")), RunState::Neutral),
        (("completed", None), RunState::Neutral),
        (("something_new", None), RunState::Neutral),
        (("completed", Some("something_new")), RunState::Neutral),
    ];
    for ((status, conclusion), expected) in cases {
        assert_eq!(
            RunState::from_github(status, conclusion),
            expected,
            "{status} / {conclusion:?}"
        );
    }
}

#[test]
fn gitlab_status_mapping_covers_every_value() {
    let cases = [
        ("created", RunState::Queued),
        ("waiting_for_resource", RunState::Queued),
        ("waiting_for_callback", RunState::Queued),
        ("preparing", RunState::Queued),
        ("pending", RunState::Queued),
        ("scheduled", RunState::Queued),
        ("running", RunState::Running),
        ("success", RunState::Success),
        ("failed", RunState::Failed),
        ("canceled", RunState::Canceled),
        ("canceling", RunState::Canceled),
        ("skipped", RunState::Neutral),
        ("manual", RunState::Neutral),
        ("something_new", RunState::Neutral),
    ];
    for (status, expected) in cases {
        assert_eq!(RunState::from_gitlab(status), expected, "{status}");
    }
}

#[test]
fn github_pull_request_events() {
    for event in [
        "pull_request",
        "pull_request_target",
        "pull_request_review",
        "pull_request_review_comment",
    ] {
        assert!(is_github_pull_request_event(event), "{event}");
    }
    for event in ["push", "schedule", "workflow_dispatch"] {
        assert!(!is_github_pull_request_event(event), "{event}");
    }
}

// --- branch filters -------------------------------------------------------

#[test]
fn default_filter_matches_only_the_default_branch() {
    let filter = BranchFilter::Default;
    assert!(filter.matches("main", "main"));
    assert!(!filter.matches("develop", "main"));
}

#[test]
fn glob_filter_matches_patterns() {
    let filter = BranchFilter::globs(&["main".into(), "release/*".into()]).unwrap();
    assert!(filter.matches("main", "develop"));
    assert!(filter.matches("release/1.2", "develop"));
    assert!(!filter.matches("develop", "develop"));
    assert!(!filter.matches("release/1.2/hotfix", "develop"));
}

#[test]
fn invalid_glob_is_an_error() {
    assert!(BranchFilter::globs(&["[".into()]).is_err());
}

// --- repo state selection -------------------------------------------------

#[test]
fn pull_request_runs_are_dropped_when_excluded() {
    let mut pr = run(2, "main", "ci", RunState::Failed);
    pr.pull_request = true;
    let runs = vec![run(1, "main", "ci", RunState::Success), pr];
    let filter = BranchFilter::Default;

    let groups = select_groups(&runs, &selection(&filter));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].run.id, 1);

    let mut include = selection(&filter);
    include.exclude_pull_requests = false;
    let groups = select_groups(&runs, &include);
    assert_eq!(groups[0].run.id, 2);
}

#[test]
fn fork_runs_are_always_dropped() {
    let mut fork = run(2, "main", "ci", RunState::Failed);
    fork.fork = true;
    let runs = vec![run(1, "main", "ci", RunState::Success), fork];
    let filter = BranchFilter::Default;
    let groups = select_groups(&runs, &selection(&filter));
    assert_eq!(groups[0].run.id, 1);
}

#[test]
fn tag_runs_are_dropped() {
    let mut tag = run(2, "main", "ci", RunState::Failed);
    tag.tag = true;
    let runs = vec![run(1, "main", "ci", RunState::Success), tag];
    let filter = BranchFilter::Default;
    let groups = select_groups(&runs, &selection(&filter));
    assert_eq!(groups[0].run.id, 1);
}

#[test]
fn inactive_groups_are_dropped() {
    let runs = vec![
        run(1, "main", "ci", RunState::Success),
        run(2, "main", "old-workflow", RunState::Failed),
    ];
    let active: HashSet<String> = ["ci".to_string()].into_iter().collect();
    let filter = BranchFilter::Default;
    let mut sel = selection(&filter);
    sel.active_groups = Some(&active);
    let groups = select_groups(&runs, &sel);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].run.group, "ci");
}

#[test]
fn groups_are_keyed_by_branch_and_group() {
    let runs = vec![
        run(1, "main", "push", RunState::Success),
        run(2, "main", "schedule", RunState::Failed),
    ];
    let filter = BranchFilter::Default;
    let groups = select_groups(&runs, &selection(&filter));
    assert_eq!(groups.len(), 2);
}

#[test]
fn canceled_and_neutral_runs_are_skipped_and_empty_groups_dropped() {
    let runs = vec![
        run(1, "main", "ci", RunState::Failed),
        run(2, "main", "ci", RunState::Canceled),
        run(3, "main", "ci", RunState::Neutral),
        run(4, "main", "lint", RunState::Canceled),
    ];
    let filter = BranchFilter::Default;
    let groups = select_groups(&runs, &selection(&filter));
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].run.id, 1);
}

#[test]
fn highest_id_wins_regardless_of_input_order() {
    let runs = vec![
        run(5, "main", "ci", RunState::Success),
        run(9, "main", "ci", RunState::Running),
        run(7, "main", "ci", RunState::Failed),
    ];
    let filter = BranchFilter::Default;
    let groups = select_groups(&runs, &selection(&filter));
    assert_eq!(groups[0].run.id, 9);
    assert_eq!(groups[0].run.state, RunState::Running);
}

#[test]
fn branch_filter_applies_to_groups() {
    let runs = vec![
        run(1, "main", "ci", RunState::Success),
        run(2, "develop", "ci", RunState::Failed),
        run(3, "release/1.0", "ci", RunState::Failed),
    ];
    let default = BranchFilter::Default;
    let groups = select_groups(&runs, &selection(&default));
    assert_eq!(groups.len(), 1);

    let globs = BranchFilter::globs(&["main".into(), "release/*".into()]).unwrap();
    let groups = select_groups(&runs, &selection(&globs));
    assert_eq!(groups.len(), 2);
}

#[test]
fn stale_non_default_branches_are_dropped() {
    let mut old = run(2, "release/0.9", "ci", RunState::Failed);
    old.updated_at = NOW - Duration::days(31);
    let mut old_default = run(3, "main", "ci", RunState::Failed);
    old_default.updated_at = NOW - Duration::days(400);
    let runs = vec![
        run(1, "release/1.0", "ci", RunState::Failed),
        old,
        old_default,
    ];
    let globs = BranchFilter::globs(&["main".into(), "release/*".into()]).unwrap();
    let groups = select_groups(&runs, &selection(&globs));
    let mut ids: Vec<u64> = groups.iter().map(|g| g.run.id).collect();
    ids.sort();
    assert_eq!(ids, vec![1, 3]);
}

// --- aggregation ----------------------------------------------------------

#[test]
fn repo_status_is_the_worst_group_and_representative_breaks_ties_by_recency() {
    let mut newer = run(2, "main", "lint", RunState::Failed);
    newer.updated_at = NOW - Duration::minutes(5);
    let runs = vec![
        run(1, "main", "ci", RunState::Failed),
        newer,
        run(3, "main", "docs", RunState::Success),
    ];
    let filter = BranchFilter::Default;
    let groups = select_groups(&runs, &selection(&filter));
    let state = vigia_lib::aggregate::repo_state(groups, NOW);
    assert_eq!(state.status, RepoStatus::Failed);
    assert_eq!(state.representative.as_ref().unwrap().id, 2);
    assert_eq!(state.groups.len(), 3);
}

#[test]
fn repo_with_no_groups_is_none() {
    let state = vigia_lib::aggregate::repo_state(vec![], NOW);
    assert_eq!(state.status, RepoStatus::None);
    assert!(state.representative.is_none());
}

#[test]
fn running_and_queued_map_to_running_status() {
    let filter = BranchFilter::Default;
    for s in [RunState::Running, RunState::Queued] {
        let groups = select_groups(&[run(1, "main", "ci", s)], &selection(&filter));
        let state = vigia_lib::aggregate::repo_state(groups, NOW);
        assert_eq!(state.status, RepoStatus::Running);
    }
}

#[test]
fn worst_state_order() {
    let order = [
        RepoStatus::Failed,
        RepoStatus::Error,
        RepoStatus::Running,
        RepoStatus::Success,
        RepoStatus::None,
    ];
    for pair in order.windows(2) {
        assert!(
            pair[0].is_worse_than(pair[1]),
            "{:?} vs {:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn overall_picks_worst_and_counts() {
    let statuses = [
        RepoStatus::Success,
        RepoStatus::Running,
        RepoStatus::Error,
        RepoStatus::Success,
        RepoStatus::None,
    ];
    let o = overall(statuses.iter().copied(), false);
    assert_eq!(o.worst, RepoStatus::Error);
    assert_eq!(o.color(), TrayColor::Orange);
    assert_eq!(
        o.tooltip(),
        "1 with errors, 1 running, 2 passing, 1 with no runs"
    );

    let o = overall([RepoStatus::Failed, RepoStatus::Success].into_iter(), false);
    assert_eq!(o.color(), TrayColor::Red);
    assert_eq!(o.tooltip(), "1 failed, 1 passing");
}

#[test]
fn overall_is_gray_when_paused_or_empty() {
    let paused = overall([RepoStatus::Failed].into_iter(), true);
    assert_eq!(paused.color(), TrayColor::Gray);
    assert_eq!(paused.tooltip(), "Paused");

    let empty: Overall = overall(std::iter::empty(), false);
    assert_eq!(empty.color(), TrayColor::Gray);
    assert_eq!(empty.tooltip(), "No watched repositories");
}

#[test]
fn tray_colors_per_status() {
    let cases = [
        (RepoStatus::Failed, TrayColor::Red),
        (RepoStatus::Error, TrayColor::Orange),
        (RepoStatus::Running, TrayColor::Yellow),
        (RepoStatus::Success, TrayColor::Green),
        (RepoStatus::None, TrayColor::Gray),
    ];
    for (status, color) in cases {
        assert_eq!(overall([status].into_iter(), false).color(), color);
    }
}

fn named_run(id: u64, group: &str, group_name: &str, name: &str) -> Run {
    let mut r = run(id, "main", group, RunState::Success);
    r.group_name = group_name.to_string();
    r.name = name.to_string();
    r
}

fn ignoring(patterns: &[&str]) -> WorkflowFilter {
    let patterns: Vec<String> = patterns.iter().map(|p| p.to_string()).collect();
    WorkflowFilter::globs(&patterns).unwrap()
}

fn group_keys(runs: &[Run], workflow_filter: &WorkflowFilter) -> Vec<String> {
    let branch = BranchFilter::Default;
    let mut sel = selection(&branch);
    sel.workflow_filter = workflow_filter;
    select_groups(runs, &sel)
        .into_iter()
        .map(|g| g.key)
        .collect()
}

#[test]
fn github_group_is_ignored_by_workflow_name_not_run_name() {
    let runs = vec![
        named_run(
            1,
            "100",
            "Dependabot Updates",
            "composer in /. for acme/widgets - Update #123",
        ),
        named_run(2, "200", "Deploy to Production", "Deploy to Production"),
    ];
    assert_eq!(group_keys(&runs, &ignoring(&["Dependabot*"])), vec!["200"]);
    // The run name alone does not match.
    assert_eq!(
        group_keys(&runs, &ignoring(&["composer*"])),
        vec!["100", "200"]
    );
}

#[test]
fn workflow_patterns_are_case_insensitive_and_empty_ignores_nothing() {
    let runs = vec![named_run(1, "100", "Deploy to Production", "x")];
    assert!(group_keys(&runs, &ignoring(&["deploy *"])).is_empty());
    assert_eq!(group_keys(&runs, &ignoring(&[])), vec!["100"]);
}

#[test]
fn gitlab_groups_match_on_pipeline_name_or_source() {
    let runs = vec![
        named_run(1, "schedule", "schedule", "schedule"),
        named_run(2, "web", "Nightly Build", "Nightly Build"),
        named_run(3, "push", "push", "push"),
    ];
    assert_eq!(
        group_keys(&runs, &ignoring(&["schedule", "nightly*"])),
        vec!["push"]
    );
}

#[test]
fn invalid_workflow_pattern_does_not_compile() {
    assert!(WorkflowFilter::globs(&["[".into()]).is_err());
}

// --- tag runs ---------------------------------------------------------------

fn tag_run(id: u64, tag: &str, group: &str, state: RunState) -> Run {
    Run {
        tag: true,
        ..run(id, tag, group, state)
    }
}

fn with_tags<'a>(filter: &'a BranchFilter) -> Selection<'a> {
    Selection {
        include_tags: true,
        ..selection(filter)
    }
}

#[test]
fn tag_runs_are_dropped_when_tags_are_off() {
    let filter = BranchFilter::Default;
    let runs = vec![tag_run(1, "1.6.0", "release", RunState::Success)];
    assert!(select_groups(&runs, &selection(&filter)).is_empty());
    // A glob matching the tag name does not bring it back.
    let globs = BranchFilter::globs(&["*".into()]).unwrap();
    assert!(select_groups(&runs, &selection(&globs)).is_empty());
}

#[test]
fn tag_runs_are_kept_alongside_the_branch_filter_when_tags_are_on() {
    let filter = BranchFilter::Default;
    let runs = vec![
        run(1, "main", "ci", RunState::Success),
        run(2, "feature/x", "ci", RunState::Failed),
        tag_run(3, "1.6.0", "release", RunState::Success),
    ];
    let groups = select_groups(&runs, &with_tags(&filter));
    let ids: Vec<u64> = groups.iter().map(|g| g.run.id).collect();
    assert_eq!(groups.len(), 2);
    assert!(ids.contains(&1) && ids.contains(&3));
    let tag_group = groups.iter().find(|g| g.run.id == 3).unwrap();
    assert_eq!(tag_group.branch, TAG_GROUP_BRANCH);
    assert_eq!(tag_group.run.branch, "1.6.0");
}

#[test]
fn tag_runs_of_one_group_collapse_to_the_newest_run() {
    let filter = BranchFilter::Default;
    let runs = vec![
        tag_run(10, "1.5.0", "release", RunState::Failed),
        tag_run(12, "1.6.0", "release", RunState::Success),
        tag_run(11, "1.5.1", "release", RunState::Failed),
        tag_run(5, "1.6.0", "publish", RunState::Success),
    ];
    let groups = select_groups(&runs, &with_tags(&filter));
    assert_eq!(groups.len(), 2);
    let release = groups.iter().find(|g| g.key == "release").unwrap();
    assert_eq!(release.run.id, 12);
    assert_eq!(release.run.branch, "1.6.0");
    assert!(groups.iter().all(|g| g.branch == TAG_GROUP_BRANCH));
}

#[test]
fn tag_groups_are_not_dropped_after_the_stale_cutoff() {
    let filter = BranchFilter::Default;
    let old = NOW - STALE_BRANCH_CUTOFF - Duration::days(30);
    let runs = vec![Run {
        updated_at: old,
        ..tag_run(1, "1.0.0", "release", RunState::Success)
    }];
    assert_eq!(select_groups(&runs, &with_tags(&filter)).len(), 1);
}

#[test]
fn tag_runs_still_follow_the_other_rules() {
    let filter = BranchFilter::Default;
    let ignore_release = WorkflowFilter::globs(&["release".into()]).unwrap();
    let active: HashSet<String> = ["ci".to_string()].into_iter().collect();
    let fork = Run {
        fork: true,
        ..tag_run(1, "1.0.0", "ci", RunState::Success)
    };
    let pull_request = Run {
        pull_request: true,
        ..tag_run(2, "1.0.0", "ci", RunState::Success)
    };
    let canceled = tag_run(3, "1.0.0", "ci", RunState::Canceled);
    let ignored = tag_run(4, "1.0.0", "release", RunState::Success);
    let inactive = tag_run(5, "1.0.0", "old", RunState::Success);
    let runs = vec![fork, pull_request, canceled, ignored, inactive];
    let selection = Selection {
        workflow_filter: &ignore_release,
        active_groups: Some(&active),
        ..with_tags(&filter)
    };
    assert!(select_groups(&runs, &selection).is_empty());
}

#[test]
fn run_state_rank_orders_failures_first_and_inactive_states_last() {
    use vigia_lib::model::RunState;
    assert_eq!(RunState::Failed.rank(), 0);
    assert_eq!(RunState::Running.rank(), RunState::Queued.rank());
    assert!(RunState::Queued.rank() < RunState::Success.rank());
    assert!(RunState::Success.rank() < RunState::Canceled.rank());
    assert_eq!(RunState::Canceled.rank(), RunState::Neutral.rank());
}
