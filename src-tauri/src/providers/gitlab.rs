//! GitLab CI on self-managed instances and gitlab.com.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use time::OffsetDateTime;
use url::Url;

use super::{
    refresh_tags, AccountIdentity, FetchOutcome, FetchRequest, Membership, Provider, ProviderError,
    RepoCache, RepoInfo,
};
use crate::filters::BranchFilter;
use crate::http::HttpClient;
use crate::model::{Run, RunState};

const PIPELINES_PER_PAGE: u32 = 100;

/// How long a project membership answer is reused. A project with CI/CD disabled answers every
/// pipeline request with 403, and the membership check behind it runs once per this period.
pub const MEMBERSHIP_TTL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum BaseUrlError {
    #[error("not a valid URL")]
    Invalid,
    #[error("the URL must start with https:// or http://")]
    Scheme,
    #[error("the URL must not contain a user name or password")]
    UserInfo,
    #[error("the URL must not contain a query string or fragment")]
    QueryOrFragment,
    #[error("the URL has no host")]
    NoHost,
}

/// Canonical form of an instance base URL, so two spellings of one instance compare equal.
///
/// The host is lowercased, user info, query and fragment are rejected, a path prefix such as
/// `/gitlab` is kept, and trailing slashes are stripped. `Url::parse` already drops a default
/// port such as `:443`.
pub fn normalize_base_url(input: &str) -> Result<Url, BaseUrlError> {
    let trimmed = input.trim();
    let mut url = Url::parse(trimmed).map_err(|_| BaseUrlError::Invalid)?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err(BaseUrlError::Scheme);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BaseUrlError::UserInfo);
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(BaseUrlError::QueryOrFragment);
    }
    let host = url
        .host_str()
        .ok_or(BaseUrlError::NoHost)?
        .to_ascii_lowercase();
    url.set_host(Some(&host))
        .map_err(|_| BaseUrlError::NoHost)?;
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url)
}

pub struct GitLabProvider {
    http: HttpClient,
    base_url: Url,
    membership_ttl: Duration,
    /// Membership answers by project ID, with when they were fetched.
    memberships: Mutex<HashMap<u64, (bool, Instant)>>,
}

impl GitLabProvider {
    pub fn new(base_url: &str, token: &str) -> Result<GitLabProvider, ProviderError> {
        let base_url =
            normalize_base_url(base_url).map_err(|e| ProviderError::Decode(e.to_string()))?;
        Ok(GitLabProvider {
            http: HttpClient::new(token)?,
            base_url,
            membership_ttl: MEMBERSHIP_TTL,
            memberships: Mutex::new(HashMap::new()),
        })
    }

