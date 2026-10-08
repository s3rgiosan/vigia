import { useEffect, useRef, useState } from "react";
import { FormRow, FormSection } from "../components/Form";
import { Icon } from "../components/Icon";
import { ActionPopUpButton } from "../components/PopUpButton";
import { Sheet } from "../components/Sheet";
import { friendlyError } from "../lib/errors";
import { useLatest } from "../lib/hooks/useLatest";
import { useRovingListbox } from "../lib/hooks/useRovingListbox";
import type { AccountSnapshot } from "../lib/snapshot";
import { deleteAccount, deleteAllAccounts, renameAccount, replaceToken, type Account } from "../lib/tauri";
import { AddAccountSheet, ReplaceTokenSheet } from "./AccountSheets";
import { accountStatus, hostOf, pluralize, providerName } from "./accounts";
import { useSettings, type SheetRequest } from "./SettingsContext";

type SheetKind = "add" | "replace" | "remove" | "removeAll" | null;

export function AccountsTab({
  request,
  onRequestHandled,
}: {
  /** A sheet asked for from outside the pane; handled once, then cleared through `onRequestHandled`. */
  request: SheetRequest | null;
  onRequestHandled: () => void;
}) {
  const { view, snapshot, reload, setError, readOnly } = useSettings();
  const { accounts } = view;
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sheet, setSheet] = useState<SheetKind>(null);
  const [removing, setRemoving] = useState(false);
  const optionRefs = useRef<(HTMLLIElement | null)[]>([]);

  const selected = accounts.find((a) => a.id === selectedId) ?? accounts[0] ?? null;
  const selectedIndex = selected ? accounts.indexOf(selected) : -1;
  const statusOf = (id: string) => snapshot?.accounts.find((a) => a.id === id);
  const repoCount = (id: string) => view.repos.filter((r) => r.account_id === id).length;
  const canAdd = !readOnly && !view.secrets_blocked;

  const onListKey = useRovingListbox({
    count: accounts.length,
    activeIndex: selectedIndex,
    onMove: (index) => {
      setSelectedId(accounts[index].id);
      optionRefs.current[index]?.focus();
    },
  });

  // A request from a deep link or another pane opens its sheet once.
  const latestAccounts = useLatest(accounts);
  useEffect(() => {
    if (!request) {
      return;
    }
    onRequestHandled();
    if (request.sheet === "add") {
      setSheet("add");
      return;
    }
    const target = latestAccounts.current.find((a) => a.id === request.accountId);
    if (target) {
      setSelectedId(target.id);
      setSheet("replace");
    }
  }, [request, onRequestHandled, latestAccounts]);

  async function run(action: () => Promise<unknown>): Promise<boolean> {
    try {
      await action();
      setError(null);
      await reload();
      return true;
    } catch (e) {
      setError(friendlyError(e));
      return false;
    }
  }

  async function confirmRemoval(action: () => Promise<unknown>) {
    setRemoving(true);
    try {
      if (await run(action)) {
        setSheet(null);
      }
    } finally {
      setRemoving(false);
    }
  }

  const addSheet =
    sheet === "add" && canAdd ? (
      <AddAccountSheet
        onCancel={() => setSheet(null)}
        onAdded={async (account) => {
          setSheet(null);
          await reload();
          setSelectedId(account.id);
        }}
        onError={setError}
      />
    ) : null;

  if (accounts.length === 0 || !selected) {
    return (
      <>
        <div className="empty-pane">
          <Icon name="person" size={28} />
          <h2>No accounts</h2>
          <p>Add a GitHub or GitLab account to choose which repositories Vigia watches.</p>
          <button type="button" className="default" onClick={() => setSheet("add")} disabled={!canAdd}>
            Add Account…
          </button>
        </div>
        {addSheet}
      </>
    );
  }

  return (
    <div className="accounts">
      <div className="accounts__sidebar">
        <ul className="source-list" role="listbox" aria-label="Accounts" onKeyDown={onListKey}>
          {accounts.map((account, index) => {
            const isSelected = account.id === selected.id;
            return (
              <li
                key={account.id}
                ref={(el) => {
                  optionRefs.current[index] = el;
                }}
                role="option"
                aria-selected={isSelected}
                tabIndex={isSelected ? 0 : -1}
                className={`source-list__row ${isSelected ? "source-list__row--selected" : ""}`}
                onClick={() => setSelectedId(account.id)}
              >
                <Icon name={account.kind} size={18} />
                <span className="source-list__text">
                  <span>{account.label}</span>
                  <span className="source-list__sub">{account.login ?? hostOf(account)}</span>
                </span>
                <StatusDot status={statusOf(account.id)} />
              </li>
            );
          })}
        </ul>
        <div className="list-bar">
          <div className="add-remove">
            <button type="button" onClick={() => setSheet("add")} disabled={!canAdd} title="Add Account" aria-label="Add account">
              <Icon name="plus" size={13} />
            </button>
            <button
              type="button"
              onClick={() => setSheet("remove")}
              disabled={readOnly}
              title="Remove Account"
              aria-label="Remove account"
            >
              <Icon name="minus" size={13} />
            </button>
          </div>
          <ActionPopUpButton
            disabled={readOnly}
            actions={[{ label: "Remove All Accounts…", onSelect: () => setSheet("removeAll"), disabled: accounts.length < 2 }]}
          />
        </div>
      </div>

      <AccountDetail
        key={selected.id}
        account={selected}
        status={statusOf(selected.id)}
        repos={repoCount(selected.id)}
        readOnly={readOnly}
        secretsBlocked={view.secrets_blocked}
        onRename={(label) => run(() => renameAccount(selected.id, label))}
        onReplaceToken={() => setSheet("replace")}
      />

      {addSheet}

      {sheet === "replace" && !view.secrets_blocked ? (
        <ReplaceTokenSheet
          account={selected}
          onCancel={() => setSheet(null)}
          onReplace={async (token) => {
            await replaceToken(selected.id, token);
            setError(null);
            setSheet(null);
            await reload();
          }}
        />
      ) : null}

      {sheet === "remove" ? (
        <Sheet
          title={`Remove “${selected.label}”?`}
          message={`Vigia stops watching its ${pluralize(repoCount(selected.id), "repository", "repositories")} and deletes its token from the Keychain. The token itself stays valid on ${providerName(selected.kind)}.`}
          submitLabel="Remove"
          destructive
          busy={removing}
          busyLabel="Removing…"
          onCancel={() => setSheet(null)}
          onSubmit={() => confirmRemoval(() => deleteAccount(selected.id))}
        />
      ) : null}

      {sheet === "removeAll" ? (
        <Sheet
          title={`Remove all ${accounts.length} accounts?`}
          message="Vigia stops watching every repository and deletes all tokens from the Keychain. Your other settings stay."
          submitLabel="Remove All"
          destructive
          busy={removing}
          busyLabel="Removing…"
          onCancel={() => setSheet(null)}
          onSubmit={() => confirmRemoval(() => deleteAllAccounts())}
        />
      ) : null}
    </div>
  );
}

