//! Prints the state of one real repo. Reads the token from an environment variable.
//!
//! GitHub: `VIGIA_GITHUB_TOKEN=... cargo run --example probe -- github owner/name`
//! GitLab: `VIGIA_GITLAB_URL=https://gitlab.example.com VIGIA_GITLAB_TOKEN=... cargo run --example probe -- gitlab group/name`
//! Set `VIGIA_INCLUDE_TAGS=1` to count tag runs.

use std::collections::HashSet;

use time::OffsetDateTime;
use vigia_lib::aggregate::{repo_state, select_groups, Selection};
use vigia_lib::filters::BranchFilter;
use vigia_lib::providers::github::GitHubProvider;
use vigia_lib::providers::gitlab::GitLabProvider;
use vigia_lib::providers::{FetchRequest, Provider, RepoCache};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (kind, target) = match args.as_slice() {
        [kind, target] => (kind.as_str(), target.as_str()),
        _ => {
            eprintln!("usage: probe github <owner/name> | probe gitlab <group/name>");
            std::process::exit(2);
        }
    };

    let provider: Box<dyn Provider> = match kind {
        "github" => {
            let token = std::env::var("VIGIA_GITHUB_TOKEN").expect("VIGIA_GITHUB_TOKEN is not set");
            Box::new(GitHubProvider::new(&token).expect("client"))
        }
        "gitlab" => {
            let base = std::env::var("VIGIA_GITLAB_URL").expect("VIGIA_GITLAB_URL is not set");
            let token = std::env::var("VIGIA_GITLAB_TOKEN").expect("VIGIA_GITLAB_TOKEN is not set");
            Box::new(GitLabProvider::new(&base, &token).expect("client"))
        }
        other => {
            eprintln!("unknown provider {other}");
            std::process::exit(2);
        }
    };

    let identity = provider.validate().await.expect("validate");
    println!("account: {} (id {})", identity.login, identity.user_id);

    let repos = provider
        .list_repos(&|loaded| eprintln!("loaded {loaded} repos"))
        .await
        .expect("list repos");
    println!("repos visible: {}", repos.len());
    let Some(repo) = repos.into_iter().find(|r| r.full_name == target) else {
        eprintln!("repo {target} is not in the list");
        std::process::exit(1);
    };

    let now = OffsetDateTime::now_utc();
    let filter = BranchFilter::Default;
    let mut cache = RepoCache::default();
    let include_tags = std::env::var("VIGIA_INCLUDE_TAGS").is_ok_and(|v| v == "1");
    let request = FetchRequest {
        repo: &repo,
        branch_filter: &filter,
        exclude_pull_requests: true,
        now,
        refresh_groups: false,
        include_tags,
    };
    let outcome = provider.fetch(request, &mut cache).await.expect("fetch");
    println!(
        "runs: {} (counted requests {}, rate limit {:?})",
        outcome.runs.len(),
        outcome.counted_requests,
        outcome.rate_limit
    );

    let active: Option<HashSet<String>> = outcome.active_groups.clone();
    let selection = Selection {
        default_branch: &repo.default_branch,
        branch_filter: &filter,
        workflow_filter: &Default::default(),
        exclude_pull_requests: true,
        include_tags,
        active_groups: active.as_ref(),
        now,
    };
    let groups = select_groups(&outcome.runs, &selection);
    let state = repo_state(groups, now);
    println!("status: {:?}", state.status);
    for run in &state.groups {
        println!(
            "  {:?}  {} / {}  #{} attempt {}  {}",
            run.state, run.branch, run.name, run.id, run.attempt, run.url
        );
    }
}
