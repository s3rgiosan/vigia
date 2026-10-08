mod common;

use std::sync::Arc;

use vigia_lib::app::Runtime;
use vigia_lib::config::{
    Account, AccountKind, Config, ConfigStore, FilterSet, OrgFilters, Settings, WatchedRepo,
};
use vigia_lib::providers::RepoInfo;
use vigia_lib::secrets::{MemoryStore, Secrets};
use vigia_lib::settings::{
    delete_account, rename_account, set_org_filters, set_repo_branches, set_repo_ignored_workflows,
    set_repo_include_tags, set_watched, update_settings, RepoListCache, SettingsError, UNKNOWN_ORG,
};

fn runtime_with(config: Config) -> Arc<Runtime> {
    common::runtime_with_config(config)
}

fn repo(id: u64) -> RepoInfo {
    common::acme_repo(id, "github.com")
}

#[tokio::test]
async fn set_watched_replaces_the_account_set_and_keeps_overrides() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: Some(vec!["release/*".into()]),
        ignored_workflows: None,
        include_tags: None,
    });
    let runtime = runtime_with(config);
    let cache = RepoListCache::default();
    cache.set(&a.id, vec![repo(1), repo(2), repo(3)]);

    let count = set_watched(&runtime, &cache, &a.id, &[1, 3]).unwrap();
    assert_eq!(count, 2);
    let repos = runtime.with_config(|c| c.config().clone()).repos.clone();
    assert_eq!(repos.len(), 2);
    assert_eq!(
        repos[0].branch_patterns.as_deref(),
        Some(&["release/*".to_string()][..])
    );
    assert_eq!(repos[1].repo.id, 3);
}

#[tokio::test]
async fn set_watched_rejects_a_repo_watched_through_another_account() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let b = Account::new(AccountKind::GitHub, "b", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.accounts.push(b.clone());
    config.repos.push(WatchedRepo {
        account_id: b.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    let runtime = runtime_with(config);
    let cache = RepoListCache::default();
    cache.set(&a.id, vec![repo(1)]);
    let err = set_watched(&runtime, &cache, &a.id, &[1]).unwrap_err();
    assert!(matches!(err, SettingsError::AlreadyWatched(label) if label == "b"));
}

#[tokio::test]
async fn delete_account_removes_repos_token_and_cache() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    let runtime = runtime_with(config);
    runtime.secrets.set_token(&a.id, "tok").unwrap();
    let cache = RepoListCache::default();
    cache.set(&a.id, vec![repo(1)]);

    delete_account(&runtime, &cache, &a.id).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert!(config.accounts.is_empty());
    assert!(config.repos.is_empty());
    assert!(runtime.secrets.token(&a.id).is_none());
    assert!(cache.get(&a.id).is_none());
    assert!(runtime.snapshot().accounts.is_empty());
    assert!(matches!(
        delete_account(&runtime, &cache, &a.id),
        Err(SettingsError::UnknownAccount)
    ));
}

#[tokio::test]
async fn rename_and_branch_override_and_settings() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    let runtime = runtime_with(config);

    rename_account(&runtime, &a.id, "  Work  ").unwrap();
    set_repo_branches(
        &runtime,
        &a.id,
        1,
        Some(vec![" main".into(), "".into(), "release/*".into()]),
    )
    .unwrap();
    update_settings(
        &runtime,
        Settings {
            poll_interval_secs: 90,
            branch_patterns: vec!["develop ".into(), " ".into()],
            ..Default::default()
        },
    )
    .unwrap();

    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.accounts[0].label, "Work");
    assert_eq!(
        config.repos[0].branch_patterns,
        Some(vec!["main".to_string(), "release/*".to_string()])
    );
    assert_eq!(config.settings.poll_interval_secs, 90);
    assert_eq!(config.settings.branch_patterns, vec!["develop".to_string()]);

    // An explicit empty list is kept as an override: the default branch only.
    set_repo_branches(&runtime, &a.id, 1, Some(vec!["".into()])).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0].branch_patterns, Some(vec![]));

    set_repo_branches(&runtime, &a.id, 1, None).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0].branch_patterns, None);
}

