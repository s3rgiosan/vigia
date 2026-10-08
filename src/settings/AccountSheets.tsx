import { useEffect, useState } from "react";
import { FormRow, FormSection } from "../components/Form";
import { Icon } from "../components/Icon";
import { Sheet, SheetNote } from "../components/Sheet";
import { friendlyError } from "../lib/errors";
import { tokenTypeError } from "../lib/github";
import { useLatest } from "../lib/hooks/useLatest";
import type { AccountKind } from "../lib/snapshot";
import {
  addAccount,
  openGithubTokenPage,
  testConnection,
  type Account,
  type AccountIdentity,
  type NewAccount,
} from "../lib/tauri";
import { defaultLabel, isInsecureUrl, providerName } from "./accounts";
import { SECRET_STORE } from "../lib/platform";

/** Server address the GitLab form starts with. */
const DEFAULT_GITLAB_URL = "https://gitlab.com";

export function AddAccountSheet({
  onCancel,
  onAdded,
  onError,
}: {
  onCancel: () => void;
  onAdded: (account: Account) => void;
  onError: (message: string | null) => void;
}) {
  const [kind, setKind] = useState<AccountKind>("github");
  const [owner, setOwner] = useState("");
  const [baseUrl, setBaseUrl] = useState(DEFAULT_GITLAB_URL);
  const [token, setToken] = useState("");
  const [label, setLabel] = useState("");
  const [insecureOk, setInsecureOk] = useState(false);
  const [busy, setBusy] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  const insecure = kind === "gitlab" && isInsecureUrl(baseUrl);
  const typeError = kind === "github" ? tokenTypeError(token) : null;
  const ready =
    token.trim() !== "" && typeError === null && (kind === "github" || baseUrl.trim() !== "") && (!insecure || insecureOk);

  function newAccount(): NewAccount {
    return {
      kind,
      label: label.trim() || defaultLabel(kind, owner, baseUrl),
      base_url: kind === "gitlab" ? baseUrl.trim() : undefined,
      token: token.trim(),
      allow_insecure: kind === "gitlab" && insecure && insecureOk,
    };
  }

  async function submit() {
    setBusy(true);
    setLocalError(null);
    try {
      const account = await addAccount(newAccount());
      onError(null);
      onAdded(account);
    } catch (e) {
      setLocalError(friendlyError(e));
    } finally {
      setBusy(false);
    }
  }

  const testButton = (
    <TestConnection
      key={`${kind}|${baseUrl}|${token}|${insecureOk}`}
      build={newAccount}
      disabled={!ready || busy}
      tokenError={typeError}
      onError={setLocalError}
    />
  );

  return (
    <Sheet
      title="Add Account"
      submitLabel="Add Account"
      submitDisabled={!ready}
      busy={busy}
      busyLabel="Adding…"
      onCancel={onCancel}
      onSubmit={submit}
      width={480}
      accessory={testButton}
    >
      <div className="segmented" role="radiogroup" aria-label="Provider">
        {(["github", "gitlab"] as const).map((k) => (
          <button
            key={k}
            type="button"
            role="radio"
            aria-checked={kind === k}
            className={kind === k ? "segmented__item segmented__item--on" : "segmented__item"}
            onClick={() => {
              setKind(k);
              setLocalError(null);
            }}
          >
            <Icon name={k} size={14} />
            {providerName(k)}
          </button>
        ))}
      </div>

      {kind === "github" ? (
        <ol className="steps">
          <li>
            <span>
              Create a fine-grained token. For an organization’s repositories, enter its name first; leave it empty for your
              own.
            </span>
            <div className="steps__row">
              <label className="steps__label" htmlFor="add-owner">
                Organization
              </label>
              <input
                id="add-owner"
                className="field grow"
                value={owner}
                onChange={(e) => setOwner(e.target.value)}
                placeholder="Optional"
                spellCheck={false}
              />
              <button type="button" onClick={() => openGithubTokenPage(owner.trim() === "" ? null : owner.trim())}>
                Create Token on GitHub <Icon name="external" size={11} />
              </button>
            </div>
          </li>
          <li>
            <span>
              On GitHub, choose the repository access, keep <strong>Actions</strong> and <strong>Metadata</strong> read-only,
              and generate the token.
            </span>
          </li>
          <li>
            <span>Paste the token below.</span>
          </li>
        </ol>
      ) : (
        <FormSection>
          <FormRow label="Server" htmlFor="add-base-url">
            <input
              id="add-base-url"
              className="field field--wide"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="https://gitlab.com"
              spellCheck={false}
            />
          </FormRow>
        </FormSection>
      )}

      <FormSection
        footer={
          kind === "gitlab"
            ? `Personal access token with the read_api scope. Servers with an internal certificate authority work when it is trusted in ${SECRET_STORE.trustStore}.`
            : "One token covers one owner. Add another account for each organization."
        }
      >
        <FormRow label="Token" htmlFor="add-token">
          <input
            id="add-token"
            className="field field--wide"
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={kind === "github" ? "github_pat_…" : "glpat-…"}
            autoComplete="off"
            spellCheck={false}
          />
        </FormRow>
        <FormRow label="Name" htmlFor="add-label">
          <input
            id="add-label"
            className="field field--wide"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder={defaultLabel(kind, owner, baseUrl)}
          />
        </FormRow>
      </FormSection>

      {insecure ? (
        <label className="check">
          <input type="checkbox" checked={insecureOk} onChange={(e) => setInsecureOk(e.target.checked)} />
          <Icon name="warning" size={13} className="check__warning" />
          This server uses http://. Send the token unencrypted anyway.
        </label>
      ) : null}
      {typeError ? <SheetNote>{typeError}</SheetNote> : null}
      {localError ? <SheetNote>{localError}</SheetNote> : null}
    </Sheet>
  );
}

