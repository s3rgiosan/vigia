use async_trait::async_trait;
use vigia_lib::providers::{
    AccountIdentity, FetchOutcome, FetchRequest, Provider, ProviderError, RepoCache, RepoInfo,
};

struct MinimalProvider;

#[async_trait]
impl Provider for MinimalProvider {
    async fn validate(&self) -> Result<AccountIdentity, ProviderError> {
        unreachable!()
    }

    async fn list_repos(
        &self,
        _on_page: &(dyn Fn(usize) + Send + Sync),
    ) -> Result<Vec<RepoInfo>, ProviderError> {
        unreachable!()
    }

    async fn fetch(
        &self,
        _request: FetchRequest<'_>,
        _cache: &mut RepoCache,
    ) -> Result<FetchOutcome, ProviderError> {
        unreachable!()
    }
}

#[tokio::test]
async fn providers_are_members_unless_they_say_otherwise() {
    let repo = RepoInfo {
        id: 1,
        full_name: "acme/widgets".into(),
        web_url: "https://example.test/acme/widgets".into(),
        default_branch: "main".into(),
    };
    assert_eq!(MinimalProvider.is_member(&repo).await, Ok(true));
}
