//! Debug-only bootstrap: `VIGIA_DEBUG_SETUP` describes accounts and repos to add at launch.
//!
//! ```text
//! VIGIA_DEBUG_SETUP='{"accounts":[
//!   {"kind":"github","label":"gh","token_env":"VIGIA_GITHUB_TOKEN","repos":["owner/name"]},
//!   {"kind":"gitlab","label":"gl","base_url":"https://gitlab.example.com",
//!    "token_env":"VIGIA_GITLAB_TOKEN","repos":["group/project"]}
//! ]}'
//! ```
//!
//! Accounts with a label that already exists are skipped, so the app can be relaunched.

use std::sync::Arc;

use serde::Deserialize;

use crate::app::Runtime;
use crate::config::{Account, AccountKind, WatchedRepo};
use crate::providers::gitlab::normalize_base_url;

#[derive(Deserialize)]
struct Setup {
    accounts: Vec<SetupAccount>,
}

#[derive(Deserialize)]
struct SetupAccount {
    kind: AccountKind,
    label: String,
    #[serde(default)]
    base_url: Option<String>,
    token_env: String,
    #[serde(default)]
    repos: Vec<String>,
}

pub async fn apply(runtime: &Arc<Runtime>) {
    let Ok(raw) = std::env::var("VIGIA_DEBUG_SETUP") else {
        return;
    };
    let setup: Setup = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => {
            log::error!("VIGIA_DEBUG_SETUP is not valid JSON: {e}");
            return;
        }
    };

    for entry in setup.accounts {
        let Ok(token) = std::env::var(&entry.token_env) else {
            log::error!("debug setup: {} is not set", entry.token_env);
            continue;
        };
        let existing = runtime.with_config(|c| {
            c.config()
                .accounts
                .iter()
                .find(|a| a.label == entry.label)
                .cloned()
        });
        if let Some(account) = existing {
            // Relaunch: only restore the token when the store lost it.
            if runtime.secrets.token(&account.id).is_none() {
                if let Err(e) = runtime.secrets.set_token(&account.id, &token) {
                    log::error!("debug setup: cannot store token: {e}");
                }
            }
            continue;
        }
        let base_url = match (entry.kind, entry.base_url.as_deref()) {
            (AccountKind::GitLab, Some(url)) => match normalize_base_url(url) {
                Ok(u) => Some(u.to_string()),
                Err(e) => {
                    log::error!("debug setup: bad base url for {}: {e}", entry.label);
                    continue;
                }
            },
            (AccountKind::GitLab, None) => {
                log::error!("debug setup: gitlab account {} needs base_url", entry.label);
                continue;
            }
            (AccountKind::GitHub, _) => None,
        };

        let mut account = Account::new(entry.kind, &entry.label, base_url);
        if let Err(e) = runtime.secrets.set_token(&account.id, &token) {
            log::error!("debug setup: cannot store token: {e}");
            continue;
        }
        let provider = match runtime.provider_for(&account) {
            Ok(p) => p,
            Err(e) => {
                log::error!("debug setup: {e}");
                continue;
            }
        };
        match provider.validate().await {
            Ok(identity) => {
                account.user_id = Some(identity.user_id);
                account.login = Some(identity.login);
            }
            Err(e) => {
                log::error!("debug setup: token for {} rejected: {e}", entry.label);
                continue;
            }
        }
        let repos = match provider.list_repos(&|_| {}).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("debug setup: list repos for {} failed: {e}", entry.label);
                continue;
            }
        };
        let watched: Vec<WatchedRepo> = repos
            .into_iter()
            .filter(|r| entry.repos.iter().any(|name| name == &r.full_name))
            .map(|repo| WatchedRepo {
                account_id: account.id.clone(),
                repo,
                branch_patterns: None,
                ignored_workflows: None,
                include_tags: None,
            })
            .collect();
        log::info!(
            "debug setup: adding {} with {} repos",
            entry.label,
            watched.len()
        );
        let result = runtime.update_config(|c| {
            c.accounts.push(account.clone());
            c.repos.extend(watched.clone());
        });
        if let Err(e) = result {
            log::error!("debug setup: config write failed: {e}");
        }
    }
}
