//! GitHub Actions on github.com.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde::Deserialize;
use time::{Duration, OffsetDateTime};
use url::Url;

use super::{
    refresh_tags, AccountIdentity, FetchOutcome, FetchRequest, Provider, ProviderError, RepoCache,
    RepoInfo,
};
use crate::filters::BranchFilter;
use crate::http::HttpClient;
use crate::model::{is_github_pull_request_event, Run, RunState};

pub const DEFAULT_BASE_URL: &str = "https://api.github.com";
pub const API_VERSION: &str = "2026-03-10";

/// Prefix GitHub gives fine-grained personal access tokens. Classic tokens start with `ghp_`.
pub const FINE_GRAINED_PREFIX: &str = "github_pat_";

const TOKEN_PAGE: &str = "https://github.com/settings/personal-access-tokens/new";
const TOKEN_NAME: &str = "Vigia";
const TOKEN_DESCRIPTION: &str =
    "Read-only access to GitHub Actions runs for the Vigia menu bar app.";
const TOKEN_EXPIRES_IN_DAYS: u32 = 90;

/// Whether `token` is a fine-grained personal access token.
pub fn is_fine_grained_token(token: &str) -> bool {
    let token = token.trim();
    token.len() > FINE_GRAINED_PREFIX.len() && token.starts_with(FINE_GRAINED_PREFIX)
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("not a valid GitHub user or organization name")]
pub struct InvalidOwner;

/// Link to GitHub's fine-grained token form, prefilled with the name, expiry, owner, and the
/// read-only Actions and Metadata permissions Vigia needs. Repository access is picked on the page.
pub fn token_creation_url(owner: Option<&str>) -> Result<Url, InvalidOwner> {
    let owner = owner.map(str::trim).filter(|o| !o.is_empty());
    if let Some(owner) = owner {
        if !is_valid_owner(owner) {
            return Err(InvalidOwner);
        }
    }
    let mut url = Url::parse(TOKEN_PAGE).map_err(|_| InvalidOwner)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("name", TOKEN_NAME);
        query.append_pair("description", TOKEN_DESCRIPTION);
        if let Some(owner) = owner {
            query.append_pair("target_name", owner);
        }
        query.append_pair("expires_in", &TOKEN_EXPIRES_IN_DAYS.to_string());
        query.append_pair("actions", "read");
        query.append_pair("metadata", "read");
    }
    Ok(url)
}

/// GitHub logins: 1 to 39 alphanumerics or single hyphens, not starting or ending with one.
fn is_valid_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && !owner.starts_with('-')
        && !owner.ends_with('-')
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Active workflow lists are reused for this long before they are fetched again.
pub const WORKFLOWS_TTL: Duration = Duration::hours(6);

const RUNS_PER_PAGE: u32 = 100;

pub struct GitHubProvider {
    http: HttpClient,
    base_url: Url,
}

impl GitHubProvider {
    pub fn new(token: &str) -> Result<GitHubProvider, ProviderError> {
        GitHubProvider::with_base_url(token, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(token: &str, base_url: &str) -> Result<GitHubProvider, ProviderError> {
        let base_url = Url::parse(base_url).map_err(|e| ProviderError::Decode(e.to_string()))?;
        Ok(GitHubProvider {
            http: HttpClient::new(token)?,
            base_url,
        })
    }

    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "x-github-api-version",
            HeaderValue::from_static(API_VERSION),
        );
        headers
    }

    fn url(&self, path: &str, query: &[(&str, String)]) -> Result<Url, ProviderError> {
        let mut url = self
            .base_url
            .join(path)
            .map_err(|e| ProviderError::Decode(e.to_string()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().cloned());
        }
        Ok(url)
    }

