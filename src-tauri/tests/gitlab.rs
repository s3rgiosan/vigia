mod common;

use serde_json::json;
use time::macros::datetime;
use time::OffsetDateTime;
use vigia_lib::aggregate::{repo_state, select_groups, Selection};
use vigia_lib::filters::{BranchFilter, WorkflowFilter};
use vigia_lib::model::RunState;
use vigia_lib::providers::gitlab::GitLabProvider;
use vigia_lib::providers::{FetchRequest, Provider, ProviderError, RepoCache, RepoInfo};
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

const NOW: OffsetDateTime = datetime!(2026-06-01 12:00 UTC);

fn repo() -> RepoInfo {
    common::repo_info(
        77,
        "group/widgets",
        "https://gitlab.example.test/group/widgets",
    )
}

fn pipeline_json(id: u64, status: &str, source: &str, git_ref: &str) -> serde_json::Value {
    json!({
        "id": id,
        "iid": id,
        "project_id": 77,
        "status": status,
        "source": source,
        "ref": git_ref,
        "sha": "0000000000000000000000000000000000000000",
        "name": null,
        "web_url": format!("https://gitlab.example.test/group/widgets/-/pipelines/{id}"),
        "created_at": "2026-06-01T10:00:00.000Z",
        "updated_at": "2026-06-01T11:00:00.000Z"
    })
}

async fn provider(server: &MockServer) -> GitLabProvider {
    // A path prefix checks that the API path is appended to the whole base URL.
    GitLabProvider::new(&format!("{}/gitlab", server.uri()), "glpat-test").unwrap()
}

fn request<'a>(repo: &'a RepoInfo, filter: &'a BranchFilter) -> FetchRequest<'a> {
    common::fetch_request(repo, filter, NOW)
}

#[tokio::test]
async fn validate_uses_bearer_and_prefix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/user"))
        .and(header("authorization", "Bearer glpat-test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": 5, "username": "dev" })),
        )
        .mount(&server)
        .await;
    let identity = provider(&server).await.validate().await.unwrap();
    assert_eq!(identity.user_id, 5);
    assert_eq!(identity.login, "dev");
}

#[tokio::test]
async fn list_repos_uses_keyset_pagination() {
    let server = MockServer::start().await;
    let next = format!(
        "{}/gitlab/api/v4/projects?pagination=keyset&per_page=100&order_by=id&sort=asc&id_after=1",
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects"))
        .and(query_param("membership", "true"))
        .and(query_param("archived", "false"))
        .and(query_param("simple", "true"))
        .and(query_param("pagination", "keyset"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", format!("<{next}>; rel=\"next\"").as_str())
                .set_body_json(json!([
                    { "id": 1, "path_with_namespace": "group/a", "web_url": "https://gitlab.example.test/group/a", "default_branch": "main" }
                ])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects"))
        .and(query_param("id_after", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 2, "path_with_namespace": "group/b", "web_url": "https://gitlab.example.test/group/b", "default_branch": null }
        ])))
        .mount(&server)
        .await;

    let repos = provider(&server).await.list_repos(&|_| {}).await.unwrap();
    assert_eq!(repos.len(), 2);
    assert_eq!(repos[1].full_name, "group/b");
    assert_eq!(repos[1].default_branch, "");
}

#[tokio::test]
async fn fetch_maps_pipelines_and_passes_ref_for_default_filter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .and(query_param("ref", "main"))
        .and(query_param("order_by", "id"))
        .and(query_param("sort", "desc"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "W/\"p1\"")
                .insert_header("ratelimit-limit", "2000")
                .insert_header("ratelimit-remaining", "1990")
                .insert_header("ratelimit-reset", "1780000000")
                .set_body_json(json!([
                    pipeline_json(301, "failed", "push", "main"),
                    pipeline_json(
                        300,
                        "success",
                        "merge_request_event",
                        "refs/merge-requests/9/head"
                    ),
                    pipeline_json(299, "running", "schedule", "main"),
                ])),
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

    assert_eq!(outcome.counted_requests, 1);
    assert_eq!(outcome.rate_limit.unwrap().limit, 2000 * 60);
    assert!(outcome.active_groups.is_none());
    let runs = &outcome.runs;
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[0].id, 301);
    assert_eq!(runs[0].state, RunState::Failed);
    assert_eq!(runs[0].group, "push");
    assert_eq!(runs[0].name, "push");
    assert_eq!(runs[0].group_name, "push");
    assert_eq!(runs[0].attempt, 1);
    assert!(runs[1].pull_request);
    assert_eq!(runs[2].group, "schedule");
    assert_eq!(cache.runs_etag.as_deref(), Some("W/\"p1\""));
}

#[tokio::test]
async fn custom_filter_fetches_without_ref() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
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
    assert!(!received[0].url.query().unwrap_or("").contains("ref="));
}

