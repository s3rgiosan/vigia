mod common;

use std::collections::HashSet;

use serde_json::json;
use time::macros::datetime;
use time::OffsetDateTime;
use vigia_lib::aggregate::{repo_state, select_groups, Selection};
use vigia_lib::filters::{BranchFilter, WorkflowFilter};
use vigia_lib::model::{RepoStatus, RunState};
use vigia_lib::providers::github::{GitHubProvider, API_VERSION};
use vigia_lib::providers::{FetchRequest, Provider, ProviderError, RepoCache, RepoInfo};
use wiremock::matchers::{
    header, header_exists, method, path, query_param, query_param_is_missing,
};
use wiremock::{Mock, MockServer, ResponseTemplate};

const NOW: OffsetDateTime = datetime!(2026-06-01 12:00 UTC);

fn repo() -> RepoInfo {
    common::repo_info(42, "acme/widgets", "https://github.com/acme/widgets")
}

fn workflows_body() -> serde_json::Value {
    json!({ "workflows": [
        { "id": 100, "name": "Dependabot Updates", "state": "active" },
        { "id": 200, "name": "Old Deploy", "state": "disabled_manually" }
    ]})
}

fn run_json(id: u64, workflow_id: u64, event: &str, head_repo: Option<u64>) -> serde_json::Value {
    json!({
        "id": id,
        "run_attempt": 2,
        "status": "completed",
        "conclusion": "failure",
        "event": event,
        "head_branch": "main",
        "workflow_id": workflow_id,
        "name": "CI",
        "html_url": format!("https://github.com/acme/widgets/actions/runs/{id}"),
        "updated_at": "2026-06-01T11:00:00Z",
        "head_repository": head_repo.map(|id| json!({ "id": id })),
    })
}

async fn provider(server: &MockServer) -> GitHubProvider {
    GitHubProvider::with_base_url("test-token", &server.uri()).unwrap()
}

async fn mock_workflows(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(workflows_body()))
        .mount(server)
        .await;
}

fn request<'a>(repo: &'a RepoInfo, filter: &'a BranchFilter) -> FetchRequest<'a> {
    common::fetch_request(repo, filter, NOW)
}

#[tokio::test]
async fn validate_returns_identity_and_sends_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer test-token"))
        .and(header("x-github-api-version", API_VERSION))
        .and(header("accept", "application/vnd.github+json"))
        .and(header_exists("user-agent"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7, "login": "octo" })))
        .mount(&server)
        .await;

    let identity = provider(&server).await.validate().await.unwrap();
    assert_eq!(identity.user_id, 7);
    assert_eq!(identity.login, "octo");
}

#[tokio::test]
async fn validate_maps_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let err = provider(&server).await.validate().await.unwrap_err();
    assert_eq!(err, ProviderError::Unauthorized);
}

#[tokio::test]
async fn list_repos_follows_link_header_and_hides_archived() {
    let server = MockServer::start().await;
    let next = format!("{}/user/repos?per_page=100&page=2", server.uri());
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param("affiliation", "owner,collaborator,organization_member"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                .set_body_json(json!([
                    { "id": 1, "full_name": "acme/a", "html_url": "https://github.com/acme/a", "default_branch": "main", "archived": false },
                    { "id": 2, "full_name": "acme/old", "html_url": "https://github.com/acme/old", "default_branch": "master", "archived": true }
                ])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 3, "full_name": "acme/b", "html_url": "https://github.com/acme/b", "default_branch": "trunk" }
        ])))
        .mount(&server)
        .await;

    let loaded = std::sync::Mutex::new(Vec::new());
    let repos = provider(&server)
        .await
        .list_repos(&|n| loaded.lock().unwrap().push(n))
        .await
        .unwrap();
    let names: Vec<&str> = repos.iter().map(|r| r.full_name.as_str()).collect();
    assert_eq!(names, vec!["acme/a", "acme/b"]);
    assert_eq!(repos[1].default_branch, "trunk");
    assert_eq!(*loaded.lock().unwrap(), vec![1, 2]);
}

