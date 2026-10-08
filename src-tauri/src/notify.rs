//! Notification rules: failure and recovery transitions per group, one auth error per account,
//! and coalescing of bursts into a summary.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::aggregate::group_branch;
use crate::config::{AccountKind, Config, Settings};
use crate::filters::{selection_changed, FilterScope};
use crate::model::{RepoState, RepoStatus, Run, RunState};
use crate::poller::AccountSnapshot;
use crate::providers::RepoInfo;

/// More transitions than this in one cycle collapse into one summary notification.
pub const COALESCE_ABOVE: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// What clicking the notification opens.
    pub target: ClickTarget,
}

/// Where a click on a notification leads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClickTarget {
    /// The popup, for summaries and account problems.
    Popup,
    /// A run page in the browser; the URL is checked against the account before opening.
    Run { account_id: String, url: String },
}

/// What changed for one group in one cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    Failed { repo: RepoInfo, run: Run },
    Recovered { repo: RepoInfo, run: Run },
}

/// Identity of a run attempt, used to avoid repeating a notification.
///
/// GitHub re-runs keep the run ID and bump `run_attempt`. GitLab retries keep the pipeline ID,
/// so `updated_at` tells the attempts apart.
pub fn identity_of(kind: AccountKind, run: &Run) -> String {
    match kind {
        AccountKind::GitHub => format!("{}#{}", run.id, run.attempt),
        AccountKind::GitLab => format!("{}@{}", run.id, run.updated_at.unix_timestamp()),
    }
}

#[derive(Debug, Clone, Default)]
struct GroupMemory {
    failed: bool,
    last_notified: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RepoMemory {
    status: RepoStatus,
    groups: HashMap<(String, String), GroupMemory>,
}

/// Remembers what was already notified, per `(account_id, repo_id)` and per account.
#[derive(Debug, Default)]
pub struct Notifier {
    repos: HashMap<(String, u64), RepoMemory>,
    auth_notified: HashSet<String>,
}

impl Notifier {
    /// Drops every repo baseline. The once-per-account auth memory is kept.
    pub fn reset(&mut self) {
        self.repos.clear();
    }

    /// Drops one repo's baseline, so its next successful state is observed silently.
    pub fn reset_repo(&mut self, account_id: &str, repo_id: u64) {
        self.repos.remove(&(account_id.to_string(), repo_id));
    }

    /// Forgets that an account's auth error was notified, so the next one notifies again.
    pub fn forget_auth(&mut self, account_id: &str) {
        self.auth_notified.remove(account_id);
    }

    /// Whether an auth error notification was already sent for the account.
    pub fn auth_notified(&self, account_id: &str) -> bool {
        self.auth_notified.contains(account_id)
    }

    /// Brings the memory in line with a newly applied config.
    ///
    /// With no previous config, as at launch, every baseline resets. Otherwise memory of
    /// removed repos and accounts is dropped, and the repos listed by `repos_to_rebaseline`
    /// reset their baseline. Everything else keeps its memory.
    pub fn apply_config(&mut self, previous: Option<&Config>, next: &Config) {
        let Some(previous) = previous else {
            self.reset();
            return;
        };
        let watched: HashSet<(String, u64)> = next
            .repos
            .iter()
            .map(|w| (w.account_id.clone(), w.repo.id))
            .collect();
        self.repos.retain(|key, _| watched.contains(key));
        let accounts: HashSet<&str> = next.accounts.iter().map(|a| a.id.as_str()).collect();
        self.auth_notified
            .retain(|id| accounts.contains(id.as_str()));
        for (account_id, repo_id) in repos_to_rebaseline(previous, next) {
            self.reset_repo(&account_id, repo_id);
        }
    }

    /// Compares a repo's new state with what was seen before and records the new state.
    ///
    /// The first successful state of a repo, or the first after an Error, sets a baseline and
    /// produces nothing. A group not seen before in a repo with a baseline notifies normally.
    /// Every failure and recovery is reported; `filter_transitions` applies the user's choice.
    pub fn observe(
        &mut self,
        kind: AccountKind,
        account_id: &str,
        repo: &RepoInfo,
        state: &RepoState,
    ) -> Vec<Transition> {
        let key = (account_id.to_string(), repo.id);
        let successful = state.last_checked.is_some() && state.status != RepoStatus::Error;
        if !successful {
            if let Some(memory) = self.repos.get_mut(&key) {
                memory.status = state.status;
            }
            return Vec::new();
        }

        let baseline = match self.repos.get(&key) {
            None => true,
            Some(memory) => memory.status == RepoStatus::Error,
        };
        let mut memory = if baseline {
            RepoMemory::default()
        } else {
            self.repos.remove(&key).unwrap_or_default()
        };
        let mut transitions = Vec::new();

        // Groups that left the repo's selection, such as merged feature branches, are forgotten.
        let current: HashSet<(&str, &str)> = state
            .groups
            .iter()
            .map(|run| (group_branch(run), run.group.as_str()))
            .collect();
        memory
            .groups
            .retain(|(branch, group), _| current.contains(&(branch.as_str(), group.as_str())));

        for run in &state.groups {
            let group_key = (group_branch(run).to_string(), run.group.clone());
            let entry = memory.groups.entry(group_key).or_default();
            let identity = identity_of(kind, run);
            match run.state {
                RunState::Failed => {
                    let already = entry.last_notified.as_deref() == Some(identity.as_str());
                    if !baseline && !already {
                        transitions.push(Transition::Failed {
                            repo: repo.clone(),
                            run: run.clone(),
                        });
                    }
                    entry.last_notified = Some(identity);
                    entry.failed = true;
                }
                RunState::Success => {
                    if !baseline && entry.failed {
                        transitions.push(Transition::Recovered {
                            repo: repo.clone(),
                            run: run.clone(),
                        });
                    }
                    entry.failed = false;
                }
                _ => {}
            }
        }

        memory.status = state.status;
        self.repos.insert(key, memory);
        transitions
    }