#[tokio::test]
async fn fetch_reuses_cache_on_304() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .and(header("if-none-match", "W/\"p1\""))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;
    let repo = repo();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache {
        runs_etag: Some("W/\"p1\"".into()),
        ..Default::default()
    };
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert!(outcome.not_modified);
}

async fn fetch_error(server: &MockServer, template: ResponseTemplate) -> ProviderError {
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
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
async fn forbidden_is_reported_as_forbidden() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(403).set_body_json(json!({ "message": "403 Forbidden" })),
    )
    .await;
    assert_eq!(err, ProviderError::Forbidden);
}

#[tokio::test]
async fn not_found_is_reported_as_not_found() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(404).set_body_json(json!({ "message": "404 Project Not Found" })),
    )
    .await;
    assert_eq!(err, ProviderError::NotFound);
}

#[tokio::test]
async fn unauthorized_is_reported() {
    let server = MockServer::start().await;
    let err = fetch_error(&server, ResponseTemplate::new(401)).await;
    assert_eq!(err, ProviderError::Unauthorized);
}

#[tokio::test]
async fn too_many_requests_with_retry_after() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(429).insert_header("retry-after", "12"),
    )
    .await;
    assert_eq!(
        err,
        ProviderError::RateLimited {
            retry_after: Some(12),
            reset_at: None,
            remaining: None,
        }
    );
}

#[tokio::test]
async fn redirect_on_repo_call_is_an_error() {
    let server = MockServer::start().await;
    let err = fetch_error(
        &server,
        ResponseTemplate::new(302).insert_header("location", "https://new.example.test/"),
    )
    .await;
    assert_eq!(
        err,
        ProviderError::Redirected {
            location: "https://new.example.test/".into()
        }
    );
}

#[tokio::test]
async fn per_minute_rate_limit_headers_are_scaled_to_an_hour() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ratelimit-limit", "2000")
                .insert_header("ratelimit-remaining", "1990")
                .insert_header("ratelimit-reset", "1780000000")
                .set_body_json(json!([])),
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
    let rl = outcome.rate_limit.unwrap();
    assert_eq!(rl.limit, 2000 * 60);
    assert_eq!(rl.remaining, 1990 * 60);
}

#[tokio::test]
async fn list_repos_drops_simple_when_it_omits_fields() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects"))
        .and(query_param("simple", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 1, "path_with_namespace": "group/a", "web_url": "https://gitlab.example.test/group/a" }
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects"))
        .and(query_param_is_missing("simple"))
        .and(query_param("membership", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            { "id": 1, "path_with_namespace": "group/a", "web_url": "https://gitlab.example.test/group/a", "default_branch": "trunk" }
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let loaded = std::sync::Mutex::new(Vec::new());
    let repos = provider(&server)
        .await
        .list_repos(&|n| loaded.lock().unwrap().push(n))
        .await
        .unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].default_branch, "trunk");
    assert_eq!(*loaded.lock().unwrap(), vec![1]);
}

async fn member_check(server: &MockServer, template: ResponseTemplate) -> bool {
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77"))
        .respond_with(template)
        .expect(1)
        .mount(server)
        .await;
    provider(server).await.is_member(&repo()).await.unwrap()
}

#[tokio::test]
async fn member_with_project_or_group_access_is_a_member() {
    let server = MockServer::start().await;
    let body = json!({ "id": 77, "permissions": {
        "project_access": null,
        "group_access": { "access_level": 30, "notification_level": 3 }
    }});
    assert!(member_check(&server, ResponseTemplate::new(200).set_body_json(body)).await);
}

#[tokio::test]
async fn visible_project_without_access_levels_is_not_a_member() {
    let server = MockServer::start().await;
    let body = json!({ "id": 77, "permissions": { "project_access": null, "group_access": null } });
    assert!(!member_check(&server, ResponseTemplate::new(200).set_body_json(body)).await);
}

#[tokio::test]
async fn project_that_is_gone_is_not_a_member() {
    let server = MockServer::start().await;
    let body = json!({ "message": "404 Project Not Found" });
    assert!(!member_check(&server, ResponseTemplate::new(404).set_body_json(body)).await);
}

#[tokio::test]
async fn pipeline_name_is_the_group_name_when_present() {
    let server = MockServer::start().await;
    let mut named = pipeline_json(400, "success", "web", "main");
    named["name"] = json!("Nightly Build");
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([named])))
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
    assert_eq!(outcome.runs[0].group, "web");
    assert_eq!(outcome.runs[0].group_name, "Nightly Build");
}

// --- tag pipelines ----------------------------------------------------------

