//! Repo state selection and the worst-state summary shown in the menu bar.

use std::collections::{BTreeMap, HashSet};

use time::{Duration, OffsetDateTime};

use crate::filters::{BranchFilter, WorkflowFilter};
use crate::model::{RepoState, RepoStatus, Run, TrayColor};

/// Groups on branches other than the default branch stop counting after this long. The tag
/// group of a workflow always counts.
pub const STALE_BRANCH_CUTOFF: Duration = Duration::days(30);

/// Branch part of the group key shared by every tag run of one workflow or pipeline source.
/// Git ref names cannot contain `*`, so no branch takes this value.
pub const TAG_GROUP_BRANCH: &str = "refs/tags/*";

/// The branch part of a run's group key: `TAG_GROUP_BRANCH` for a tag run, its branch otherwise.
pub fn group_branch(run: &Run) -> &str {
    if run.tag {
        TAG_GROUP_BRANCH
    } else {
        &run.branch
    }
}

/// Rules that turn a list of runs into groups.
pub struct Selection<'a> {
    pub default_branch: &'a str,
    pub branch_filter: &'a BranchFilter,
    pub workflow_filter: &'a WorkflowFilter,
    pub exclude_pull_requests: bool,
    /// Keeps tag runs, whatever the branch filter says.
    pub include_tags: bool,
    /// Group keys that still exist, such as active GitHub workflow IDs. `None` keeps every group.
    pub active_groups: Option<&'a HashSet<String>>,
    pub now: OffsetDateTime,
}

/// One branch-and-group pair with its chosen run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The branch, or `TAG_GROUP_BRANCH` for the tag group. The run keeps its tag name.
    pub branch: String,
    pub key: String,
    pub run: Run,
}

/// Applies the selection rules and returns one chosen run per remaining group.
pub fn select_groups(runs: &[Run], selection: &Selection) -> Vec<Group> {
    let mut chosen: BTreeMap<(String, String), Run> = BTreeMap::new();

    for run in runs {
        if run.fork {
            continue;
        }
        if run.tag && !selection.include_tags {
            continue;
        }
        if selection.exclude_pull_requests && run.pull_request {
            continue;
        }
        if selection.workflow_filter.ignores(&run.group_name) {
            continue;
        }
        if let Some(active) = selection.active_groups {
            if !active.contains(&run.group) {
                continue;
            }
        }
        if !run.tag
            && !selection
                .branch_filter
                .matches(&run.branch, selection.default_branch)
        {
            continue;
        }
        if !run.state.is_selectable() {
            continue;
        }

        let key = (group_branch(run).to_string(), run.group.clone());
        let replace = chosen.get(&key).is_none_or(|current| run.id > current.id);
        if replace {
            chosen.insert(key, run.clone());
        }
    }

    let cutoff = selection.now - STALE_BRANCH_CUTOFF;
    chosen
        .into_iter()
        .filter(|((branch, _), run)| {
            branch == selection.default_branch
                || branch == TAG_GROUP_BRANCH
                || run.updated_at >= cutoff
        })
        .map(|((branch, key), run)| Group { branch, key, run })
        .collect()
}

/// Builds the repo state from its groups. The representative run is the chosen run of the worst
/// group; ties go to the most recently updated run.
pub fn repo_state(groups: Vec<Group>, checked_at: OffsetDateTime) -> RepoState {
    let runs: Vec<Run> = groups.into_iter().map(|g| g.run).collect();
    let representative = runs
        .iter()
        .min_by(|a, b| {
            a.state
                .rank()
                .cmp(&b.state.rank())
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        })
        .cloned();
    let status = representative
        .as_ref()
        .map(|run| RepoStatus::from_run_state(run.state))
        .unwrap_or(RepoStatus::None);

    RepoState {
        status,
        representative,
        groups: runs,
        last_checked: Some(checked_at),
        stale: false,
        note: None,
    }
}

/// Worst status across every watched repo plus counts for the tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overall {
    pub worst: RepoStatus,
    pub paused: bool,
    pub failed: usize,
    pub error: usize,
    pub running: usize,
    pub passing: usize,
    pub none: usize,
}

impl Overall {
    pub fn total(&self) -> usize {
        self.failed + self.error + self.running + self.passing + self.none
    }

    pub fn color(&self) -> TrayColor {
        if self.paused || self.total() == 0 {
            return TrayColor::Gray;
        }
        match self.worst {
            RepoStatus::Failed => TrayColor::Red,
            RepoStatus::Error => TrayColor::Orange,
            RepoStatus::Running => TrayColor::Yellow,
            RepoStatus::Success => TrayColor::Green,
            RepoStatus::None => TrayColor::Gray,
        }
    }

    pub fn tooltip(&self) -> String {
        if self.paused {
            return "Paused".to_string();
        }
        if self.total() == 0 {
            return "No watched repositories".to_string();
        }
        let parts = [
            (self.failed, "failed"),
            (self.error, "with errors"),
            (self.running, "running"),
            (self.passing, "passing"),
            (self.none, "with no runs"),
        ];
        parts
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, label)| format!("{count} {label}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn overall(statuses: impl Iterator<Item = RepoStatus>, paused: bool) -> Overall {
    let mut o = Overall {
        worst: RepoStatus::None,
        paused,
        failed: 0,
        error: 0,
        running: 0,
        passing: 0,
        none: 0,
    };
    for status in statuses {
        match status {
            RepoStatus::Failed => o.failed += 1,
            RepoStatus::Error => o.error += 1,
            RepoStatus::Running => o.running += 1,
            RepoStatus::Success => o.passing += 1,
            RepoStatus::None => o.none += 1,
        }
        if status.is_worse_than(o.worst) {
            o.worst = status;
        }
    }
    o
}