#[tokio::test]
async fn fetch_maps_runs_and_filters_workflows() {
    let server = MockServer::start().await;
    mock_workflows(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .and(query_param("branch", "main"))
        .and(query_param("per_page", "100"))
        .and(query_param("exclude_pull_requests", "true"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"abc\"")
                .insert_header("x-ratelimit-limit", "5000")
                .insert_header("x-ratelimit-remaining", "4990")
                .insert_header("x-ratelimit-reset", "1780000000")
                .set_body_json(json!({ "workflow_runs": [
                    run_json(10, 100, "push", Some(42)),
                    run_json(11, 100, "pull_request", Some(42)),
                    run_json(12, 100, "push", Some(99)),
                    run_json(13, 100, "push", None),
                ]})),
        )
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache::default();
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();

    assert!(!outcome.not_modified);
    assert_eq!(outcome.counted_requests, 2);
    assert_eq!(outcome.rate_limit.unwrap().remaining, 4990);
    let expected: HashSet<String> = ["100".to_string()].into_iter().collect();
    assert_eq!(outcome.active_groups, Some(expected));

    let runs = &outcome.runs;
    assert_eq!(runs.len(), 4);
    assert_eq!(runs[0].id, 10);
    assert_eq!(runs[0].attempt, 2);
    assert_eq!(runs[0].state, RunState::Failed);
    assert_eq!(runs[0].branch, "main");
    assert_eq!(runs[0].group, "100");
    assert_eq!(runs[0].name, "CI");
    assert_eq!(runs[0].group_name, "Dependabot Updates");
    assert!(!runs[0].pull_request && !runs[0].fork);
    assert!(runs[1].pull_request);
    assert!(runs[2].fork);
    assert!(runs[3].fork);

    assert_eq!(cache.runs_etag.as_deref(), Some("\"abc\""));
    assert_eq!(cache.runs.len(), 4);
    assert_eq!(cache.active_groups_fetched_at, Some(NOW));
}

#[tokio::test]
async fn custom_filter_fetches_without_branch_and_larger_page() {
    let server = MockServer::start().await;
    mock_workflows(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [] })))
        .expect(1)
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::globs(&["release/*".into()]).unwrap();
    let mut cache = RepoCache::default();
    provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    let received = server.received_requests().await.unwrap();
    let runs_request = received
        .iter()
        .find(|r| r.url.path().ends_with("/runs"))
        .unwrap();
    assert!(!runs_request.url.query().unwrap_or("").contains("branch="));
}

#[tokio::test]
async fn fetch_sends_etag_and_reuses_cache_on_304() {
    let server = MockServer::start().await;
    mock_workflows(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .and(header("if-none-match", "\"abc\""))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache {
        runs_etag: Some("\"abc\"".into()),
        runs: vec![],
        ..Default::default()
    };
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert!(outcome.not_modified);
    // The workflow list was still fetched once; the 304 itself is free.
    assert_eq!(outcome.counted_requests, 1);
}

#[tokio::test]
async fn cached_workflows_are_not_refetched_within_ttl() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(workflows_body()))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                run_json(10, 100, "push", Some(42))
            ]})),
        )
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache {
        active_groups: Some(["100".to_string()].into_iter().collect()),
        active_groups_fetched_at: Some(NOW - time::Duration::hours(1)),
        ..Default::default()
    };
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert_eq!(outcome.counted_requests, 1);
}

#[tokio::test]
async fn unknown_workflow_id_refreshes_the_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflows": [
                { "id": 100, "state": "active" }, { "id": 300, "state": "active" }
            ]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                run_json(10, 300, "push", Some(42))
            ]})),
        )
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache {
        active_groups: Some(["100".to_string()].into_iter().collect()),
        active_groups_fetched_at: Some(NOW),
        ..Default::default()
    };
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert!(outcome.active_groups.unwrap().contains("300"));
    assert_eq!(outcome.counted_requests, 2);
}

