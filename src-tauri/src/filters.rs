//! Branch filters (the repo default branch, or a list of glob patterns), workflow filters, and
//! their resolution from repo, organization and global settings.

use std::collections::HashMap;
use std::sync::Arc;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::config::{Account, Config, FilterSet, OrgFilters, OrgKey, Settings, WatchedRepo};

#[derive(Debug, Clone)]
pub enum BranchFilter {
    Default,
    Globs { patterns: Vec<String>, set: GlobSet },
}

impl BranchFilter {
    /// Builds a glob filter. A `/` in a branch name is a literal separator, so `release/*` does
    /// not match `release/1.0/hotfix`.
    pub fn globs(patterns: &[String]) -> Result<BranchFilter, globset::Error> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = GlobBuilder::new(pattern).literal_separator(true).build()?;
            builder.add(glob);
        }
        let set = builder.build()?;
        Ok(BranchFilter::Globs {
            patterns: patterns.to_vec(),
            set,
        })
    }

    pub fn matches(&self, branch: &str, default_branch: &str) -> bool {
        match self {
            BranchFilter::Default => branch == default_branch,
            BranchFilter::Globs { set, .. } => set.is_match(branch),
        }
    }

    pub fn patterns(&self) -> &[String] {
        match self {
            BranchFilter::Default => &[],
            BranchFilter::Globs { patterns, .. } => patterns,
        }
    }
}

/// Case-insensitive glob patterns naming workflows to ignore. No patterns ignores nothing.
#[derive(Debug, Clone, Default)]
pub struct WorkflowFilter {
    set: GlobSet,
}

impl WorkflowFilter {
    pub fn globs(patterns: &[String]) -> Result<WorkflowFilter, globset::Error> {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            let glob = GlobBuilder::new(pattern).case_insensitive(true).build()?;
            builder.add(glob);
        }
        Ok(WorkflowFilter {
            set: builder.build()?,
        })
    }

    pub fn ignores(&self, workflow_name: &str) -> bool {
        self.set.is_match(workflow_name)
    }
}

/// A compiled filter, or the error that kept its patterns from compiling.
pub type Compiled<T> = Arc<Result<T, globset::Error>>;

/// The filter values one repo is selected with. Each value is the repo's override, else its
/// organization's, else the global setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveFilters<'a> {
    /// Branch glob patterns; empty means the default branch only.
    pub branch_patterns: &'a [String],
    /// Workflow glob patterns to ignore; empty ignores nothing.
    pub ignored_workflows: &'a [String],
    pub include_tags: bool,
}

impl<'a> EffectiveFilters<'a> {
    /// The values a repo of the organization gets when it has no overrides of its own.
    pub fn for_org(settings: &'a Settings, org: &'a FilterSet) -> Self {
        EffectiveFilters {
            branch_patterns: org
                .branch_patterns
                .as_deref()
                .unwrap_or(&settings.branch_patterns),
            ignored_workflows: org
                .ignored_workflows
                .as_deref()
                .unwrap_or(&settings.ignored_workflows),
            include_tags: org.include_tags.unwrap_or(settings.include_tags),
        }
    }

    /// The values `repo` is selected with, given its organization's overrides.
    pub fn resolve(settings: &'a Settings, org: &'a FilterSet, repo: &'a WatchedRepo) -> Self {
        let inherited = EffectiveFilters::for_org(settings, org);
        EffectiveFilters {
            branch_patterns: repo
                .branch_patterns
                .as_deref()
                .unwrap_or(inherited.branch_patterns),
            ignored_workflows: repo
                .ignored_workflows
                .as_deref()
                .unwrap_or(inherited.ignored_workflows),
            include_tags: repo.include_tags.unwrap_or(inherited.include_tags),
        }
    }
}

/// The compiled filters and tag choice one repo is selected with.
#[derive(Debug, Clone)]
pub struct RepoFilters {
    pub branch: Compiled<BranchFilter>,
    pub workflow: Compiled<WorkflowFilter>,
    pub include_tags: bool,
}

impl Default for RepoFilters {
    /// The default branch only, with no ignored workflows and no tag runs.
    fn default() -> RepoFilters {
        RepoFilters {
            branch: Arc::new(Ok(BranchFilter::Default)),
            workflow: Arc::new(Ok(WorkflowFilter::default())),
            include_tags: false,
        }
    }
}

/// Compiles the filters of one account's repos. Each organization's inherited filters are
/// compiled once and shared by every repo that inherits them; a repo with its own patterns gets
/// its own compiled filter.
#[derive(Debug)]
pub struct FilterCompiler<'a> {
    settings: &'a Settings,
    organizations: &'a [OrgFilters],
    account: &'a Account,
    /// Inherited filters per lowercase owner.
    inherited: HashMap<String, RepoFilters>,
}