#[tokio::test]
async fn blocked_keychain_marks_every_repo_as_error() {
    use vigia_lib::model::RepoStatus;
    use vigia_lib::secrets::MemoryStore;
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::failing())));
    let runtime = Arc::new(Runtime::new(
        ConfigStore::in_memory(config),
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    runtime.restart();
    let snapshot = runtime.snapshot();
    assert!(snapshot.secrets_blocked);
    assert_eq!(snapshot.repos.len(), 1);
    assert_eq!(snapshot.repos[0].state.status, RepoStatus::Error);
    assert_eq!(
        snapshot.repos[0].state.note.as_deref(),
        Some("Vigia can't read the Keychain")
    );
    assert_eq!(
        snapshot.accounts[0].error.as_deref(),
        Some("Vigia can't read the Keychain")
    );
}

#[tokio::test]
async fn set_watched_drops_removed_repos_from_the_snapshot() {
    use vigia_lib::poller::RepoSnapshot;
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    for id in [1, 2] {
        config.repos.push(WatchedRepo {
            account_id: a.id.clone(),
            repo: repo(id),
            branch_patterns: None,
            ignored_workflows: None,
            include_tags: None,
        });
    }
    let runtime = runtime_with(config);
    let state = vigia_lib::poller::RepoRuntime::new(
        WatchedRepo {
            account_id: a.id.clone(),
            repo: repo(1),
            branch_patterns: None,
            ignored_workflows: None,
            include_tags: None,
        },
        time::OffsetDateTime::now_utc(),
    )
    .state;
    for id in [1, 2] {
        runtime.store.set_repo(RepoSnapshot {
            account_id: a.id.clone(),
            repo: repo(id),
            state: state.clone(),
        });
    }
    let cache = RepoListCache::default();
    cache.set(&a.id, vec![repo(1), repo(2)]);
    set_watched(&runtime, &cache, &a.id, &[2]).unwrap();
    let ids: Vec<u64> = runtime.snapshot().repos.iter().map(|r| r.repo.id).collect();
    assert_eq!(ids, vec![2]);
}

#[tokio::test]
async fn insecure_gitlab_url_needs_explicit_consent() {
    use vigia_lib::settings::{test_connection, NewAccount};
    let input = NewAccount {
        kind: AccountKind::GitLab,
        label: "gl".into(),
        base_url: Some("http://gitlab.internal".into()),
        token: "t".into(),
        allow_insecure: false,
    };
    let err = test_connection(&input).await.unwrap_err();
    assert!(matches!(err, SettingsError::BaseUrl(_)), "{err}");
}

#[tokio::test]
async fn github_accounts_require_a_fine_grained_token() {
    use vigia_lib::settings::{test_connection, NewAccount};
    let input = NewAccount {
        kind: AccountKind::GitHub,
        label: "gh".into(),
        base_url: None,
        token: "ghp_classictoken".into(),
        allow_insecure: false,
    };
    let err = test_connection(&input).await.unwrap_err();
    assert!(matches!(err, SettingsError::TokenType), "{err}");
}

#[tokio::test]
async fn delete_all_accounts_clears_accounts_repos_tokens_and_caches() {
    use vigia_lib::settings::delete_all_accounts;
    let a = Account::new(AccountKind::GitHub, "a", None);
    let b = Account::new(
        AccountKind::GitLab,
        "b",
        Some("https://gitlab.example.test/".into()),
    );
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.accounts.push(b.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    config.settings.poll_interval_secs = 90;
    let runtime = runtime_with(config);
    runtime.secrets.set_token(&a.id, "github_pat_x").unwrap();
    runtime.secrets.set_token(&b.id, "glpat-x").unwrap();
    let cache = RepoListCache::default();
    cache.set(&a.id, vec![repo(1)]);

    let removed = delete_all_accounts(&runtime, &cache).unwrap();
    assert_eq!(removed, 2);
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert!(config.accounts.is_empty());
    assert!(config.repos.is_empty());
    // Settings are kept; only accounts go.
    assert_eq!(config.settings.poll_interval_secs, 90);
    assert!(runtime.secrets.token(&a.id).is_none());
    assert!(runtime.secrets.token(&b.id).is_none());
    assert!(cache.get(&a.id).is_none());
    let snapshot = runtime.snapshot();
    assert!(snapshot.accounts.is_empty() && snapshot.repos.is_empty());
}

#[tokio::test]
async fn unwatch_repo_removes_only_that_repo() {
    use vigia_lib::settings::{unwatch_repo, UnwatchedRepos};
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    for id in [1, 2] {
        config.repos.push(WatchedRepo {
            account_id: a.id.clone(),
            repo: repo(id),
            branch_patterns: None,
            ignored_workflows: None,
            include_tags: None,
        });
    }
    let runtime = runtime_with(config);
    unwatch_repo(&runtime, &UnwatchedRepos::default(), &a.id, 1).unwrap();
    let ids: Vec<u64> = runtime
        .with_config(|c| c.config().clone())
        .repos
        .iter()
        .map(|r| r.repo.id)
        .collect();
    assert_eq!(ids, vec![2]);
}

fn watched(account_id: &str, id: u64) -> WatchedRepo {
    common::watched_repo(account_id, repo(id), None)
}

#[tokio::test]
async fn unwatch_then_restore_round_trips_the_entry() {
    use vigia_lib::settings::{restore_watched_repo, unwatch_repo, RestoreToken, UnwatchedRepos};
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    let original = WatchedRepo {
        branch_patterns: Some(vec!["main".into(), "release/*".into()]),
        ignored_workflows: Some(vec!["Lint".into()]),
        include_tags: Some(true),
        ..watched(&a.id, 1)
    };
    config.repos.push(original.clone());
    let runtime = runtime_with(config);
    let unwatched = UnwatchedRepos::default();

    let token = unwatch_repo(&runtime, &unwatched, &a.id, 1)
        .unwrap()
        .unwrap();
    assert!(runtime.with_config(|c| c.config().clone()).repos.is_empty());
    assert_eq!(unwatch_repo(&runtime, &unwatched, &a.id, 1).unwrap(), None);

    // The token crosses the command boundary as JSON with ids only.
    let json = serde_json::to_value(&token).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "account_id": a.id, "repo_id": 1 })
    );
    let restored: RestoreToken = serde_json::from_value(json).unwrap();
    assert!(restore_watched_repo(&runtime, &unwatched, &restored).unwrap());
    assert_eq!(
        runtime.with_config(|c| c.config().clone()).repos,
        vec![original.clone()]
    );

    // Restoring twice changes nothing.
    assert!(!restore_watched_repo(&runtime, &unwatched, &restored).unwrap());
    assert_eq!(runtime.with_config(|c| c.config().clone()).repos.len(), 1);
}

