//! Account management against a mock GitLab instance.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use vigia_lib::app::Runtime;
use vigia_lib::config::{Account, AccountKind, Config, ConfigError, ConfigStore, WatchedRepo};
use vigia_lib::providers::{ProviderError, RepoInfo};
use vigia_lib::secrets::{MemoryStore, Secrets};
use vigia_lib::settings::{
    add_account, delete_account, delete_all_accounts, picker_repos, rename_account, replace_token,
    test_connection, NewAccount, RepoListCache, SettingsError,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::runtime_with;

fn gitlab_input(server: &MockServer, token: &str) -> NewAccount {
    NewAccount {
        kind: AccountKind::GitLab,
        label: "  Work  ".into(),
        base_url: Some(server.uri()),
        token: token.into(),
        allow_insecure: true,
    }
}

async fn mount_user(server: &MockServer, id: u64, token: &str) {
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .and(header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "id": id, "username": "dev" })),
        )
        .mount(server)
        .await;
}

fn project(id: u64) -> serde_json::Value {
    json!({
        "id": id,
        "path_with_namespace": format!("acme/r{id}"),
        "web_url": format!("https://gitlab.example.test/acme/r{id}"),
        "default_branch": "main"
    })
}

fn gitlab_account(server: &MockServer) -> Account {
    let mut account = Account::new(AccountKind::GitLab, "GL", Some(server.uri()));
    account.user_id = Some(5);
    account
}

#[test]
fn debug_output_redacts_the_token() {
    let input = NewAccount {
        kind: AccountKind::GitLab,
        label: "Work".into(),
        base_url: Some("https://gitlab.example.test".into()),
        token: "super-secret-token".into(),
        allow_insecure: false,
    };
    let text = format!("{input:?}");
    assert!(!text.contains("super-secret-token"), "{text}");
    assert!(text.contains("<redacted>"));
    assert!(text.contains("Work"));
}

#[tokio::test]
async fn test_connection_returns_the_identity_without_storing_anything() {
    let server = MockServer::start().await;
    mount_user(&server, 5, "glpat-ok").await;
    let identity = test_connection(&gitlab_input(&server, "glpat-ok"))
        .await
        .unwrap();
    assert_eq!(identity.user_id, 5);
    assert_eq!(identity.login, "dev");
}

#[tokio::test]
async fn test_connection_surfaces_a_rejected_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let err = test_connection(&gitlab_input(&server, "glpat-bad"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Provider(ProviderError::Unauthorized)
    ));
}

#[tokio::test]
async fn gitlab_accounts_need_a_valid_base_url() {
    let mut input = NewAccount {
        kind: AccountKind::GitLab,
        label: "gl".into(),
        base_url: None,
        token: "t".into(),
        allow_insecure: false,
    };
    let err = test_connection(&input).await.unwrap_err();
    assert!(matches!(err, SettingsError::MissingBaseUrl));
    input.base_url = Some("ftp://gitlab.example.test".into());
    let err = test_connection(&input).await.unwrap_err();
    assert!(matches!(err, SettingsError::BaseUrl(_)));
}

#[tokio::test]
async fn add_account_validates_stores_the_token_and_saves_the_account() {
    let server = MockServer::start().await;
    mount_user(&server, 5, "glpat-ok").await;
    let runtime = runtime_with(ConfigStore::in_memory(Config::default()));

    let account = add_account(&runtime, gitlab_input(&server, "glpat-ok"))
        .await
        .unwrap();
    assert_eq!(account.label, "Work");
    assert_eq!(account.user_id, Some(5));
    assert_eq!(account.login.as_deref(), Some("dev"));
    assert_eq!(
        runtime.secrets.token(&account.id).as_deref(),
        Some("glpat-ok")
    );
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.accounts.len(), 1);
    assert_eq!(config.accounts[0].id, account.id);
}

#[tokio::test]
async fn add_account_stores_nothing_when_validation_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let runtime = runtime_with(ConfigStore::in_memory(Config::default()));
    let err = add_account(&runtime, gitlab_input(&server, "glpat-bad"))
        .await
        .unwrap_err();
    assert!(matches!(err, SettingsError::Provider(_)));
    assert!(runtime
        .with_config(|c| c.config().clone())
        .accounts
        .is_empty());
}

#[tokio::test]
async fn add_account_removes_the_token_when_the_config_cannot_be_saved() {
    let server = MockServer::start().await;
    mount_user(&server, 5, "glpat-ok").await;
    let runtime = runtime_with(ConfigStore::unavailable(&ConfigError::ReadOnly));
    let err = add_account(&runtime, gitlab_input(&server, "glpat-ok"))
        .await
        .unwrap_err();
    assert!(matches!(err, SettingsError::Config(ConfigError::ReadOnly)));
    assert!(runtime
        .with_config(|c| c.config().clone())
        .accounts
        .is_empty());
    // No token is left behind: pruning against an empty keep list removes nothing.
    assert_eq!(runtime.secrets.prune(&[]).unwrap(), 0);
}