    async fn active_workflows(
        &self,
        repo: &RepoInfo,
        cache: &mut RepoCache,
        now: OffsetDateTime,
        force: bool,
    ) -> Result<u32, ProviderError> {
        let fresh = cache
            .active_groups_fetched_at
            .is_some_and(|at| now - at < WORKFLOWS_TTL);
        if cache.active_groups.is_some() && fresh && !force {
            return Ok(0);
        }

        // The ETag covers the first page only, so it is sent and kept only for single-page lists.
        // A non-empty list whose names are not cached skips it, so the names load.
        let cached_etag = cache
            .active_groups
            .as_ref()
            .filter(|a| a.is_empty() || !cache.workflow_names.is_empty())
            .and(cache.groups_etag.clone());
        let mut counted = 0;
        let mut active = HashSet::new();
        let mut names = HashMap::new();
        let mut first_etag = None;
        let mut url = self.url(
            &format!("repositories/{}/actions/workflows", repo.id),
            &[("per_page", "100".to_string())],
        )?;
        loop {
            let etag = if counted == 0 {
                cached_etag.as_deref()
            } else {
                None
            };
            let response = self.http.get(&url, &Self::headers(), etag).await?;
            if response.not_modified() {
                cache.active_groups_fetched_at = Some(now);
                return Ok(counted);
            }
            if counted == 0 {
                first_etag = response.etag();
            }
            counted += 1;
            let page: WorkflowsPage = response.json()?;
            for workflow in page.workflows.into_iter().filter(|w| w.state == "active") {
                let id = workflow.id.to_string();
                names.insert(id.clone(), workflow.name);
                active.insert(id);
            }
            match response.next_link_on(&self.base_url)? {
                Some(next) => url = next,
                None => break,
            }
        }
        cache.active_groups = Some(active);
        cache.workflow_names = names;
        cache.active_groups_fetched_at = Some(now);
        cache.groups_etag = if counted == 1 { first_etag } else { None };
        Ok(counted)
    }
}

#[async_trait]
impl Provider for GitHubProvider {
    async fn validate(&self) -> Result<AccountIdentity, ProviderError> {
        let url = self.url("user", &[])?;
        let response = self.http.get(&url, &Self::headers(), None).await?;
        let user: User = response.json()?;
        Ok(AccountIdentity {
            user_id: user.id,
            login: user.login,
        })
    }