#[tokio::test]
async fn an_expired_or_unknown_restore_token_restores_nothing() {
    use std::time::Duration;
    use vigia_lib::settings::{restore_watched_repo, unwatch_repo, RestoreToken, UnwatchedRepos};
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(watched(&a.id, 1));
    let runtime = runtime_with(config);

    let expired = UnwatchedRepos::with_ttl(Duration::ZERO);
    let token = unwatch_repo(&runtime, &expired, &a.id, 1).unwrap().unwrap();
    assert!(!restore_watched_repo(&runtime, &expired, &token).unwrap());

    // A token naming a repo that was never unwatched cannot add it, so the webview cannot
    // inject entries of its own.
    let unwatched = UnwatchedRepos::default();
    let forged = RestoreToken {
        account_id: a.id.clone(),
        repo_id: 99,
    };
    assert!(!restore_watched_repo(&runtime, &unwatched, &forged).unwrap());
    assert!(runtime.with_config(|c| c.config().clone()).repos.is_empty());
}

#[tokio::test]
async fn restore_skips_a_gone_account_or_a_repo_watched_elsewhere() {
    use vigia_lib::settings::{restore_watched_repo, unwatch_repo, UnwatchedRepos};
    let a = Account::new(AccountKind::GitHub, "a", None);
    let b = Account::new(AccountKind::GitHub, "b", None);
    let c = Account::new(AccountKind::GitHub, "c", None);
    let mut config = Config::default();
    config.accounts.extend([a.clone(), b.clone(), c.clone()]);
    config.repos.push(watched(&a.id, 1));
    config.repos.push(watched(&c.id, 2));
    let runtime = runtime_with(config);
    let unwatched = UnwatchedRepos::default();

    // The repo is watched again through another account of the same instance.
    let token = unwatch_repo(&runtime, &unwatched, &a.id, 1)
        .unwrap()
        .unwrap();
    runtime
        .update_config(|cfg| cfg.repos.push(watched(&b.id, 1)))
        .unwrap();
    assert!(!restore_watched_repo(&runtime, &unwatched, &token).unwrap());

    // The account is deleted after the repo was unwatched.
    let token = unwatch_repo(&runtime, &unwatched, &c.id, 2)
        .unwrap()
        .unwrap();
    runtime
        .update_config(|cfg| cfg.remove_account(&c.id))
        .unwrap();
    assert!(!restore_watched_repo(&runtime, &unwatched, &token).unwrap());

    let repos = runtime.with_config(|c| c.config().clone()).repos.clone();
    assert_eq!(repos, vec![watched(&b.id, 1)]);
}