async fn fetch_error(server: &MockServer, template: ResponseTemplate) -> ProviderError {
    mock_workflows(server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(template)
        .mount(server)
        .await;
    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache::default();
    provider(server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap_err()
}

#[tokio::test]
async fn primary_rate_limit_403() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(403)
            .insert_header("x-ratelimit-limit", "5000")
            .insert_header("x-ratelimit-remaining", "0")
            .insert_header("x-ratelimit-reset", "1780000000"),
    )
    .await;
    assert_eq!(
        err,
        ProviderError::RateLimited {
            retry_after: None,
            reset_at: Some(1780000000),
            remaining: Some(0),
        }
    );
}

#[tokio::test]
async fn secondary_rate_limit_403_by_message() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(403).set_body_json(json!({
            "message": "You have exceeded a secondary rate limit. Please wait a few minutes before you try again."
        })),
    )
    .await;
    assert!(matches!(err, ProviderError::RateLimited { .. }));
}

#[tokio::test]
async fn secondary_rate_limit_reports_the_primary_window_as_not_exhausted() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(403)
            .insert_header("x-ratelimit-limit", "5000")
            .insert_header("x-ratelimit-remaining", "4000")
            .insert_header("x-ratelimit-reset", "1780000000")
            .set_body_json(json!({
                "message": "You have exceeded a secondary rate limit."
            })),
    )
    .await;
    assert_eq!(
        err,
        ProviderError::RateLimited {
            retry_after: None,
            reset_at: Some(1780000000),
            remaining: Some(4000),
        }
    );
}

#[tokio::test]
async fn rate_limit_429_with_retry_after() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(429).insert_header("retry-after", "30"),
    )
    .await;
    assert_eq!(
        err,
        ProviderError::RateLimited {
            retry_after: Some(30),
            reset_at: None,
            remaining: None,
        }
    );
}

#[tokio::test]
async fn forbidden_without_rate_limit_is_forbidden() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(403)
            .insert_header("x-ratelimit-remaining", "100")
            .insert_header("x-ratelimit-limit", "5000")
            .insert_header("x-ratelimit-reset", "1780000000")
            .set_body_json(
                json!({ "message": "Resource not accessible by personal access token" }),
            ),
    )
    .await;
    assert_eq!(err, ProviderError::Forbidden);
}

#[tokio::test]
async fn not_found_is_reported() {
    let server = MockServer::start().await;
    let err = fetch_error(&server, ResponseTemplate::new(404)).await;
    assert_eq!(err, ProviderError::NotFound);
}

#[tokio::test]
async fn server_error_is_reported() {
    let server = MockServer::start().await;
    let err = fetch_error(&server, ResponseTemplate::new(502)).await;
    assert_eq!(err, ProviderError::Server { status: 502 });
}

#[tokio::test]
async fn redirect_on_account_call_is_not_followed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(
            ResponseTemplate::new(301).insert_header("location", "https://elsewhere.test/user"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let err = provider(&server).await.validate().await.unwrap_err();
    assert_eq!(
        err,
        ProviderError::Redirected {
            location: "https://elsewhere.test/user".into()
        }
    );
}

#[tokio::test]
async fn next_link_on_another_host_is_not_followed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(
                    "link",
                    "<https://elsewhere.test/user/repos?page=2>; rel=\"next\"",
                )
                .set_body_json(json!([])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let err = provider(&server)
        .await
        .list_repos(&|_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Redirected { .. }), "{err:?}");
}

#[tokio::test]
async fn inactive_workflow_ids_do_not_refetch_the_list_every_time() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .respond_with(ResponseTemplate::new(200).set_body_json(workflows_body()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                run_json(10, 200, "push", Some(42))
            ]})),
        )
        .mount(&server)
        .await;
    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache {
        active_groups: Some(["100".to_string()].into_iter().collect()),
        active_groups_fetched_at: Some(NOW),
        ..Default::default()
    };
    let p = provider(&server).await;
    p.fetch(request(&repo, &filter), &mut cache).await.unwrap();
    p.fetch(request(&repo, &filter), &mut cache).await.unwrap();
    p.fetch(request(&repo, &filter), &mut cache).await.unwrap();
}