    async fn list_repos(
        &self,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Vec<RepoInfo>, ProviderError> {
        let mut repos = Vec::new();
        let mut url = self.url(
            "user/repos",
            &[
                ("per_page", "100".to_string()),
                (
                    "affiliation",
                    "owner,collaborator,organization_member".to_string(),
                ),
            ],
        )?;
        loop {
            let response = self.http.get(&url, &Self::headers(), None).await?;
            let page: Vec<Repository> = response.json()?;
            repos.extend(page.into_iter().filter(|r| !r.archived).map(|r| RepoInfo {
                id: r.id,
                full_name: r.full_name,
                web_url: r.html_url,
                default_branch: r.default_branch,
            }));
            on_page(repos.len());
            match response.next_link_on(&self.base_url)? {
                Some(next) => url = next,
                None => break,
            }
        }
        Ok(repos)
    }

    async fn fetch(
        &self,
        request: FetchRequest<'_>,
        cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError> {
        let repo = request.repo;
        let mut counted = self
            .active_workflows(repo, cache, request.now, request.refresh_groups)
            .await?;

        let mut query = vec![
            ("exclude_pull_requests", "true".to_string()),
            ("per_page", RUNS_PER_PAGE.to_string()),
        ];
        let default_only = matches!(request.branch_filter, BranchFilter::Default);
        if default_only && !request.include_tags {
            query.push(("branch", repo.default_branch.clone()));
        }
        let url = self.url(&format!("repositories/{}/actions/runs", repo.id), &query)?;
        let response = self
            .http
            .get(&url, &Self::headers(), cache.runs_etag.as_deref())
            .await?;
        let rate_limit = response.rate_limit();
        let not_modified = response.not_modified();

        let (mut runs, runs_etag) = if not_modified {
            (cache.runs.clone(), cache.runs_etag.clone())
        } else {
            counted += 1;
            let page: RunsPage = response.json()?;
            let runs: Vec<Run> = page
                .workflow_runs
                .into_iter()
                .map(|r| r.into_run(repo.id))
                .collect();

            // A run from a workflow outside the cached list means the list may be out of date.
            // Each ID triggers one refresh; a disabled workflow stays unknown and must not loop.
            let unknown: Vec<String> = match cache.active_groups.as_ref() {
                Some(active) => runs
                    .iter()
                    .filter(|r| {
                        !active.contains(&r.group) && !cache.refreshed_for.contains(&r.group)
                    })
                    .map(|r| r.group.clone())
                    .collect(),
                None => Vec::new(),
            };
            if !unknown.is_empty() {
                cache.refreshed_for.extend(unknown);
                counted += self
                    .active_workflows(repo, cache, request.now, true)
                    .await?;
            }
            (runs, response.etag())
        };

        apply_workflow_names(&mut runs, &cache.workflow_names);
        if request.include_tags {
            let unknown = cache.tags.unknown_refs(&runs, &repo.default_branch);
            if !unknown.is_empty() {
                let url = self.url(
                    &format!("repositories/{}/tags", repo.id),
                    &[("per_page", "100".to_string())],
                )?;
                let refresh = refresh_tags(
                    &self.http,
                    url,
                    &Self::headers(),
                    &self.base_url,
                    &mut cache.tags,
                    unknown,
                )
                .await?;
                // A 304 does not count against GitHub's rate limit.
                counted += refresh.requests - u32::from(refresh.not_modified);
            }
            cache.tags.mark(&mut runs);
        }

        cache.runs_etag = runs_etag;
        cache.runs = runs.clone();
        Ok(FetchOutcome {
            runs,
            active_groups: cache.active_groups.clone(),
            not_modified,
            counted_requests: counted,
            rate_limit,
        })
    }
}

#[derive(Deserialize)]
struct User {
    id: u64,
    login: String,
}

#[derive(Deserialize)]
struct Repository {
    id: u64,
    full_name: String,
    html_url: String,
    default_branch: String,
    #[serde(default)]
    archived: bool,
}

#[derive(Deserialize)]
struct WorkflowsPage {
    workflows: Vec<Workflow>,
}

#[derive(Deserialize)]
struct Workflow {
    id: u64,
    #[serde(default)]
    name: String,
    state: String,
}

#[derive(Deserialize)]
struct RunsPage {
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
    #[serde(default = "one")]
    run_attempt: u32,
    status: String,
    conclusion: Option<String>,
    event: String,
    head_branch: Option<String>,
    workflow_id: u64,
    name: Option<String>,
    html_url: String,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
    head_repository: Option<HeadRepository>,
}

#[derive(Deserialize)]
struct HeadRepository {
    id: u64,
}

fn one() -> u32 {
    1
}

impl WorkflowRun {
    fn into_run(self, repo_id: u64) -> Run {
        let fork = self.head_repository.is_none_or(|h| h.id != repo_id);
        Run {
            id: self.id,
            attempt: self.run_attempt,
            state: RunState::from_github(&self.status, self.conclusion.as_deref()),
            branch: self.head_branch.unwrap_or_default(),
            group: self.workflow_id.to_string(),
            group_name: self.name.clone().unwrap_or_default(),
            name: self.name.unwrap_or_else(|| self.workflow_id.to_string()),
            url: self.html_url,
            updated_at: self.updated_at,
            pull_request: is_github_pull_request_event(&self.event),
            fork,
            tag: false,
        }
    }
}

/// Sets each run's group name to its workflow's name. Runs of unknown workflows keep the run
/// name, falling back to the workflow ID when the run has none.
fn apply_workflow_names(runs: &mut [Run], names: &HashMap<String, String>) {
    for run in runs {
        match names.get(&run.group) {
            Some(name) if !name.is_empty() => run.group_name = name.clone(),
            _ => {
                if run.group_name.is_empty() {
                    run.group_name = run.name.clone();
                }
            }
        }
    }
}