impl<'a> FilterCompiler<'a> {
    pub fn new(
        settings: &'a Settings,
        organizations: &'a [OrgFilters],
        account: &'a Account,
    ) -> FilterCompiler<'a> {
        FilterCompiler {
            settings,
            organizations,
            account,
            inherited: HashMap::new(),
        }
    }

    pub fn for_repo(&mut self, repo: &WatchedRepo) -> RepoFilters {
        let key = OrgKey::of(self.account, &repo.repo.full_name);
        let org = key.filters_in(self.organizations);
        let base = EffectiveFilters::for_org(self.settings, org);
        let own = EffectiveFilters::resolve(self.settings, org, repo);
        let owner = key.owner.to_ascii_lowercase();
        let inherited = self.inherited.entry(owner).or_insert_with(|| RepoFilters {
            branch: Arc::new(compile_branch(base.branch_patterns)),
            workflow: Arc::new(WorkflowFilter::globs(base.ignored_workflows)),
            include_tags: base.include_tags,
        });
        let branch = if own.branch_patterns == base.branch_patterns {
            Arc::clone(&inherited.branch)
        } else {
            Arc::new(compile_branch(own.branch_patterns))
        };
        let workflow = if own.ignored_workflows == base.ignored_workflows {
            Arc::clone(&inherited.workflow)
        } else {
            Arc::new(WorkflowFilter::globs(own.ignored_workflows))
        };
        RepoFilters {
            branch,
            workflow,
            include_tags: own.include_tags,
        }
    }
}

/// No patterns selects the default branch only.
fn compile_branch(patterns: &[String]) -> Result<BranchFilter, globset::Error> {
    if patterns.is_empty() {
        Ok(BranchFilter::Default)
    } else {
        BranchFilter::globs(patterns)
    }
}

/// Where a repo's filters come from: the global settings, its organization's overrides, and the
/// repo with its own overrides.
#[derive(Debug, Clone, Copy)]
pub struct FilterScope<'a> {
    pub settings: &'a Settings,
    pub org: &'a FilterSet,
    pub repo: &'a WatchedRepo,
}

impl<'a> FilterScope<'a> {
    /// The scope of `repo`, watched through `account`, with the organization overrides looked
    /// up in `organizations`.
    pub fn new(
        settings: &'a Settings,
        organizations: &'a [OrgFilters],
        account: &Account,
        repo: &'a WatchedRepo,
    ) -> FilterScope<'a> {
        let org = OrgKey::of(account, &repo.repo.full_name).filters_in(organizations);
        FilterScope {
            settings,
            org,
            repo,
        }
    }

    /// The scope of `repo` within `config`. A repo of an unknown account inherits the global
    /// settings.
    pub fn in_config(config: &'a Config, repo: &'a WatchedRepo) -> FilterScope<'a> {
        FilterScope {
            settings: &config.settings,
            org: config.org_filters_of(repo),
            repo,
        }
    }

    pub fn effective(&self) -> EffectiveFilters<'a> {
        EffectiveFilters::resolve(self.settings, self.org, self.repo)
    }
}