#[test]
fn only_fine_grained_tokens_are_accepted() {
    use vigia_lib::providers::github::is_fine_grained_token;
    assert!(is_fine_grained_token(
        "github_pat_11ABCDEF0123456789_abcdef"
    ));
    assert!(is_fine_grained_token("  github_pat_11ABCDEF  "));
    for token in ["ghp_abc", "gho_abc", "ghu_abc", "ghs_abc", "", "github_pat"] {
        assert!(!is_fine_grained_token(token), "{token}");
    }
}

#[test]
fn token_creation_url_prefills_name_permissions_and_owner() {
    use vigia_lib::providers::github::token_creation_url;
    let url = token_creation_url(Some("acme-corp")).unwrap();
    assert_eq!(url.host_str(), Some("github.com"));
    assert_eq!(url.path(), "/settings/personal-access-tokens/new");
    let pairs: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs.get("name").map(String::as_str), Some("Vigia"));
    assert_eq!(pairs.get("actions").map(String::as_str), Some("read"));
    assert_eq!(pairs.get("metadata").map(String::as_str), Some("read"));
    assert_eq!(
        pairs.get("target_name").map(String::as_str),
        Some("acme-corp")
    );
    assert_eq!(pairs.get("expires_in").map(String::as_str), Some("90"));
    assert!(pairs.get("description").is_some_and(|d| d.len() <= 1024));
    assert_eq!(pairs.len(), 6);

    let personal = token_creation_url(None).unwrap();
    assert!(!personal.query().unwrap().contains("target_name"));
    let blank = token_creation_url(Some("  ")).unwrap();
    assert!(!blank.query().unwrap().contains("target_name"));
}

#[test]
fn token_creation_url_rejects_invalid_owner_slugs() {
    use vigia_lib::providers::github::token_creation_url;
    for owner in [
        "has space",
        "a/b",
        "-leading",
        "x".repeat(40).as_str(),
        "evil&contents=write",
    ] {
        assert!(token_creation_url(Some(owner)).is_err(), "{owner}");
    }
}

#[tokio::test]
async fn workflow_list_etag_is_stored_and_a_304_keeps_cached_groups() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .and(header("if-none-match", "\"w1\""))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"w1\"")
                .set_body_json(workflows_body()),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                run_json(10, 100, "push", Some(42))
            ]})),
        )
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let p = provider(&server).await;
    let mut cache = RepoCache::default();
    let outcome = p.fetch(request(&repo, &filter), &mut cache).await.unwrap();
    assert_eq!(outcome.counted_requests, 2);
    assert_eq!(cache.groups_etag.as_deref(), Some("\"w1\""));

    // Past the TTL, the list is revalidated with the stored ETag; the 304 is free.
    let later = NOW + time::Duration::hours(7);
    let mut refresh = request(&repo, &filter);
    refresh.now = later;
    let outcome = p.fetch(refresh, &mut cache).await.unwrap();
    assert_eq!(outcome.counted_requests, 1);
    let expected: HashSet<String> = ["100".to_string()].into_iter().collect();
    assert_eq!(outcome.active_groups, Some(expected));
    assert_eq!(cache.active_groups_fetched_at, Some(later));
}

#[tokio::test]
async fn multi_page_workflow_list_keeps_no_etag() {
    let server = MockServer::start().await;
    let next = format!(
        "{}/repositories/42/actions/workflows?per_page=100&page=2",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "workflows": [{ "id": 300, "state": "active" }] })),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"w1\"")
                .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                .set_body_json(workflows_body()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [] })))
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache::default();
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert_eq!(outcome.counted_requests, 3);
    assert!(outcome.active_groups.unwrap().contains("300"));
    assert_eq!(cache.groups_etag, None);
}

// --- tag runs ---------------------------------------------------------------