#[tokio::test]
async fn add_account_reports_a_blocked_secret_store() {
    let server = MockServer::start().await;
    mount_user(&server, 5, "glpat-ok").await;
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::failing())));
    let runtime = Arc::new(Runtime::new(
        ConfigStore::in_memory(Config::default()),
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    let err = add_account(&runtime, gitlab_input(&server, "glpat-ok"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Secrets(vigia_lib::secrets::SecretError::WriteBlocked)
    ));
    assert!(runtime
        .with_config(|c| c.config().clone())
        .accounts
        .is_empty());
}

#[tokio::test]
async fn replace_token_stores_a_token_for_the_same_user() {
    let server = MockServer::start().await;
    mount_user(&server, 5, "glpat-new").await;
    let account = gitlab_account(&server);
    let mut config = Config::default();
    config.accounts.push(account.clone());
    let runtime = runtime_with(ConfigStore::in_memory(config));
    runtime.secrets.set_token(&account.id, "glpat-old").unwrap();

    replace_token(&runtime, &account.id, "glpat-new")
        .await
        .unwrap();
    assert_eq!(
        runtime.secrets.token(&account.id).as_deref(),
        Some("glpat-new")
    );
}

#[tokio::test]
async fn replace_token_rejects_a_token_of_another_user() {
    let server = MockServer::start().await;
    mount_user(&server, 99, "glpat-other").await;
    let account = gitlab_account(&server);
    let mut config = Config::default();
    config.accounts.push(account.clone());
    let runtime = runtime_with(ConfigStore::in_memory(config));
    runtime.secrets.set_token(&account.id, "glpat-old").unwrap();

    let err = replace_token(&runtime, &account.id, "glpat-other")
        .await
        .unwrap_err();
    assert!(matches!(err, SettingsError::DifferentInstance));
    assert_eq!(
        runtime.secrets.token(&account.id).as_deref(),
        Some("glpat-old")
    );
}

#[tokio::test]
async fn replace_token_rejects_unknown_accounts_and_bad_github_tokens() {
    let runtime = runtime_with(ConfigStore::in_memory(Config::default()));
    let err = replace_token(&runtime, "missing", "t").await.unwrap_err();
    assert!(matches!(err, SettingsError::UnknownAccount));

    let github = Account::new(AccountKind::GitHub, "gh", None);
    let mut config = Config::default();
    config.accounts.push(github.clone());
    let runtime = runtime_with(ConfigStore::in_memory(config));
    let err = replace_token(&runtime, &github.id, "ghp_classic")
        .await
        .unwrap_err();
    assert!(matches!(err, SettingsError::TokenType));
}

#[tokio::test]
async fn picker_repos_lists_caches_and_refreshes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([project(1), project(2)])))
        .expect(2)
        .mount(&server)
        .await;
    let account = gitlab_account(&server);
    let mut config = Config::default();
    config.accounts.push(account.clone());
    config.repos.push(WatchedRepo {
        account_id: account.id.clone(),
        repo: RepoInfo {
            id: 1,
            full_name: "acme/r1".into(),
            web_url: "https://gitlab.example.test/acme/r1".into(),
            default_branch: "main".into(),
        },
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    let runtime = runtime_with(ConfigStore::in_memory(config));
    runtime.secrets.set_token(&account.id, "glpat-ok").unwrap();
    let cache = RepoListCache::default();

    let pages = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let on_page = {
        let pages = pages.clone();
        let seen = seen.clone();
        move |loaded: usize| {
            pages.fetch_add(1, Ordering::SeqCst);
            seen.lock().unwrap().push(loaded);
        }
    };

    let first = picker_repos(&runtime, &cache, &account.id, false, &on_page)
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    assert!(first[0].watched);
    assert!(!first[1].watched);
    assert_eq!(*seen.lock().unwrap(), vec![2]);
    assert_eq!(cache.get(&account.id).unwrap().len(), 2);

    // A cached list is served without a request and without progress callbacks.
    let cached = picker_repos(&runtime, &cache, &account.id, false, &on_page)
        .await
        .unwrap();
    assert_eq!(cached, first);
    assert_eq!(pages.load(Ordering::SeqCst), 1);

    // Refreshing fetches again (the mock expects exactly two requests).
    picker_repos(&runtime, &cache, &account.id, true, &on_page)
        .await
        .unwrap();
    assert_eq!(pages.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn picker_repos_rejects_unknown_accounts_and_missing_tokens() {
    let account = Account::new(
        AccountKind::GitLab,
        "GL",
        Some("https://gitlab.example.test".into()),
    );
    let mut config = Config::default();
    config.accounts.push(account.clone());
    let runtime = runtime_with(ConfigStore::in_memory(config));
    let cache = RepoListCache::default();

    let err = picker_repos(&runtime, &cache, "missing", false, &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, SettingsError::UnknownAccount));
    let err = picker_repos(&runtime, &cache, &account.id, false, &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Provider(ProviderError::Unauthorized)
    ));
}

#[tokio::test]
async fn rename_account_rejects_unknown_accounts() {
    let runtime = runtime_with(ConfigStore::in_memory(Config::default()));
    assert!(matches!(
        rename_account(&runtime, "missing", "x"),
        Err(SettingsError::UnknownAccount)
    ));
}

#[tokio::test]
async fn delete_account_keeps_going_when_the_token_store_is_blocked() {
    let account = Account::new(AccountKind::GitHub, "gh", None);
    let mut config = Config::default();
    config.accounts.push(account.clone());
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::failing())));
    let runtime = Arc::new(Runtime::new(
        ConfigStore::in_memory(config),
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    let cache = RepoListCache::default();
    delete_account(&runtime, &cache, &account.id).unwrap();
    assert!(runtime
        .with_config(|c| c.config().clone())
        .accounts
        .is_empty());
}

#[tokio::test]
async fn delete_all_accounts_keeps_going_when_the_token_store_is_blocked() {
    let account = Account::new(AccountKind::GitHub, "gh", None);
    let mut config = Config::default();
    config.accounts.push(account);
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::failing())));
    let runtime = Arc::new(Runtime::new(
        ConfigStore::in_memory(config),
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    let cache = RepoListCache::default();
    assert_eq!(delete_all_accounts(&runtime, &cache).unwrap(), 1);
}
