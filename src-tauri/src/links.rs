//! Guards URLs from API responses before they reach the default browser.

use url::Url;

use crate::config::{Account, AccountKind};

pub const GITHUB_WEB_ORIGIN: &str = "https://github.com";

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LinkError {
    #[error("not a valid URL")]
    Invalid,
    #[error("the URL does not belong to this account's host")]
    WrongHost,
}

/// Accepts a URL only when its scheme and host match the account's web origin.
pub fn check_url(account: &Account, raw: &str) -> Result<Url, LinkError> {
    let url = Url::parse(raw).map_err(|_| LinkError::Invalid)?;
    let origin = match account.kind {
        AccountKind::GitHub => Url::parse(GITHUB_WEB_ORIGIN).map_err(|_| LinkError::Invalid)?,
        AccountKind::GitLab => {
            let base = account.base_url.as_deref().ok_or(LinkError::WrongHost)?;
            Url::parse(base).map_err(|_| LinkError::Invalid)?
        }
    };
    let same = url.scheme() == origin.scheme()
        && url.host_str() == origin.host_str()
        && url.port_or_known_default() == origin.port_or_known_default();
    if same {
        Ok(url)
    } else {
        Err(LinkError::WrongHost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github() -> Account {
        Account::new(AccountKind::GitHub, "gh", None)
    }

    fn gitlab() -> Account {
        Account::new(
            AccountKind::GitLab,
            "gl",
            Some("https://gitlab.example.test/gitlab".into()),
        )
    }

    #[test]
    fn github_links_must_be_on_github_com() {
        assert!(check_url(&github(), "https://github.com/acme/widgets/actions/runs/1").is_ok());
        assert_eq!(
            check_url(&github(), "https://evil.test/acme"),
            Err(LinkError::WrongHost)
        );
        assert_eq!(
            check_url(&github(), "http://github.com/acme"),
            Err(LinkError::WrongHost)
        );
    }

    #[test]
    fn gitlab_links_must_match_the_instance() {
        assert!(check_url(
            &gitlab(),
            "https://gitlab.example.test/group/p/-/pipelines/1"
        )
        .is_ok());
        assert_eq!(
            check_url(&gitlab(), "https://gitlab.example.test:8443/x"),
            Err(LinkError::WrongHost)
        );
        assert_eq!(
            check_url(&gitlab(), "file:///etc/passwd"),
            Err(LinkError::WrongHost)
        );
    }

    #[test]
    fn garbage_is_invalid() {
        assert_eq!(check_url(&github(), "not a url"), Err(LinkError::Invalid));
    }
}