#[tokio::test]
async fn restore_reports_a_read_only_config_and_keeps_the_entry() {
    use vigia_lib::settings::{restore_watched_repo, RestoreToken, UnwatchedRepos};
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::default())));
    let runtime = Arc::new(Runtime::new(
        ConfigStore::unavailable(&vigia_lib::config::ConfigError::ReadOnly),
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    let unwatched = UnwatchedRepos::default();
    let token = unwatched.keep(watched("a", 1));
    let err = restore_watched_repo(&runtime, &unwatched, &token).unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Config(vigia_lib::config::ConfigError::ReadOnly)
    ));
    assert_eq!(
        token,
        RestoreToken {
            account_id: "a".into(),
            repo_id: 1
        }
    );
    assert_eq!(unwatched.take(&token), Some(watched("a", 1)));
}

#[tokio::test]
async fn clear_repo_overrides_clears_all_three_in_one_restart() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use vigia_lib::settings::clear_repo_overrides;
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        branch_patterns: Some(vec!["main".into()]),
        ignored_workflows: Some(vec!["Lint".into()]),
        include_tags: Some(true),
        ..watched(&a.id, 1)
    });
    config.repos.push(WatchedRepo {
        include_tags: Some(false),
        ..watched(&a.id, 2)
    });
    let publishes = Arc::new(AtomicUsize::new(0));
    let counter = publishes.clone();
    let runtime = Arc::new(Runtime::new(
        ConfigStore::in_memory(config),
        Arc::new(Secrets::new(Box::new(MemoryStore::default()))),
        Arc::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(|_| {}),
    ));

    clear_repo_overrides(&runtime, &a.id, 1).unwrap();
    // Every restart publishes once.
    assert_eq!(publishes.load(Ordering::SeqCst), 1);
    let repos = runtime.with_config(|c| c.config().clone()).repos.clone();
    assert_eq!(repos[0], watched(&a.id, 1));
    assert_eq!(repos[1].include_tags, Some(false));

    let err = clear_repo_overrides(&runtime, "missing", 1).unwrap_err();
    assert!(matches!(err, SettingsError::UnknownAccount));
}

#[tokio::test]
async fn clear_repo_overrides_reports_a_read_only_config() {
    use vigia_lib::settings::clear_repo_overrides;
    let runtime = Arc::new(Runtime::new(
        ConfigStore::unavailable(&vigia_lib::config::ConfigError::ReadOnly),
        Arc::new(Secrets::new(Box::new(MemoryStore::default()))),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    let err = clear_repo_overrides(&runtime, "a", 1).unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Config(vigia_lib::config::ConfigError::ReadOnly)
    ));
}

#[tokio::test]
async fn update_checks_round_trip_through_update_settings_and_the_settings_view() {
    let runtime = runtime_with(Config::default());
    let view = vigia_lib::commands::settings_view(&runtime);
    assert!(view.settings.check_for_updates);
    for enabled in [false, true] {
        update_settings(
            &runtime,
            Settings {
                check_for_updates: enabled,
                ..Default::default()
            },
        )
        .unwrap();
        let json = serde_json::to_value(vigia_lib::commands::settings_view(&runtime)).unwrap();
        assert_eq!(json["settings"]["check_for_updates"], enabled);
    }
}