#[tokio::test]
async fn tag_pipeline_is_kept_only_with_tags_on() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .and(query_param("ref", "main"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([pipeline_json(49, "success", "push", "main"),])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .and(query_param_is_missing("ref"))
        .and(query_param("per_page", "100"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            pipeline_json(52, "failed", "push", "v1.1.0"),
            pipeline_json(51, "success", "push", "v1.0.0"),
            pipeline_json(50, "success", "push", "feature/login"),
            pipeline_json(49, "success", "push", "main"),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/repository/tags"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "W/\"t1\"")
                .set_body_json(json!([{ "name": "v1.1.0" }, { "name": "v1.0.0" }])),
        )
        .expect(1)
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
    assert_eq!(select(&off, &filter, false).groups.len(), 1);

    let mut cache = RepoCache::default();
    let on = provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    // Pipelines and the tag list; every GitLab request counts.
    assert_eq!(on.counted_requests, 2);
    let tagged: Vec<u64> = on.runs.iter().filter(|r| r.tag).map(|r| r.id).collect();
    assert_eq!(tagged, vec![52, 51]);
    let state = select(&on, &filter, true);
    assert_eq!(state.groups.len(), 2);
    let shown = state.representative.unwrap();
    assert_eq!(shown.id, 52);
    assert_eq!(shown.branch, "v1.1.0");
    assert_eq!(shown.state, RunState::Failed);

    // The next poll knows every branch name, so the tag list is not requested again.
    let again = provider
        .fetch(with_tags(&repo, &filter), &mut cache)
        .await
        .unwrap();
    assert_eq!(again.counted_requests, 1);
    assert_eq!(cache.tags.etag.as_deref(), Some("W/\"t1\""));
}

fn with_tags<'a>(repo: &'a RepoInfo, filter: &'a BranchFilter) -> FetchRequest<'a> {
    common::fetch_request_with_tags(repo, filter, NOW)
}

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
        active_groups: None,
        now: NOW,
    };
    repo_state(select_groups(&outcome.runs, &selection), NOW)
}

#[tokio::test]
async fn base_url_is_normalized_and_exposed() {
    let provider = GitLabProvider::new("https://GitLab.Example.test/prefix//", "t").unwrap();
    assert_eq!(
        provider.base_url().as_str(),
        "https://gitlab.example.test/prefix"
    );
    assert!(matches!(
        GitLabProvider::new("not a url", "t"),
        Err(ProviderError::Decode(_))
    ));
}

#[tokio::test]
async fn huge_per_minute_rate_limits_saturate_instead_of_overflowing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ratelimit-limit", u64::MAX.to_string().as_str())
                .insert_header("ratelimit-remaining", (u64::MAX / 2).to_string().as_str())
                .insert_header("ratelimit-reset", "1780000000")
                .set_body_json(json!([])),
        )
        .mount(&server)
        .await;
    let repo = repo();
    let filter = BranchFilter::Default;
    let outcome = provider(&server)
        .await
        .fetch(request(&repo, &filter), &mut RepoCache::default())
        .await
        .unwrap();
    let rate_limit = outcome.rate_limit.unwrap();
    assert_eq!(rate_limit.limit, u64::MAX);
    assert_eq!(rate_limit.remaining, u64::MAX);
}

/// A project with CI/CD disabled: pipelines answer 403 and the account is a member.
async fn mount_ci_disabled(server: &MockServer, member_checks: u64) {
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77/pipelines"))
        .respond_with(ResponseTemplate::new(403))
        .mount(server)
        .await;
    let body = json!({ "id": 77, "permissions": {
        "project_access": { "access_level": 30, "notification_level": 3 },
        "group_access": null
    }});
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .expect(member_checks)
        .mount(server)
        .await;
}

#[tokio::test]
async fn membership_is_checked_once_while_cached() {
    let server = MockServer::start().await;
    mount_ci_disabled(&server, 1).await;
    let gitlab = provider(&server).await;
    let repo = repo();
    let filter = BranchFilter::Default;
    for _ in 0..3 {
        let err = gitlab
            .fetch(request(&repo, &filter), &mut RepoCache::default())
            .await
            .unwrap_err();
        assert_eq!(err, ProviderError::Forbidden);
        assert!(gitlab.is_member(&repo).await.unwrap());
    }
    let requests = server.received_requests().await.unwrap();
    let pipelines = requests
        .iter()
        .filter(|r| r.url.path().ends_with("/pipelines"))
        .count();
    assert_eq!(pipelines, 3);
    assert_eq!(requests.len(), 4);
}

#[tokio::test]
async fn an_expired_membership_is_checked_again() {
    let server = MockServer::start().await;
    mount_ci_disabled(&server, 2).await;
    let gitlab = provider(&server)
        .await
        .with_membership_ttl(std::time::Duration::ZERO);
    assert!(gitlab.is_member(&repo()).await.unwrap());
    assert!(gitlab.is_member(&repo()).await.unwrap());
}

#[tokio::test]
async fn a_failed_membership_check_is_not_cached() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gitlab/api/v4/projects/77"))
        .respond_with(ResponseTemplate::new(500))
        .expect(2)
        .mount(&server)
        .await;
    let gitlab = provider(&server).await;
    for _ in 0..2 {
        let err = gitlab.is_member(&repo()).await.unwrap_err();
        assert_eq!(err, ProviderError::Server { status: 500 });
    }
}
