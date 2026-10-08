/** Prefix GitHub gives fine-grained personal access tokens. */
export const FINE_GRAINED_PREFIX = "github_pat_";

/** Whether a pasted value looks like a fine-grained token. Empty input is not flagged. */
export function tokenTypeError(token: string): string | null {
  const value = token.trim();
  if (value === "" || value.startsWith(FINE_GRAINED_PREFIX)) {
    return null;
  }
  if (value.startsWith("ghp_")) {
    return "This is a classic token. Create a fine-grained token instead; classic tokens grant write access.";
  }
  return "This is not a fine-grained token. Fine-grained tokens start with github_pat_.";
}

/** Note the backend sets on a repo GitHub answered with 403 or 404. */
export const NO_ACCESS_NOTE = "Not found, or the token can't access it";

/** Why an organization's repositories can be missing even though the token is valid. */
export const SAML_HINT =
  "Organizations that enforce SAML single sign-on require the token to be authorized for SSO, or their repositories will not appear.";
