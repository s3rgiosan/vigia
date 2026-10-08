//! GitHub account paths, against a local server standing in for the GitHub API.

use serde_json::json;
use vigia_lib::config::AccountKind;
use vigia_lib::settings::{test_connection_against, NewAccount};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn a_fine_grained_github_token_is_trimmed_and_validated() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer github_pat_exampletoken"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7, "login": "dev" })))
        .expect(1)
        .mount(&server)
        .await;
    let input = NewAccount {
        kind: AccountKind::GitHub,
        label: "gh".into(),
        base_url: None,
        token: "  github_pat_exampletoken ".into(),
        allow_insecure: false,
    };
    let identity = test_connection_against(&input, &server.uri())
        .await
        .unwrap();
    assert_eq!(identity.user_id, 7);
    assert_eq!(identity.login, "dev");
}