function AccountDetail({
  account,
  status,
  repos,
  readOnly,
  secretsBlocked,
  onRename,
  onReplaceToken,
}: {
  account: Account;
  status: AccountSnapshot | undefined;
  repos: number;
  readOnly: boolean;
  secretsBlocked: boolean;
  onRename: (label: string) => Promise<boolean>;
  onReplaceToken: () => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  const label = draft ?? saving ?? account.label;
  const latest = useLatest({ draft, account, onRename });

  async function commitLabel() {
    const current = latest.current;
    if (current.draft === null) {
      return;
    }
    const value = current.draft.trim();
    setDraft(null);
    if (value === "" || value === current.account.label) {
      return;
    }
    // The new name shows while it saves; a failed rename falls back to the stored one.
    setSaving(value);
    await current.onRename(value);
    setSaving(null);
  }

  // Leaving the account mid-edit saves the typed name.
  useEffect(() => {
    return () => {
      const current = latest.current;
      const value = (current.draft ?? "").trim();
      if (value !== "" && value !== current.account.label) {
        void current.onRename(value);
      }
    };
  }, [latest]);

  const { text } = accountStatus(status);

  return (
    <div className="accounts__detail">
      <div className="detail-header">
        <div className="detail-header__icon">
          <Icon name={account.kind} size={26} />
        </div>
        <div>
          <h2>{account.label}</h2>
          <p>
            {providerName(account.kind)} · {pluralize(repos, "repository", "repositories")} watched
          </p>
        </div>
      </div>

      {status?.auth_error ? (
        <div className="banner banner--error">
          <Icon name="warning" size={14} />
          <span className="grow">The token was rejected. Replace it to resume checking.</span>
        </div>
      ) : null}
      {status?.unreachable ? (
        <div className="banner banner--warn">
          <Icon name="warning" size={14} />
          <span className="grow">The server is unreachable. Vigia keeps retrying.</span>
        </div>
      ) : null}

      <FormSection>
        <FormRow label="Name" htmlFor="account-label">
          <input
            id="account-label"
            className="field"
            value={label}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commitLabel}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.currentTarget.blur();
              }
            }}
            disabled={readOnly}
          />
        </FormRow>
        <FormRow label="Signed in as">
          <span className="value">{account.login ?? "—"}</span>
        </FormRow>
        {account.kind === "gitlab" ? (
          <FormRow label="Server">
            <span className="value">{hostOf(account)}</span>
          </FormRow>
        ) : null}
        <FormRow label="Status">
          <span className="value value--status">
            <StatusDot status={status} decorative />
            {text}
          </span>
        </FormRow>
      </FormSection>

      <FormSection
        title="Token"
        footer={
          account.kind === "github"
            ? "Fine-grained token with read-only Actions and Metadata access. Replace it before it expires."
            : "Personal access token with the read_api scope."
        }
      >
        <FormRow label="Stored in the Keychain">
          <button type="button" onClick={onReplaceToken} disabled={secretsBlocked}>
            Replace Token…
          </button>
        </FormRow>
      </FormSection>
    </div>
  );
}

/**
 * Status indicator: a coloured dot with a text label. `decorative` drops the label where the
 * state is already written next to it.
 */
function StatusDot({ status, decorative }: { status: AccountSnapshot | undefined; decorative?: boolean }) {
  const { color, text } = accountStatus(status);
  return (
    <span className="status-dot" title={decorative ? undefined : text}>
      <span className={`dot dot--${color}`} aria-hidden="true" />
      {decorative ? null : <span className="visually-hidden">{text}</span>}
    </span>
  );
}
