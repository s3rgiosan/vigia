//! Provider interface shared by GitHub and GitLab.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;

use crate::filters::BranchFilter;
use crate::http::{HttpClient, RateLimit};
use crate::model::Run;

pub mod github;
pub mod gitlab;

/// Identity returned by token validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountIdentity {
    pub user_id: u64,
    pub login: String,
}

/// One repo as listed by a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    pub id: u64,
    pub full_name: String,
    pub web_url: String,
    pub default_branch: String,
}

/// Per-repo cache kept between polls: ETags, the last parsed runs, and the active group list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoCache {
    pub runs_etag: Option<String>,
    pub runs: Vec<Run>,
    pub groups_etag: Option<String>,
    /// Workflow names by ID on GitHub, from the same list as `active_groups`.
    #[serde(default)]
    pub workflow_names: HashMap<String, String>,
    /// Active workflow IDs on GitHub. GitLab does not use it.
    pub active_groups: Option<HashSet<String>>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub active_groups_fetched_at: Option<OffsetDateTime>,
    /// Group IDs that already triggered a list refresh; inactive workflows stay here.
    #[serde(default)]
    pub refreshed_for: HashSet<String>,
    #[serde(default)]
    pub tags: TagCache,
}

/// A repo's tag names, used to mark runs whose branch is a tag.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagCache {
    /// ETag of the tag list, kept only when the list fits one page.
    pub etag: Option<String>,
    /// `None` until the list has been fetched.
    pub names: Option<HashSet<String>>,
    /// Branch names already checked against the list and found not to be tags, so a feature
    /// branch triggers one list refresh, not one per poll. A refresh drops the names that turn
    /// out to be tags and keeps the rest.
    pub not_tags: HashSet<String>,
}

impl TagCache {
    /// Branch names of `runs` that the cached list cannot classify yet. The default branch,
    /// pull request runs and fork runs are skipped: their branches are never tags.
    pub fn unknown_refs(&self, runs: &[Run], default_branch: &str) -> HashSet<String> {
        runs.iter()
            .filter(|r| !r.pull_request && !r.fork)
            .map(|r| r.branch.as_str())
            .filter(|b| !b.is_empty() && *b != default_branch)
            .filter(|b| !self.names.as_ref().is_some_and(|n| n.contains(*b)))
            .filter(|b| !self.not_tags.contains(*b))
            .map(str::to_string)
            .collect()
    }

    /// Sets each run's `tag` flag from the cached list.
    pub fn mark(&self, runs: &mut [Run]) {
        for run in runs {
            run.tag = self.names.as_ref().is_some_and(|n| n.contains(&run.branch));
        }
    }
}

/// Requests one tag list refresh sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TagRefresh {
    pub requests: u32,
    /// The first page came back as 304 and the cached list was kept.
    pub not_modified: bool,
}

#[derive(Deserialize)]
struct TagEntry {
    name: String,
}

/// Most tag list pages read in one refresh. Names past the last page read count as branches.
pub const MAX_TAG_PAGES: u32 = 10;

/// Fetches a tag list from `url`, following up to `MAX_TAG_PAGES` pages, and records `checked`
/// names that are not tags. The cached ETag is sent with the first page and kept only when the
/// list fits one page; a 304 keeps the cached list. A 403 or 404 stores an empty list, so the
/// checked names count as branches until the next refresh.
pub(crate) async fn refresh_tags(
    http: &HttpClient,
    mut url: Url,
    headers: &HeaderMap,
    base_url: &Url,
    tags: &mut TagCache,
    checked: HashSet<String>,
) -> Result<TagRefresh, ProviderError> {
    let cached_etag = tags.names.as_ref().and(tags.etag.clone());
    let mut requests = 0;
    let mut names = HashSet::new();
    let mut first_etag = None;
    loop {
        let etag = if requests == 0 {
            cached_etag.as_deref()
        } else {
            None
        };
        let result = http.get(&url, headers, etag).await;
        requests += 1;
        let response = match result {
            Ok(response) => response,
            Err(ProviderError::Forbidden | ProviderError::NotFound) => {
                log::info!("tag list unavailable; treating the repo as having no tags");
                tags.names = Some(HashSet::new());
                tags.etag = None;
                tags.not_tags.extend(checked);
                return Ok(TagRefresh {
                    requests,
                    not_modified: false,
                });
            }
            Err(e) => return Err(e),
        };
        if response.not_modified() {
            tags.not_tags.extend(checked);
            return Ok(TagRefresh {
                requests,
                not_modified: true,
            });
        }
        if requests == 1 {
            first_etag = response.etag();
        }
        let page: Vec<TagEntry> = response.json()?;
        names.extend(page.into_iter().map(|t| t.name));
        let Some(next) = response.next_link_on(base_url)? else {
            break;
        };
        if requests >= MAX_TAG_PAGES {
            log::info!("tag list longer than {MAX_TAG_PAGES} pages; the rest is not read");
            break;
        }
        url = next;
    }
    tags.etag = if requests == 1 { first_etag } else { None };
    tags.not_tags.extend(checked);
    tags.not_tags.retain(|b| !names.contains(b));
    tags.names = Some(names);
    Ok(TagRefresh {
        requests,
        not_modified: false,
    })
}