fn release_run(id: u64, event: &str, head_branch: &str) -> serde_json::Value {
    json!({
        "id": id,
        "run_attempt": 1,
        "status": "completed",
        "conclusion": "success",
        "event": event,
        "head_branch": head_branch,
        "workflow_id": 500,
        "name": "Release",
        "html_url": format!("https://github.com/acme/widgets/actions/runs/{id}"),
        "updated_at": "2026-06-01T11:00:00Z",
        "head_repository": { "id": 42 },
    })
}

fn tags_body(names: &[&str]) -> serde_json::Value {
    serde_json::Value::Array(
        names
            .iter()
            .map(|n| json!({ "name": n, "commit": { "sha": "0000000000000000000000000000000000000000" } }))
            .collect(),
    )
}

async fn mock_release_workflow(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/workflows"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflows": [
                { "id": 500, "name": "Release", "state": "active" }
            ]})),
        )
        .mount(server)
        .await;
}

fn with_tags<'a>(repo: &'a RepoInfo, filter: &'a BranchFilter) -> FetchRequest<'a> {
    common::fetch_request_with_tags(repo, filter, NOW)
}

/// Runs a fetch through selection with the given tag setting.
fn select(
    outcome: &vigia_lib::providers::FetchOutcome,
    filter: &BranchFilter,
    include_tags: bool,
) -> vigia_lib::model::RepoState {
    let ignore_nothing = WorkflowFilter::default();
    let selection = Selection {
        default_branch: "main",
        branch_filter: filter,
        workflow_filter: &ignore_nothing,
        exclude_pull_requests: true,
        include_tags,
        active_groups: outcome.active_groups.as_ref(),
        now: NOW,
    };
    repo_state(select_groups(&outcome.runs, &selection), NOW)
}

async fn tag_requests(server: &MockServer) -> Vec<Option<String>> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/repositories/42/tags")
        .map(|r| {
            r.headers
                .get("if-none-match")
                .map(|v| v.to_str().unwrap().to_string())
        })
        .collect()
}

