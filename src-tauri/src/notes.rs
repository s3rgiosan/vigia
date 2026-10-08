//! Status text shown on repos and accounts in the popup and Settings. Technical detail goes to
//! the log; these strings only say what happened and what to do.

pub const NO_ACCESS: &str = "Not found, or the token can't access it";
pub const CI_DISABLED: &str = "CI/CD is turned off";
pub const UNREACHABLE: &str = "Can't reach the server";
pub const TOKEN_REJECTED: &str = "Token rejected. Replace it in Settings.";
pub const INTERNAL_ERROR: &str = "Something went wrong. Vigia will retry.";
pub const NO_TOKEN: &str = "No token saved";
/// The token store named as the OS names it.
#[cfg(target_os = "macos")]
pub const KEYCHAIN_DENIED: &str = "Vigia can't read the Keychain";
#[cfg(target_os = "windows")]
pub const KEYCHAIN_DENIED: &str = "Vigia can't read the Credential Manager";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const KEYCHAIN_DENIED: &str = "Vigia can't read the keyring";
pub const REDIRECTED: &str = "The server redirected Vigia. Check the base URL in Settings.";
pub const UNEXPECTED_RESPONSE: &str = "The server sent a response Vigia can't read";

/// Note for a branch filter that does not compile; `detail` names the offending pattern.
pub fn invalid_branch_filter(detail: &str) -> String {
    format!("Branch filter isn't valid: {detail}")
}

/// Note for a workflow filter that does not compile; `detail` names the offending pattern.
pub fn invalid_workflow_filter(detail: &str) -> String {
    format!("Workflow filter isn't valid: {detail}")
}