export function ReplaceTokenSheet({
  account,
  onCancel,
  onReplace,
}: {
  account: Account;
  onCancel: () => void;
  onReplace: (token: string) => Promise<void>;
}) {
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const typeError = account.kind === "github" ? tokenTypeError(token) : null;
  const insecure = isInsecureUrl(account.base_url);

  const testButton = (
    <TestConnection
      key={token}
      build={() => ({
        kind: account.kind,
        label: account.label,
        base_url: account.base_url,
        token: token.trim(),
        allow_insecure: insecure,
      })}
      disabled={token.trim() === "" || busy}
      tokenError={typeError}
      onError={setLocalError}
    />
  );

  return (
    <Sheet
      title={`Replace Token for “${account.label}”`}
      submitLabel="Replace"
      busyLabel="Replacing…"
      accessory={testButton}
      submitDisabled={token.trim() === "" || typeError !== null}
      busy={busy}
      onCancel={onCancel}
      onSubmit={async () => {
        setBusy(true);
        setLocalError(null);
        try {
          await onReplace(token.trim());
        } catch (e) {
          setLocalError(friendlyError(e));
          setBusy(false);
        }
      }}
    >
      {account.kind === "github" ? (
        <p className="sheet__text">
          Use the same owner and repository access as the current token.{" "}
          <button type="button" className="link" onClick={() => openGithubTokenPage(null)}>
            Create Token on GitHub
          </button>
        </p>
      ) : null}
      <FormSection>
        <FormRow label="New token" htmlFor="replace-token">
          <input
            id="replace-token"
            className="field field--wide"
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={account.kind === "github" ? "github_pat_…" : "glpat-…"}
            autoComplete="off"
            spellCheck={false}
          />
        </FormRow>
      </FormSection>
      {typeError ? <SheetNote>{typeError}</SheetNote> : null}
      {localError ? <SheetNote>{localError}</SheetNote> : null}
    </Sheet>
  );
}

/**
 * Checks a token against the server and shows a short result next to the button. The login is
 * in the tooltip and the spoken text; a failure's reason goes to the sheet through `onError`. The
 * parent remounts it with a new key whenever the inputs change, which clears the previous result.
 */
function TestConnection({
  build,
  disabled,
  tokenError,
  onError,
}: {
  build: () => NewAccount;
  disabled: boolean;
  tokenError: string | null;
  onError: (message: string | null) => void;
}) {
  const [testing, setTesting] = useState(false);
  const [identity, setIdentity] = useState<AccountIdentity | null>(null);
  const [failed, setFailed] = useState(false);
  const latestOnError = useLatest(onError);

  // A result belongs to the inputs it was tested with, so its message leaves with it.
  useEffect(() => () => latestOnError.current(null), [latestOnError]);

  async function test() {
    setIdentity(null);
    setFailed(false);
    onError(null);
    if (tokenError !== null) {
      setFailed(true);
      return;
    }
    setTesting(true);
    try {
      setIdentity(await testConnection(build()));
    } catch (e) {
      setFailed(true);
      onError(friendlyError(e));
    } finally {
      setTesting(false);
    }
  }

  return (
    <>
      <button type="button" onClick={test} disabled={disabled || testing}>
        {testing ? "Testing…" : "Test Connection"}
      </button>
      <span className="sheet__status" role="status">
        {identity ? (
          <span className="value value--status" title={`Connected as ${identity.login}`}>
            <span className="dot dot--green" aria-hidden="true" />
            Connected
            <span className="visually-hidden"> as {identity.login}</span>
          </span>
        ) : null}
        {failed ? (
          <span className="value value--status">
            <span className="dot dot--red" aria-hidden="true" />
            Failed
          </span>
        ) : null}
      </span>
    </>
  );
}