    /// How many groups of a repo are remembered.
    pub fn remembered_groups(&self, account_id: &str, repo_id: u64) -> usize {
        self.repos
            .get(&(account_id.to_string(), repo_id))
            .map_or(0, |memory| memory.groups.len())
    }

    /// One notification per account when its token starts failing; none again until it recovers.
    pub fn observe_account(&mut self, account: &AccountSnapshot) -> Option<Notification> {
        if account.auth_error {
            if self.auth_notified.insert(account.id.clone()) {
                return Some(Notification {
                    title: format!("{}: token rejected", account.label),
                    body: "Open Settings to replace the token.".to_string(),
                    target: ClickTarget::Popup,
                });
            }
            None
        } else {
            self.auth_notified.remove(&account.id);
            None
        }
    }
}

/// Repos of `next` whose baseline resets: newly watched repos, repos whose effective branch
/// filter, workflow filter or tag setting changed (from the repo, organization or global
/// settings), and every repo when the "exclude pull request runs" setting changed.
pub fn repos_to_rebaseline(previous: &Config, next: &Config) -> Vec<(String, u64)> {
    next.repos
        .iter()
        .filter(|w| {
            let before = previous
                .repos
                .iter()
                .find(|p| p.account_id == w.account_id && p.repo.id == w.repo.id);
            match before {
                None => true,
                Some(p) => selection_changed(
                    FilterScope::in_config(previous, p),
                    FilterScope::in_config(next, w),
                ),
            }
        })
        .map(|w| (w.account_id.clone(), w.repo.id))
        .collect()
}

/// Keeps the transitions the user wants to hear about: failures only with `notify_failures`,
/// recoveries only with `notify_recoveries`.
pub fn filter_transitions(transitions: Vec<Transition>, settings: &Settings) -> Vec<Transition> {
    transitions
        .into_iter()
        .filter(|t| match t {
            Transition::Failed { .. } => settings.notify_failures,
            Transition::Recovered { .. } => settings.notify_recoveries,
        })
        .collect()
}

/// Turns the transitions of one cycle into notifications, summarizing bursts. A summary counts
/// repositories, so a repo with several failing groups counts once.
pub fn render(
    account_id: &str,
    account_label: &str,
    transitions: &[Transition],
) -> Vec<Notification> {
    if transitions.len() > COALESCE_ABOVE {
        let mut failed = HashSet::new();
        let mut recovered = HashSet::new();
        for t in transitions {
            match t {
                Transition::Failed { repo, .. } => failed.insert(repo.id),
                Transition::Recovered { repo, .. } => recovered.insert(repo.id),
            };
        }
        let mut parts = Vec::new();
        if !failed.is_empty() {
            parts.push(format!("{} failed", repositories(failed.len())));
        }
        if !recovered.is_empty() {
            parts.push(format!("{} recovered", repositories(recovered.len())));
        }
        return vec![Notification {
            title: format!("{account_label}: {}", parts.join(", ")),
            body: "Open Vigia for the list.".to_string(),
            target: ClickTarget::Popup,
        }];
    }
    transitions
        .iter()
        .map(|t| {
            let (repo, run, verb) = match t {
                Transition::Failed { repo, run } => (repo, run, "failed"),
                Transition::Recovered { repo, run } => (repo, run, "recovered"),
            };
            Notification {
                title: format!("{} {verb}", short_name(&repo.full_name)),
                body: format!("{} · {}", run.branch, run.name),
                target: ClickTarget::Run {
                    account_id: account_id.to_string(),
                    url: run.url.clone(),
                },
            }
        })
        .collect()
}

/// "1 repository", "3 repositories".
fn repositories(count: usize) -> String {
    if count == 1 {
        "1 repository".to_string()
    } else {
        format!("{count} repositories")
    }
}

fn short_name(full_name: &str) -> &str {
    full_name.rsplit('/').next().unwrap_or(full_name)
}