/// Whether a repo selects its runs differently under the new config: its effective branch
/// patterns, ignored workflows, tag choice or pull request exclusion changed.
pub fn selection_changed(before: FilterScope<'_>, after: FilterScope<'_>) -> bool {
    before.settings.exclude_pull_requests != after.settings.exclude_pull_requests
        || before.effective() != after.effective()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_filter_matches_only_the_default_branch_and_has_no_patterns() {
        let filter = BranchFilter::Default;
        assert!(filter.matches("main", "main"));
        assert!(!filter.matches("dev", "main"));
        assert!(filter.patterns().is_empty());
    }

    #[test]
    fn glob_filter_exposes_its_patterns_and_keeps_slashes_literal() {
        let patterns = vec!["main".to_string(), "release/*".to_string()];
        let filter = BranchFilter::globs(&patterns).unwrap();
        assert_eq!(filter.patterns(), patterns.as_slice());
        assert!(filter.matches("release/1.0", "main"));
        assert!(!filter.matches("release/1.0/hotfix", "main"));
        assert!(!filter.matches("dev", "main"));
    }

    #[test]
    fn invalid_glob_is_rejected() {
        assert!(BranchFilter::globs(&["[".to_string()]).is_err());
        assert!(WorkflowFilter::globs(&["[".to_string()]).is_err());
    }

    #[test]
    fn workflow_filter_is_case_insensitive_and_empty_ignores_nothing() {
        let filter = WorkflowFilter::globs(&["nightly*".to_string()]).unwrap();
        assert!(filter.ignores("Nightly Build"));
        assert!(!filter.ignores("CI"));
        assert!(!WorkflowFilter::default().ignores("anything"));
    }

    fn settings(branches: &[&str], ignored: &[&str], include_tags: bool) -> Settings {
        Settings {
            branch_patterns: branches.iter().map(|s| s.to_string()).collect(),
            ignored_workflows: ignored.iter().map(|s| s.to_string()).collect(),
            include_tags,
            ..Default::default()
        }
    }

    fn repo() -> WatchedRepo {
        WatchedRepo {
            account_id: "acme".into(),
            repo: crate::providers::RepoInfo {
                id: 1,
                full_name: "acme/widgets".into(),
                web_url: "https://example.test/acme/widgets".into(),
                default_branch: "main".into(),
            },
            branch_patterns: None,
            ignored_workflows: None,
            include_tags: None,
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    fn org(branches: Option<&[&str]>, ignored: Option<&[&str]>, tags: Option<bool>) -> FilterSet {
        FilterSet {
            branch_patterns: branches.map(strings),
            ignored_workflows: ignored.map(strings),
            include_tags: tags,
        }
    }

    #[test]
    fn values_resolve_from_repo_then_org_then_global() {
        let global = settings(&["develop"], &["Dependabot*"], false);
        let mut watched = repo();

        let effective = EffectiveFilters::resolve(&global, &FilterSet::INHERIT, &watched);
        assert_eq!(effective.branch_patterns, strings(&["develop"]).as_slice());
        assert_eq!(
            effective.ignored_workflows,
            strings(&["Dependabot*"]).as_slice()
        );
        assert!(!effective.include_tags);

        let acme = org(Some(&["release/*"]), Some(&["Nightly*"]), Some(true));
        let effective = EffectiveFilters::resolve(&global, &acme, &watched);
        assert_eq!(
            effective.branch_patterns,
            strings(&["release/*"]).as_slice()
        );
        assert_eq!(
            effective.ignored_workflows,
            strings(&["Nightly*"]).as_slice()
        );
        assert!(effective.include_tags);

        watched.branch_patterns = Some(strings(&["main"]));
        watched.ignored_workflows = Some(strings(&["Docs"]));
        watched.include_tags = Some(false);
        let effective = EffectiveFilters::resolve(&global, &acme, &watched);
        assert_eq!(effective.branch_patterns, strings(&["main"]).as_slice());
        assert_eq!(effective.ignored_workflows, strings(&["Docs"]).as_slice());
        assert!(!effective.include_tags);
    }

    #[test]
    fn an_org_that_ignores_nothing_lets_one_repo_ignore_dependabot_again() {
        let global = settings(&[], &["Dependabot*"], false);
        let acme = org(None, Some(&[]), None);
        let kept = repo();
        let effective = EffectiveFilters::resolve(&global, &acme, &kept);
        assert!(effective.ignored_workflows.is_empty());
        let compiled = FilterCompiler::new(&global, &[], &github())
            .for_repo(&kept)
            .workflow;
        assert!(compiled
            .as_ref()
            .as_ref()
            .unwrap()
            .ignores("Dependabot Updates"));

        let organizations = vec![OrgFilters {
            host: "github.com".into(),
            owner: "ACME".into(),
            filters: acme.clone(),
        }];
        let account = github();
        let mut compiler = FilterCompiler::new(&global, &organizations, &account);
        let workflow = compiler.for_repo(&kept).workflow;
        assert!(!workflow
            .as_ref()
            .as_ref()
            .unwrap()
            .ignores("Dependabot Updates"));

        let ignores_again = WatchedRepo {
            ignored_workflows: Some(strings(&["Dependabot*"])),
            ..repo()
        };
        let effective = EffectiveFilters::resolve(&global, &acme, &ignores_again);
        assert_eq!(
            effective.ignored_workflows,
            strings(&["Dependabot*"]).as_slice()
        );
        let workflow = compiler.for_repo(&ignores_again).workflow;
        assert!(workflow
            .as_ref()
            .as_ref()
            .unwrap()
            .ignores("Dependabot Updates"));
    }

    #[test]
    fn empty_lists_are_real_overrides_at_org_and_repo_level() {
        let global = settings(&["develop"], &["Dependabot*"], false);
        let acme = org(Some(&[]), Some(&[]), None);
        let inheriting = repo();
        let effective = EffectiveFilters::resolve(&global, &acme, &inheriting);
        assert!(effective.branch_patterns.is_empty());
        assert!(effective.ignored_workflows.is_empty());

        let mut watched = repo();
        watched.branch_patterns = Some(vec![]);
        watched.ignored_workflows = Some(vec![]);
        let effective = EffectiveFilters::resolve(&global, &FilterSet::INHERIT, &watched);
        assert!(effective.branch_patterns.is_empty());
        assert!(effective.ignored_workflows.is_empty());
    }

    fn github() -> Account {
        Account::new(crate::config::AccountKind::GitHub, "acme", None)
    }

    fn org_entry(owner: &str, filters: FilterSet) -> OrgFilters {
        OrgFilters {
            host: "github.com".into(),
            owner: owner.into(),
            filters,
        }
    }

    #[test]
    fn compiled_filters_follow_the_resolution_order() {
        let global = settings(&[], &["Dependabot*"], false);
        let organizations = vec![org_entry(
            "acme",
            org(Some(&["release/*"]), Some(&[]), Some(true)),
        )];
        let account = github();
        let mut compiler = FilterCompiler::new(&global, &organizations, &account);

        let inherits = compiler.for_repo(&repo());
        let branch = inherits.branch.as_ref().as_ref().unwrap();
        assert!(branch.matches("release/1.0", "main"));
        assert!(!branch.matches("main", "main"));
        assert!(!inherits
            .workflow
            .as_ref()
            .as_ref()
            .unwrap()
            .ignores("Dependabot Updates"));
        assert!(inherits.include_tags);

        let own = WatchedRepo {
            branch_patterns: Some(vec![]),
            ignored_workflows: Some(strings(&["Dependabot*"])),
            include_tags: Some(false),
            ..repo()
        };
        let overridden = compiler.for_repo(&own);
        let branch = overridden.branch.as_ref().as_ref().unwrap();
        assert!(branch.matches("main", "main"));
        assert!(!branch.matches("release/1.0", "main"));
        assert!(overridden
            .workflow
            .as_ref()
            .as_ref()
            .unwrap()
            .ignores("Dependabot Updates"));
        assert!(!overridden.include_tags);

        let other_org = WatchedRepo {
            repo: crate::providers::RepoInfo {
                full_name: "example/tools".into(),
                ..repo().repo
            },
            ..repo()
        };
        let global_only = compiler.for_repo(&other_org);
        let branch = global_only.branch.as_ref().as_ref().unwrap();
        assert!(branch.matches("main", "main"));
        assert!(!global_only.include_tags);
    }

    #[test]
    fn repos_inheriting_their_org_filters_share_one_compiled_set() {
        let global = Settings::default();
        let organizations = vec![org_entry("acme", org(Some(&["release/*"]), None, None))];
        let account = github();
        let mut compiler = FilterCompiler::new(&global, &organizations, &account);
        let first = compiler.for_repo(&repo());
        let second = compiler.for_repo(&WatchedRepo {
            branch_patterns: Some(strings(&["release/*"])),
            ..repo()
        });
        assert!(Arc::ptr_eq(&first.branch, &second.branch));
        assert!(Arc::ptr_eq(&first.workflow, &second.workflow));
        let other = compiler.for_repo(&WatchedRepo {
            repo: crate::providers::RepoInfo {
                full_name: "example/tools".into(),
                ..repo().repo
            },
            ..repo()
        });
        assert!(!Arc::ptr_eq(&first.branch, &other.branch));
    }

    #[test]
    fn selection_changes_with_the_effective_values_only() {
        let global = settings(&[], &["Dependabot*"], false);
        let inherit = FilterSet::INHERIT;
        let same_as_global = org(None, Some(&["Dependabot*"]), None);
        let keep_all = org(None, Some(&[]), None);
        let watched = repo();
        let scope = |org| FilterScope {
            settings: &global,
            org,
            repo: &watched,
        };
        assert!(!selection_changed(scope(&inherit), scope(&same_as_global)));
        assert!(selection_changed(scope(&inherit), scope(&keep_all)));

        let pinned = WatchedRepo {
            ignored_workflows: Some(strings(&["Dependabot*"])),
            ..repo()
        };
        let pinned_scope = |org| FilterScope {
            settings: &global,
            org,
            repo: &pinned,
        };
        assert!(!selection_changed(
            pinned_scope(&inherit),
            pinned_scope(&keep_all)
        ));
    }

    #[test]
    fn a_scope_finds_the_org_of_the_repo_owner() {
        let global = Settings::default();
        let organizations = vec![org_entry("Acme", org(None, None, Some(true)))];
        let account = github();
        let watched = repo();
        let scope = FilterScope::new(&global, &organizations, &account, &watched);
        assert!(scope.effective().include_tags);
        let gitlab = Account::new(
            crate::config::AccountKind::GitLab,
            "gl",
            Some("https://gitlab.example.test/".into()),
        );
        let scope = FilterScope::new(&global, &organizations, &gitlab, &watched);
        assert!(!scope.effective().include_tags);
    }
}