#[tokio::test]
async fn release_only_repo_shows_its_tag_run_only_with_tags_on() {
    let server = MockServer::start().await;
    mock_release_workflow(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .and(query_param("branch", "main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .and(query_param_is_missing("branch"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                release_run(905, "release", "1.7.0"),
                release_run(900, "push", "1.6.0"),
            ]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/tags"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tags_body(&["1.7.0", "1.6.0"])))
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let provider = provider(&server).await;

    let mut cache = RepoCache::default();
    let off = provider
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    let state = select(&off, &filter, false);
    assert_eq!(state.status, RepoStatus::None);
    assert!(state.groups.is_empty());
    assert!(tag_requests(&server).await.is_empty());

    let mut cache = RepoCache::default();
    let on = provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert!(on.runs.iter().all(|r| r.tag));
    // Workflows, runs and the tag list.
    assert_eq!(on.counted_requests, 3);
    let state = select(&on, &filter, true);
    assert_eq!(state.status, RepoStatus::Success);
    assert_eq!(state.groups.len(), 1);
    let shown = state.representative.unwrap();
    assert_eq!(shown.id, 905);
    assert_eq!(shown.branch, "1.7.0");
    assert_eq!(shown.name, "Release");
}

#[tokio::test]
async fn tag_list_uses_its_etag_and_is_refetched_only_for_an_unknown_branch() {
    let server = MockServer::start().await;
    mock_release_workflow(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"t1\"")
                .set_body_json(tags_body(&["1.6.0"])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/tags"))
        .and(header("if-none-match", "\"t1\""))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"t2\"")
                .set_body_json(tags_body(&["1.7.0", "1.6.0"])),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/tags"))
        .and(header("if-none-match", "\"t2\""))
        .respond_with(ResponseTemplate::new(304))
        .with_priority(1)
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let provider = provider(&server).await;
    let mut cache = RepoCache::default();
    let runs_returning = |runs: Vec<serde_json::Value>| {
        Mock::given(method("GET"))
            .and(path("/repositories/42/actions/runs"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": runs })),
            )
    };

    // First poll: the list is unknown, so it is fetched without an ETag.
    let guard = server
        .register_as_scoped(runs_returning(vec![release_run(900, "push", "1.6.0")]))
        .await;
    let outcome = provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert!(outcome.runs[0].tag);
    assert_eq!(tag_requests(&server).await, vec![None]);
    assert_eq!(cache.tags.etag.as_deref(), Some("\"t1\""));

    // Second poll: the same tag is known, so the list is not requested.
    let outcome = provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert_eq!(outcome.counted_requests, 1);
    assert_eq!(tag_requests(&server).await.len(), 1);
    drop(guard);

    // Third poll: a new tag name refreshes the list with the cached ETag.
    let guard = server
        .register_as_scoped(runs_returning(vec![
            release_run(901, "push", "1.7.0"),
            release_run(900, "push", "1.6.0"),
        ]))
        .await;
    let outcome = provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert!(outcome.runs.iter().all(|r| r.tag));
    assert_eq!(outcome.counted_requests, 2);
    assert_eq!(
        tag_requests(&server).await,
        vec![None, Some("\"t1\"".to_string())]
    );
    drop(guard);

    // Fourth poll: an unknown name answered by 304 keeps the list, counts nothing for it, and
    // is not checked again on the fifth poll.
    let _guard = server
        .register_as_scoped(runs_returning(vec![release_run(902, "push", "hotfix")]))
        .await;
    let outcome = provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert!(!outcome.runs[0].tag);
    assert_eq!(outcome.counted_requests, 1);
    assert_eq!(tag_requests(&server).await.len(), 3);
    assert!(cache.tags.names.as_ref().unwrap().contains("1.7.0"));
    provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert_eq!(tag_requests(&server).await.len(), 3);
}

#[tokio::test]
async fn a_feature_branch_does_not_refetch_the_tag_list_every_poll() {
    let server = MockServer::start().await;
    mock_release_workflow(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                release_run(900, "push", "feature/login"),
                release_run(899, "push", "main"),
            ]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tags_body(&["1.0.0"])))
        .expect(1)
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let provider = provider(&server).await;
    let mut cache = RepoCache::default();
    for _ in 0..3 {
        let outcome = provider
            .fetch(with_tags(&repo, &filter), &mut cache)
            .await
            .unwrap();
        assert!(outcome.runs.iter().all(|r| !r.tag));
    }
    assert!(cache.tags.not_tags.contains("feature/login"));
}

#[tokio::test]
async fn an_unreadable_tag_list_counts_as_no_tags() {
    let server = MockServer::start().await;
    mock_release_workflow(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                release_run(900, "push", "1.6.0"),
            ]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/tags"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let provider = provider(&server).await;
    let mut cache = RepoCache::default();
    for _ in 0..2 {
        let outcome = provider
            .fetch(with_tags(&repo, &filter), &mut cache)
            .await
            .unwrap();
        assert!(!outcome.runs[0].tag);
    }
}

#[tokio::test]
async fn runs_without_attempt_or_name_default_sensibly_and_unknown_workflows_use_their_id() {
    let server = MockServer::start().await;
    mock_workflows(&server).await;
    Mock::given(method("GET"))
        .and(path("/repositories/42/actions/runs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "workflow_runs": [
                {
                    "id": 50,
                    "status": "completed",
                    "conclusion": "success",
                    "event": "push",
                    "head_branch": "main",
                    "workflow_id": 777,
                    "name": null,
                    "html_url": "https://github.com/acme/widgets/actions/runs/50",
                    "updated_at": "2026-06-01T11:00:00Z",
                    "head_repository": { "id": 42 }
                }
            ]})),
        )
        .mount(&server)
        .await;

    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache::default();
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    let run = &outcome.runs[0];
    assert_eq!(run.attempt, 1);
    assert_eq!(run.name, "777");
    assert_eq!(run.group_name, "777");
}
