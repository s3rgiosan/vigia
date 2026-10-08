//! Provider-neutral domain types: run states, repo status, and the tray summary.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Normalized state of one workflow run or pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Failed,
    Running,
    Queued,
    Success,
    Canceled,
    Neutral,
}

static UNKNOWN_VALUES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Logs a status value the mapping does not know, once per launch.
fn log_unknown(provider: &str, value: &str) {
    let key = format!("{provider}:{value}");
    let mut seen = UNKNOWN_VALUES.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.contains(&key) {
        log::warn!("unknown {provider} status value {value:?}; treating as neutral");
        seen.push(key);
    }
}

impl RunState {
    /// Maps a GitHub Actions run `status` and `conclusion` pair.
    pub fn from_github(status: &str, conclusion: Option<&str>) -> RunState {
        match status {
            "queued" | "requested" | "pending" => RunState::Queued,
            "waiting" => RunState::Neutral,
            "in_progress" => RunState::Running,
            "completed" => match conclusion {
                Some("success") => RunState::Success,
                Some("failure") | Some("timed_out") | Some("startup_failure") => RunState::Failed,
                Some("cancelled") => RunState::Canceled,
                Some("skipped")
                | Some("neutral")
                | Some("stale")
                | Some("action_required")
                | None => RunState::Neutral,
                Some(other) => {
                    log_unknown("github conclusion", other);
                    RunState::Neutral
                }
            },
            other => {
                log_unknown("github status", other);
                RunState::Neutral
            }
        }
    }

    /// Maps a GitLab pipeline `status`.
    pub fn from_gitlab(status: &str) -> RunState {
        match status {
            "created"
            | "waiting_for_resource"
            | "waiting_for_callback"
            | "preparing"
            | "pending"
            | "scheduled" => RunState::Queued,
            "running" => RunState::Running,
            "success" => RunState::Success,
            "failed" => RunState::Failed,
            "canceled" | "canceling" => RunState::Canceled,
            "skipped" | "manual" => RunState::Neutral,
            other => {
                log_unknown("gitlab status", other);
                RunState::Neutral
            }
        }
    }

    /// Canceled and Neutral runs never represent a group.
    pub fn is_selectable(self) -> bool {
        !matches!(self, RunState::Canceled | RunState::Neutral)
    }

    /// Lower is worse. Only selectable states have a meaningful rank.
    pub fn rank(self) -> u8 {
        match self {
            RunState::Failed => 0,
            RunState::Running | RunState::Queued => 1,
            RunState::Success => 2,
            RunState::Canceled | RunState::Neutral => 3,
        }
    }
}

/// GitHub events that belong to pull requests.
pub fn is_github_pull_request_event(event: &str) -> bool {
    matches!(
        event,
        "pull_request"
            | "pull_request_target"
            | "pull_request_review"
            | "pull_request_review_comment"
    )
}

/// One run or pipeline, already normalized by its provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: u64,
    /// GitHub `run_attempt`; always 1 on GitLab.
    pub attempt: u32,
    pub state: RunState,
    pub branch: String,
    /// Group key within a branch: workflow ID on GitHub, pipeline source on GitLab.
    pub group: String,
    /// Display name: workflow name or pipeline source.
    pub name: String,
    /// Name of the group the run belongs to, matched by the workflow filter: the workflow name
    /// on GitHub (the run name when the workflow is unknown), the pipeline name or source on
    /// GitLab.
    #[serde(default)]
    pub group_name: String,
    pub url: String,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub pull_request: bool,
    /// The run came from a fork, or its head repository is gone.
    pub fork: bool,
    /// The run's branch is a tag name from the repo's tag list. Providers check it only when
    /// tag runs are on for the repo.
    pub tag: bool,
}

/// State of a watched repo. Variants are listed worst first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoStatus {
    Failed,
    Error,
    Running,
    Success,
    #[default]
    None,
}

impl RepoStatus {
    fn rank(self) -> u8 {
        match self {
            RepoStatus::Failed => 0,
            RepoStatus::Error => 1,
            RepoStatus::Running => 2,
            RepoStatus::Success => 3,
            RepoStatus::None => 4,
        }
    }

    pub fn is_worse_than(self, other: RepoStatus) -> bool {
        self.rank() < other.rank()
    }

    pub fn from_run_state(state: RunState) -> RepoStatus {
        match state {
            RunState::Failed => RepoStatus::Failed,
            RunState::Running | RunState::Queued => RepoStatus::Running,
            RunState::Success => RepoStatus::Success,
            RunState::Canceled | RunState::Neutral => RepoStatus::None,
        }
    }
}

/// Color of the menu bar dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrayColor {
    Red,
    Orange,
    Yellow,
    Green,
    Gray,
}

/// Full state of one watched repo after a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoState {
    pub status: RepoStatus,
    /// Chosen run of the worst group.
    pub representative: Option<Run>,
    /// Chosen run of every remaining group, in selection order.
    pub groups: Vec<Run>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_checked: Option<OffsetDateTime>,
    pub stale: bool,
    /// Error text or an informational note such as `notes::CI_DISABLED`.
    pub note: Option<String>,
}