/// A membership answer and the requests it took; a cached answer takes none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Membership {
    pub member: bool,
    pub requests: u32,
}

/// Result of one repo fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutcome {
    pub runs: Vec<Run>,
    pub active_groups: Option<HashSet<String>>,
    /// The runs came back as 304 and were reused from the cache.
    pub not_modified: bool,
    /// Requests that counted against the rate limit.
    pub counted_requests: u32,
    pub rate_limit: Option<RateLimit>,
}

/// What the poller asks a provider to fetch.
#[derive(Debug, Clone, Copy)]
pub struct FetchRequest<'a> {
    pub repo: &'a RepoInfo,
    pub branch_filter: &'a BranchFilter,
    pub exclude_pull_requests: bool,
    pub now: OffsetDateTime,
    /// Fetch the active group list again even when the cached one is fresh.
    pub refresh_groups: bool,
    /// Tag runs count for this repo: runs are fetched without a branch restriction and marked
    /// from the tag list.
    pub include_tags: bool,
}

/// Commands send it to the frontend as a `CommandError` carrying its kind and this text.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProviderError {
    #[error("token rejected")]
    Unauthorized,
    #[error("rate limited")]
    RateLimited {
        /// Seconds to wait when the server said so.
        retry_after: Option<u64>,
        /// Unix time when the primary window resets.
        reset_at: Option<i64>,
        /// Requests left in the primary window. Zero means the primary limit is exhausted.
        remaining: Option<u64>,
    },
    #[error("not found")]
    NotFound,
    #[error("no access")]
    Forbidden,
    #[error("the server redirected to {location}; update the base URL")]
    Redirected { location: String },
    #[error("server error {status}")]
    Server { status: u16 },
    #[error("network error: {0}")]
    Network(String),
    #[error("unexpected response: {0}")]
    Decode(String),
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn validate(&self) -> Result<AccountIdentity, ProviderError>;

    /// Lists every repo the token can see, following pagination. `on_page` receives the number
    /// of repos loaded so far after each page.
    async fn list_repos(
        &self,
        on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Vec<RepoInfo>, ProviderError>;

    /// Whether the account is still a member of `repo`, so it would appear in `list_repos`.
    /// GitLab uses it to tell a project with CI/CD disabled from one the token lost access to.
    async fn is_member(&self, _repo: &RepoInfo) -> Result<bool, ProviderError> {
        Ok(true)
    }

    /// `is_member` together with the requests the answer cost. Without an override the answer
    /// counts as one request.
    async fn membership(&self, repo: &RepoInfo) -> Result<Membership, ProviderError> {
        let member = self.is_member(repo).await?;
        Ok(Membership {
            member,
            requests: 1,
        })
    }

    /// Fetches the runs of one repo. The cache is read for ETags and updated in place.
    async fn fetch(
        &self,
        request: FetchRequest<'_>,
        cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn names(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn base(server: &MockServer) -> Url {
        Url::parse(&server.uri()).unwrap()
    }

    async fn refresh(
        server: &MockServer,
        tags: &mut TagCache,
        checked: &[&str],
    ) -> Result<TagRefresh, ProviderError> {
        let http = HttpClient::new("tok").unwrap();
        let url = base(server).join("/tags").unwrap();
        refresh_tags(
            &http,
            url,
            &HeaderMap::new(),
            &base(server),
            tags,
            names(checked),
        )
        .await
    }

    #[tokio::test]
    async fn tag_pages_are_followed_and_merged_without_an_etag() {
        let server = MockServer::start().await;
        let next = format!("{}/tags2", server.uri());
        Mock::given(method("GET"))
            .and(path("/tags"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"one\"")
                    .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                    .set_body_json(json!([{ "name": "v1" }])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tags2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "name": "v2" }])))
            .mount(&server)
            .await;

        let mut tags = TagCache::default();
        let refreshed = refresh(&server, &mut tags, &["v1", "main"]).await.unwrap();
        assert_eq!(refreshed.requests, 2);
        assert!(!refreshed.not_modified);
        assert_eq!(tags.names, Some(names(&["v1", "v2"])));
        assert_eq!(tags.not_tags, names(&["main"]));
        assert_eq!(tags.etag, None);
    }

    #[tokio::test]
    async fn a_single_tag_page_keeps_its_etag_and_sends_it_next_time() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tags"))
            .and(header("if-none-match", "\"one\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tags"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"one\"")
                    .set_body_json(json!([{ "name": "v1" }])),
            )
            .mount(&server)
            .await;

        let mut tags = TagCache::default();
        refresh(&server, &mut tags, &["v1"]).await.unwrap();
        assert_eq!(tags.etag.as_deref(), Some("\"one\""));

        let again = refresh(&server, &mut tags, &["dev"]).await.unwrap();
        assert!(again.not_modified);
        assert_eq!(tags.names, Some(names(&["v1"])));
        assert_eq!(tags.not_tags, names(&["dev"]));
    }

    #[tokio::test]
    async fn an_unavailable_tag_list_counts_every_checked_name_as_a_branch() {
        for status in [403, 404] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let mut tags = TagCache {
                etag: Some("old".into()),
                names: Some(names(&["v1"])),
                not_tags: HashSet::new(),
            };
            let refreshed = refresh(&server, &mut tags, &["v1"]).await.unwrap();
            assert_eq!(refreshed.requests, 1);
            assert_eq!(tags.names, Some(HashSet::new()));
            assert_eq!(tags.etag, None);
            assert_eq!(tags.not_tags, names(&["v1"]));
        }
    }

    #[tokio::test]
    async fn tag_pagination_stops_after_the_page_cap() {
        let server = MockServer::start().await;
        let next = format!("{}/tags", server.uri());
        Mock::given(method("GET"))
            .and(path("/tags"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"loop\"")
                    .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                    .set_body_json(json!([{ "name": "v1" }])),
            )
            .expect(u64::from(MAX_TAG_PAGES))
            .mount(&server)
            .await;

        let mut tags = TagCache::default();
        let refreshed = refresh(&server, &mut tags, &["v1", "feature"])
            .await
            .unwrap();
        assert_eq!(refreshed.requests, MAX_TAG_PAGES);
        assert_eq!(tags.names, Some(names(&["v1"])));
        assert_eq!(tags.not_tags, names(&["feature"]));
        assert_eq!(tags.etag, None);
    }

    #[tokio::test]
    async fn a_new_tag_list_keeps_known_branches_and_drops_new_tags() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/tags"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{ "name": "v1" }, { "name": "v2" }])),
            )
            .mount(&server)
            .await;

        let mut tags = TagCache {
            etag: None,
            names: Some(names(&["v1"])),
            not_tags: names(&["feature", "v2"]),
        };
        refresh(&server, &mut tags, &["topic"]).await.unwrap();
        assert_eq!(tags.names, Some(names(&["v1", "v2"])));
        assert_eq!(tags.not_tags, names(&["feature", "topic"]));
    }

    #[tokio::test]
    async fn other_tag_list_errors_are_returned() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let mut tags = TagCache::default();
        let err = refresh(&server, &mut tags, &[]).await.unwrap_err();
        assert_eq!(err, ProviderError::Server { status: 500 });
        assert!(tags.names.is_none());
    }
}