#[tokio::test]
async fn update_settings_clamps_the_interval() {
    let runtime = runtime_with(Config::default());
    for (given, stored) in [(1, 15), (90, 90), (86_400, 3600)] {
        update_settings(
            &runtime,
            Settings {
                poll_interval_secs: given,
                ..Default::default()
            },
        )
        .unwrap();
        let interval = runtime
            .with_config(|c| c.config().clone())
            .settings
            .poll_interval_secs;
        assert_eq!(interval, stored);
    }
}

#[tokio::test]
async fn pattern_lists_over_the_limits_are_rejected() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(watched(&a.id, 1));
    let runtime = runtime_with(config);
    let too_many: Vec<String> = (0..51).map(|i| format!("branch-{i}")).collect();
    let too_long = vec!["x".repeat(201)];

    for patterns in [too_many.clone(), too_long.clone()] {
        let err = update_settings(
            &runtime,
            Settings {
                branch_patterns: patterns.clone(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, SettingsError::InvalidInput(_)));
        let err = update_settings(
            &runtime,
            Settings {
                ignored_workflows: patterns.clone(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, SettingsError::InvalidInput(_)));
        let err = set_repo_branches(&runtime, &a.id, 1, Some(patterns.clone())).unwrap_err();
        assert!(matches!(err, SettingsError::InvalidInput(_)));
        let err = set_repo_ignored_workflows(&runtime, &a.id, 1, Some(patterns)).unwrap_err();
        assert!(matches!(err, SettingsError::InvalidInput(_)));
    }
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert!(config.settings.branch_patterns.is_empty());
    assert_eq!(config.repos[0], watched(&a.id, 1));

    // Whitespace around a pattern does not count toward its length.
    let padded = format!("  {}  ", "x".repeat(200));
    set_repo_branches(&runtime, &a.id, 1, Some(vec![padded])).unwrap();
}

type Sent = Arc<std::sync::Mutex<Vec<vigia_lib::notify::Notification>>>;

/// A runtime whose notifications are collected in the returned list.
fn runtime_collecting(config: Config, secrets: Secrets) -> (Arc<Runtime>, Sent) {
    let sent: Sent = Arc::default();
    let sink = Arc::clone(&sent);
    let runtime = Arc::new(Runtime::new(
        ConfigStore::in_memory(config),
        Arc::new(secrets),
        Arc::new(|_| {}),
        Arc::new(move |n| sink.lock().unwrap().push(n.clone())),
    ));
    (runtime, sent)
}

#[tokio::test]
async fn an_account_without_a_token_shows_an_error_without_notifying() {
    let a = Account::new(AccountKind::GitHub, "Work", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.settings.poll_interval_secs = 120;
    let (runtime, sent) =
        runtime_collecting(config, Secrets::new(Box::new(MemoryStore::default())));

    runtime.restart();
    assert!(sent.lock().unwrap().is_empty());
    let snapshot = runtime.snapshot();
    assert!(snapshot.accounts[0].auth_error);
    assert!(!snapshot.accounts[0].keychain_denied);
    assert_eq!(snapshot.accounts[0].configured_interval_secs, 120);

    rename_account(&runtime, &a.id, "Renamed").unwrap();
    assert!(sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_blocked_keychain_sets_the_typed_flag() {
    let a = Account::new(AccountKind::GitHub, "Work", None);
    let mut config = Config::default();
    config.accounts.push(a);
    let (runtime, sent) =
        runtime_collecting(config, Secrets::new(Box::new(MemoryStore::failing())));
    runtime.restart();
    let snapshot = runtime.snapshot();
    assert!(snapshot.accounts[0].keychain_denied);
    assert_eq!(
        snapshot.accounts[0].error.as_deref(),
        Some("Vigia can't read the Keychain")
    );
    assert!(sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_config_load_error_is_carried_on_the_snapshot() {
    let error = vigia_lib::config::ConfigError::ReadOnly;
    let store = ConfigStore::unavailable(&error);
    let load_error = store.load_error().map(str::to_string);
    let runtime = Arc::new(Runtime::new(
        store,
        Arc::new(Secrets::new(Box::new(MemoryStore::default()))),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    let snapshot = runtime.snapshot();
    assert!(load_error.is_some());
    assert_eq!(snapshot.config_error, load_error);
    assert!(snapshot.config_read_only);
}

#[tokio::test]
async fn ignored_workflows_are_cleaned_and_a_repo_override_may_be_empty() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    let runtime = runtime_with(config);

    update_settings(
        &runtime,
        Settings {
            ignored_workflows: vec![" Dependabot* ".into(), " ".into()],
            ..Default::default()
        },
    )
    .unwrap();
    set_repo_ignored_workflows(&runtime, &a.id, 1, Some(vec![])).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(
        config.settings.ignored_workflows,
        vec!["Dependabot*".to_string()]
    );
    assert_eq!(config.repos[0].ignored_workflows, Some(vec![]));

    set_repo_ignored_workflows(
        &runtime,
        &a.id,
        1,
        Some(vec![" Deploy* ".into(), "".into()]),
    )
    .unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(
        config.repos[0].ignored_workflows,
        Some(vec!["Deploy*".to_string()])
    );

    set_repo_ignored_workflows(&runtime, &a.id, 1, None).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0].ignored_workflows, None);
}

#[tokio::test]
async fn repo_workflow_override_reports_a_read_only_config() {
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::default())));
    let runtime = Arc::new(Runtime::new(
        ConfigStore::unavailable(&vigia_lib::config::ConfigError::ReadOnly),
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    assert!(set_repo_ignored_workflows(&runtime, "a", 1, Some(vec![])).is_err());
}

#[tokio::test]
async fn repo_tag_choice_is_stored_and_cleared() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: None,
    });
    let runtime = runtime_with(config);

    update_settings(
        &runtime,
        Settings {
            include_tags: true,
            ..Default::default()
        },
    )
    .unwrap();
    set_repo_include_tags(&runtime, &a.id, 1, Some(false)).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert!(config.settings.include_tags);
    assert_eq!(config.repos[0].include_tags, Some(false));
    let json = serde_json::to_value(&config.repos[0]).unwrap();
    assert_eq!(json["include_tags"], false);

    set_repo_include_tags(&runtime, &a.id, 1, Some(true)).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0].include_tags, Some(true));

    set_repo_include_tags(&runtime, &a.id, 1, None).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0].include_tags, None);
}

