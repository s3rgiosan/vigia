//! Builders shared by the integration tests. Each test binary uses a subset.
#![allow(dead_code)]

use std::sync::Arc;

use time::OffsetDateTime;
use vigia_lib::app::Runtime;
use vigia_lib::config::{Config, ConfigStore, WatchedRepo};
use vigia_lib::filters::BranchFilter;
use vigia_lib::model::{Run, RunState};
use vigia_lib::providers::{FetchRequest, RepoInfo};
use vigia_lib::secrets::{MemoryStore, Secrets};

/// A runtime over `config` with an in-memory secret store and no-op sinks.
pub fn runtime_with(config: ConfigStore) -> Arc<Runtime> {
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::default())));
    Arc::new(Runtime::new(
        config,
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ))
}

/// A runtime over an in-memory store seeded with `config`.
pub fn runtime_with_config(config: Config) -> Arc<Runtime> {
    runtime_with(ConfigStore::in_memory(config))
}

pub fn repo_info(id: u64, full_name: &str, web_url: &str) -> RepoInfo {
    RepoInfo {
        id,
        full_name: full_name.into(),
        web_url: web_url.into(),
        default_branch: "main".into(),
    }
}

/// `acme/r{id}` hosted at `https://{host}/acme/r{id}`.
pub fn acme_repo(id: u64, host: &str) -> RepoInfo {
    repo_info(
        id,
        &format!("acme/r{id}"),
        &format!("https://{host}/acme/r{id}"),
    )
}

pub fn watched_repo(
    account_id: &str,
    repo: RepoInfo,
    branch_patterns: Option<Vec<String>>,
) -> WatchedRepo {
    WatchedRepo {
        account_id: account_id.into(),
        repo,
        branch_patterns,
        ignored_workflows: None,
        include_tags: None,
    }
}

/// A run whose group, name and group name are all `group`.
pub fn run_in(
    id: u64,
    attempt: u32,
    branch: &str,
    group: &str,
    state: RunState,
    url: String,
    updated_at: OffsetDateTime,
) -> Run {
    Run {
        id,
        attempt,
        state,
        branch: branch.into(),
        group: group.into(),
        name: group.into(),
        group_name: group.into(),
        url,
        updated_at,
        pull_request: false,
        fork: false,
        tag: false,
    }
}

pub fn fetch_request<'a>(
    repo: &'a RepoInfo,
    filter: &'a BranchFilter,
    now: OffsetDateTime,
) -> FetchRequest<'a> {
    FetchRequest {
        repo,
        branch_filter: filter,
        exclude_pull_requests: true,
        now,
        refresh_groups: false,
        include_tags: false,
    }
}

pub fn fetch_request_with_tags<'a>(
    repo: &'a RepoInfo,
    filter: &'a BranchFilter,
    now: OffsetDateTime,
) -> FetchRequest<'a> {
    FetchRequest {
        include_tags: true,
        ..fetch_request(repo, filter, now)
    }
}