    /// Reuses membership answers for `ttl`; `MEMBERSHIP_TTL` is the default.
    pub fn with_membership_ttl(mut self, ttl: Duration) -> GitLabProvider {
        self.membership_ttl = ttl;
        self
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    fn cached_membership(&self, project_id: u64) -> Option<bool> {
        let memberships = self.memberships.lock().unwrap();
        let (member, fetched_at) = memberships.get(&project_id)?;
        let age = fetched_at.elapsed();
        (age < self.membership_ttl).then_some(*member)
    }

    fn remember_membership(&self, project_id: u64, member: bool) {
        self.memberships
            .lock()
            .unwrap()
            .insert(project_id, (member, Instant::now()));
    }

    /// Asks the API whether the account is a member of the project.
    async fn fetch_membership(&self, project_id: u64) -> Result<bool, ProviderError> {
        let url = self.url(&format!("projects/{project_id}"), &[])?;
        let response = match self.http.get(&url, &HeaderMap::new(), None).await {
            Ok(response) => response,
            Err(ProviderError::NotFound | ProviderError::Forbidden) => return Ok(false),
            Err(e) => return Err(e),
        };
        let project: ProjectAccess = response.json()?;
        Ok(project
            .permissions
            .is_some_and(|p| p.project_access.is_some() || p.group_access.is_some()))
    }

    fn url(&self, path: &str, query: &[(&str, String)]) -> Result<Url, ProviderError> {
        let base = self.base_url.as_str().trim_end_matches('/');
        let joined = format!("{base}/api/v4/{path}");
        let mut url = Url::parse(&joined).map_err(|e| ProviderError::Decode(e.to_string()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().cloned());
        }
        Ok(url)
    }

    /// Lists member projects with keyset pagination. With `simple`, returns `None` when the
    /// first page lacks `default_branch` or `web_url`, which some instances omit from the
    /// simple representation.
    async fn list_projects(
        &self,
        simple: bool,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Option<Vec<RepoInfo>>, ProviderError> {
        let mut query = vec![
            ("membership", "true".to_string()),
            ("archived", "false".to_string()),
        ];
        if simple {
            query.push(("simple", "true".to_string()));
        }
        query.extend([
            ("per_page", "100".to_string()),
            ("pagination", "keyset".to_string()),
            ("order_by", "id".to_string()),
            ("sort", "asc".to_string()),
        ]);
        let mut url = self.url("projects", &query)?;
        let mut repos = Vec::new();
        let mut first_page = true;
        loop {
            let response = self.http.get(&url, &HeaderMap::new(), None).await?;
            let page: Vec<serde_json::Value> = response.json()?;
            if simple && first_page && !page.iter().all(has_listing_fields) {
                return Ok(None);
            }
            first_page = false;
            for item in page {
                let project: Project = serde_json::from_value(item)
                    .map_err(|e| ProviderError::Decode(e.to_string()))?;
                repos.push(RepoInfo {
                    id: project.id,
                    full_name: project.path_with_namespace,
                    web_url: project.web_url,
                    default_branch: project.default_branch.unwrap_or_default(),
                });
            }
            on_page(repos.len());
            match response.next_link_on(&self.base_url)? {
                Some(next) => url = next,
                None => break,
            }
        }
        Ok(Some(repos))
    }
}

/// Whether a listed project carries the fields Vigia stores. A null `default_branch` is
/// present; an absent key is not.
fn has_listing_fields(item: &serde_json::Value) -> bool {
    item.get("default_branch").is_some() && item.get("web_url").is_some()
}

#[async_trait]
impl Provider for GitLabProvider {
    async fn validate(&self) -> Result<AccountIdentity, ProviderError> {
        let url = self.url("user", &[])?;
        let response = self.http.get(&url, &HeaderMap::new(), None).await?;
        let user: User = response.json()?;
        Ok(AccountIdentity {
            user_id: user.id,
            login: user.username,
        })
    }

    async fn list_repos(
        &self,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Vec<RepoInfo>, ProviderError> {
        match self.list_projects(true, on_page).await? {
            Some(repos) => Ok(repos),
            None => {
                log::info!("project list without simple=true: fields were missing");
                let repos = self.list_projects(false, on_page).await?;
                Ok(repos.unwrap_or_default())
            }
        }
    }

    /// Answers from the membership cache while it is fresh. Errors other than 403 and 404 are
    /// not cached.
    async fn is_member(&self, repo: &RepoInfo) -> Result<bool, ProviderError> {
        self.membership(repo).await.map(|m| m.member)
    }

    async fn membership(&self, repo: &RepoInfo) -> Result<Membership, ProviderError> {
        if let Some(member) = self.cached_membership(repo.id) {
            return Ok(Membership {
                member,
                requests: 0,
            });
        }
        let member = self.fetch_membership(repo.id).await?;
        self.remember_membership(repo.id, member);
        Ok(Membership {
            member,
            requests: 1,
        })
    }

    async fn fetch(
        &self,
        request: FetchRequest<'_>,
        cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError> {
        let repo = request.repo;
        let mut query = vec![
            ("order_by", "id".to_string()),
            ("sort", "desc".to_string()),
            ("per_page", PIPELINES_PER_PAGE.to_string()),
        ];
        let default_only = matches!(request.branch_filter, BranchFilter::Default);
        if default_only && !request.include_tags {
            query.push(("ref", repo.default_branch.clone()));
        }
        let url = self.url(&format!("projects/{}/pipelines", repo.id), &query)?;
        let response = self
            .http
            .get(&url, &HeaderMap::new(), cache.runs_etag.as_deref())
            .await?;
        // GitLab reports a per-minute quota; the pool works in requests per hour.
        let rate_limit = response.rate_limit().map(|r| crate::http::RateLimit {
            limit: r.limit.saturating_mul(60),
            remaining: r.remaining.saturating_mul(60),
            reset_at: r.reset_at,
        });
        let not_modified = response.not_modified();
        let mut counted = 1;

        let (mut runs, runs_etag) = if not_modified {
            (cache.runs.clone(), cache.runs_etag.clone())
        } else {
            let pipelines: Vec<Pipeline> = response.json()?;
            let runs: Vec<Run> = pipelines.into_iter().map(Pipeline::into_run).collect();
            (runs, response.etag())
        };

        if request.include_tags {
            let unknown = cache.tags.unknown_refs(&runs, &repo.default_branch);
            if !unknown.is_empty() {
                let url = self.url(
                    &format!("projects/{}/repository/tags", repo.id),
                    &[("per_page", "100".to_string())],
                )?;
                let refresh = refresh_tags(
                    &self.http,
                    url,
                    &HeaderMap::new(),
                    &self.base_url,
                    &mut cache.tags,
                    unknown,
                )
                .await?;
                counted += refresh.requests;
            }
            cache.tags.mark(&mut runs);
        }

        cache.runs_etag = runs_etag;
        cache.runs = runs.clone();
        Ok(FetchOutcome {
            runs,
            active_groups: None,
            not_modified,
            counted_requests: counted,
            rate_limit,
        })
    }
}

#[derive(Deserialize)]
struct User {
    id: u64,
    username: String,
}

#[derive(Deserialize)]
struct Project {
    id: u64,
    path_with_namespace: String,
    web_url: String,
    default_branch: Option<String>,
}

#[derive(Deserialize)]
struct ProjectAccess {
    permissions: Option<Permissions>,
}

/// Access levels from a single-project response. Both are null when the user can see the
/// project, for example a public one, without being a member.
#[derive(Deserialize)]
struct Permissions {
    project_access: Option<serde_json::Value>,
    group_access: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Pipeline {
    id: u64,
    status: String,
    source: String,
    #[serde(rename = "ref")]
    git_ref: String,
    name: Option<String>,
    web_url: String,
    #[serde(with = "time::serde::rfc3339")]
    updated_at: OffsetDateTime,
}

impl Pipeline {
    fn into_run(self) -> Run {
        Run {
            id: self.id,
            attempt: 1,
            state: RunState::from_gitlab(&self.status),
            branch: self.git_ref,
            group_name: self.name.clone().unwrap_or_else(|| self.source.clone()),
            name: self.name.unwrap_or_else(|| self.source.clone()),
            pull_request: self.source == "merge_request_event",
            group: self.source,
            url: self.web_url,
            updated_at: self.updated_at,
            fork: false,
            // The list response does not say whether `ref` is a tag; `TagCache::mark` sets it.
            tag: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_host_case_port_and_trailing_slash() {
        let url = normalize_base_url(" HTTPS://GitLab.Example.com:443/gitlab/ ").unwrap();
        assert_eq!(url.as_str(), "https://gitlab.example.com/gitlab");
        let url = normalize_base_url("https://gitlab.example.com/").unwrap();
        // A host-only URL keeps the root path, which is how `Url` serializes it.
        assert_eq!(url.as_str(), "https://gitlab.example.com/");
        let url = normalize_base_url("http://gitlab.internal:8080").unwrap();
        assert_eq!(url.as_str(), "http://gitlab.internal:8080/");
    }

    #[test]
    fn rejects_bad_base_urls() {
        assert_eq!(
            normalize_base_url("gitlab.example.com"),
            Err(BaseUrlError::Invalid)
        );
        assert_eq!(
            normalize_base_url("ftp://x.test"),
            Err(BaseUrlError::Scheme)
        );
        assert_eq!(
            normalize_base_url("https://user:pw@x.test"),
            Err(BaseUrlError::UserInfo)
        );
        assert_eq!(
            normalize_base_url("https://x.test/?a=1"),
            Err(BaseUrlError::QueryOrFragment)
        );
        assert_eq!(
            normalize_base_url("https://x.test/#top"),
            Err(BaseUrlError::QueryOrFragment)
        );
    }

    #[test]
    fn same_instance_compares_equal() {
        let a = normalize_base_url("https://Git.Example.com/").unwrap();
        let b = normalize_base_url("https://git.example.com").unwrap();
        assert_eq!(a, b);
    }
}