#[tokio::test]
async fn set_watched_keeps_the_repo_tag_choice() {
    let a = Account::new(AccountKind::GitHub, "a", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(WatchedRepo {
        account_id: a.id.clone(),
        repo: repo(1),
        branch_patterns: None,
        ignored_workflows: None,
        include_tags: Some(true),
    });
    let runtime = runtime_with(config);
    let cache = RepoListCache::default();
    cache.set(&a.id, vec![repo(1)]);
    set_watched(&runtime, &cache, &a.id, &[1]).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0].include_tags, Some(true));
}

#[tokio::test]
async fn repo_tag_choice_reports_a_read_only_config() {
    let secrets = Arc::new(Secrets::new(Box::new(MemoryStore::default())));
    let runtime = Arc::new(Runtime::new(
        ConfigStore::unavailable(&vigia_lib::config::ConfigError::ReadOnly),
        secrets,
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    assert!(set_repo_include_tags(&runtime, "a", 1, Some(true)).is_err());
}

fn one_account_runtime() -> (Account, Arc<Runtime>) {
    let a = Account::new(AccountKind::GitHub, "acme", None);
    let mut config = Config::default();
    config.accounts.push(a.clone());
    config.repos.push(watched(&a.id, 1));
    (a, runtime_with(config))
}

#[tokio::test]
async fn set_org_filters_cleans_and_stores_each_override() {
    let (a, runtime) = one_account_runtime();
    set_org_filters(
        &runtime,
        " GitHub.com ",
        "ACME",
        FilterSet {
            branch_patterns: Some(vec![" release/* ".into(), "".into()]),
            ignored_workflows: Some(vec![" ".into()]),
            include_tags: Some(true),
        },
    )
    .unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(
        config.organizations,
        vec![OrgFilters {
            host: "github.com".into(),
            owner: "acme".into(),
            filters: FilterSet {
                branch_patterns: Some(vec!["release/*".into()]),
                ignored_workflows: Some(vec![]),
                include_tags: Some(true),
            },
        }]
    );
    // Repo overrides are left alone.
    assert_eq!(config.repos[0], watched(&a.id, 1));

    set_org_filters(
        &runtime,
        "github.com",
        "acme",
        FilterSet {
            include_tags: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.organizations.len(), 1);
    assert_eq!(config.organizations[0].filters.include_tags, Some(false));
    assert_eq!(config.organizations[0].filters.branch_patterns, None);

    set_org_filters(&runtime, "github.com", "acme", FilterSet::default()).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert!(config.organizations.is_empty());
}

#[tokio::test]
async fn set_org_filters_rejects_long_lists_and_unknown_orgs() {
    let (_, runtime) = one_account_runtime();
    let too_many: Vec<String> = (0..51).map(|i| format!("branch-{i}")).collect();
    let too_long = vec!["x".repeat(201)];
    for patterns in [too_many, too_long] {
        let err = set_org_filters(
            &runtime,
            "github.com",
            "acme",
            FilterSet {
                branch_patterns: Some(patterns.clone()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, SettingsError::InvalidInput(_)));
        let err = set_org_filters(
            &runtime,
            "github.com",
            "acme",
            FilterSet {
                ignored_workflows: Some(patterns),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, SettingsError::InvalidInput(_)));
    }
    let only_tags = FilterSet {
        include_tags: Some(true),
        ..Default::default()
    };
    for (host, owner) in [("github.com", "example"), ("gitlab.example.test", "acme")] {
        let err = set_org_filters(&runtime, host, owner, only_tags.clone()).unwrap_err();
        assert!(
            matches!(&err, SettingsError::InvalidInput(m) if m == UNKNOWN_ORG),
            "{err:?}"
        );
    }
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert!(config.organizations.is_empty());
}

#[tokio::test]
async fn set_org_filters_updates_a_saved_org_without_watched_repos() {
    let a = Account::new(AccountKind::GitHub, "acme", None);
    let mut config = Config::default();
    config.accounts.push(a);
    config.organizations.push(OrgFilters {
        host: "github.com".into(),
        owner: "Example".into(),
        filters: FilterSet {
            include_tags: Some(true),
            ..Default::default()
        },
    });
    let runtime = runtime_with(config);
    set_org_filters(&runtime, "github.com", "example", FilterSet::default()).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert!(config.organizations.is_empty());
}

#[tokio::test]
async fn set_org_filters_reports_a_read_only_config() {
    let runtime = Arc::new(Runtime::new(
        ConfigStore::unavailable(&vigia_lib::config::ConfigError::ReadOnly),
        Arc::new(Secrets::new(Box::new(MemoryStore::default()))),
        Arc::new(|_| {}),
        Arc::new(|_| {}),
    ));
    let err = set_org_filters(&runtime, "github.com", "acme", FilterSet::default()).unwrap_err();
    assert!(matches!(
        err,
        SettingsError::Config(vigia_lib::config::ConfigError::ReadOnly)
    ));
}

#[tokio::test]
async fn repo_overrides_store_an_explicit_empty_list() {
    let (a, runtime) = one_account_runtime();
    set_repo_branches(&runtime, &a.id, 1, Some(vec![])).unwrap();
    set_repo_ignored_workflows(&runtime, &a.id, 1, Some(vec![])).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0].branch_patterns, Some(vec![]));
    assert_eq!(config.repos[0].ignored_workflows, Some(vec![]));

    set_repo_branches(&runtime, &a.id, 1, None).unwrap();
    set_repo_ignored_workflows(&runtime, &a.id, 1, None).unwrap();
    let config = runtime.with_config(|c| c.config().clone()).clone();
    assert_eq!(config.repos[0], watched(&a.id, 1));
}
