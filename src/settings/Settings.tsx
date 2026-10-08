import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Icon, type IconName } from "../components/Icon";
import { OpenSheetsContext, Sheet, SheetNote } from "../components/Sheet";
import { friendlyError } from "../lib/errors";
import { useLatest } from "../lib/hooks/useLatest";
import { useSafeStorage } from "../lib/hooks/useSafeStorage";
import { isPreview } from "../lib/preview";
import type { Snapshot } from "../lib/snapshot";
import {
  getSettings,
  getSnapshot,
  onSettingsOpen,
  onSettingsPane,
  onSnapshot,
  resetSecrets,
  retrySecrets,
  selectSettingsPane,
  subscribe,
  type SettingsOpen,
  type SettingsPane,
  type SettingsView,
} from "../lib/tauri";
import { AccountsTab } from "./AccountsTab";
import { BranchesTab } from "./BranchesTab";
import { GeneralTab } from "./GeneralTab";
import { ReposTab } from "./ReposTab";
import { SettingsContext, type SettingsContextValue, type SheetRequest } from "./SettingsContext";
import { useSettingsSaver } from "./useSettingsSaver";
import "./Settings.css";
import { IS_MACOS, SECRET_STORE } from "../lib/platform";

const PANES: { id: SettingsPane; title: string; icon: IconName }[] = [
  { id: "accounts", title: "Accounts", icon: "person" },
  { id: "repos", title: "Repositories", icon: "folder" },
  { id: "branches", title: "Filters", icon: "filter" },
  { id: "general", title: "General", icon: "gear" },
];

const PANE_KEY = "vigia.settings.tab";

function asPane(value: string | null | undefined): SettingsPane | null {
  return PANES.find((p) => p.id === value)?.id ?? null;
}

/** The sheet named by a deep link or `settings-open` event, or null when it names none. */
export function sheetRequestOf(sheet: string | null, accountId: string | null): SheetRequest | null {
  switch (sheet) {
    case "add":
      return { sheet: "add", accountId: null };
    case "replace":
      return accountId ? { sheet: "replace", accountId } : null;
    default:
      return null;
  }
}

function searchParam(name: string): string | null {
  return new URLSearchParams(window.location.search).get(name);
}

export function Settings() {
  const preview = isPreview();
  const [storedPane, storePane] = useSafeStorage(PANE_KEY, "accounts");
  const [request, setRequest] = useState<SheetRequest | null>(() =>
    sheetRequestOf(searchParam("sheet"), searchParam("account")),
  );
  // A requested sheet lives on the Accounts pane; otherwise the URL's pane, then the remembered one.
  const [pane, setPane] = useState<SettingsPane>(() => {
    if (request) {
      return "accounts";
    }
    return asPane(searchParam("pane")) ?? asPane(storedPane) ?? "accounts";
  });
  const [view, setView] = useState<SettingsView | null>(null);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [resetOpen, setResetOpen] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  const openSheets = useRef(0);
  const latestRequest = useRef(0);
  const currentPane = useLatest(pane);

  const reload = useCallback(async () => {
    latestRequest.current += 1;
    const ticket = latestRequest.current;
    try {
      const next = await getSettings();
      if (ticket === latestRequest.current) {
        setView(next);
      }
    } catch (e) {
      if (ticket === latestRequest.current) {
        setError(friendlyError(e));
      }
    }
  }, []);

  const saveSettings = useSettingsSaver(view?.settings ?? null, reload, setError);

  useEffect(() => {
    reload();
    getSnapshot().then(setSnapshot).catch((e) => console.error("could not load the snapshot", e));
    return subscribe(onSnapshot(setSnapshot));
  }, [reload]);

  /** Pane switches are ignored while a sheet is open, so typed input is not discarded. */
  const switchPane = useCallback(
    (next: SettingsPane) => {
      if (openSheets.current > 0) {
        if (!isPreview()) {
          selectSettingsPane(currentPane.current).catch(() => undefined);
        }
        return;
      }
      setPane(next);
    },
    [currentPane],
  );

  /** Shows a pane or sheet on request, unless a sheet already holds typed input. */
  const showTarget = useCallback((target: SettingsOpen) => {
    if (openSheets.current > 0) {
      return;
    }
    const sheet = sheetRequestOf(target.sheet, target.account_id);
    if (sheet) {
      setPane("accounts");
      setRequest(sheet);
      return;
    }
    const next = asPane(target.pane);
    if (next) {
      setPane(next);
    }
  }, []);

  const requestAdd = useCallback(() => {
    setPane("accounts");
    setRequest({ sheet: "add", accountId: null });
  }, []);

  const clearRequest = useCallback(() => setRequest(null), []);

  const retry = useCallback(() => {
    retrySecrets()
      .then(reload)
      .catch((e) => {
        console.error("retry_secrets failed", e);
        setError(friendlyError(e));
      });
  }, [reload]);

  async function confirmReset() {
    setResetting(true);
    try {
      const reset = await resetSecrets();
      if (!reset) {
        setResetError(`The ${SECRET_STORE.item} couldn’t be reset.`);
        return;
      }
      setError(null);
      setResetError(null);
      setResetOpen(false);
      await reload();
    } catch (e) {
      console.error("reset_secrets failed", e);
      setError(friendlyError(e));
      setResetOpen(false);
    } finally {
      setResetting(false);
    }
  }

  // The window title names the current pane, the native toolbar highlights it, and it is
  // remembered for the next time Settings opens.
  useEffect(() => {
    storePane(pane);
    const title = PANES.find((p) => p.id === pane)?.title ?? "Settings";
    document.title = title;
    if (!isPreview()) {
      getCurrentWindow().setTitle(title).catch(() => undefined);
      selectSettingsPane(pane).catch(() => undefined);
    }
  }, [pane, storePane]);

  // Clicks on the native toolbar arrive as events.
  useEffect(() => {
    if (isPreview()) {
      return;
    }
    return subscribe(
      onSettingsPane((name) => {
        const next = asPane(name);
        if (next) {
          switchPane(next);
        }
      }),
    );
  }, [switchPane]);

  // An already open window is asked to show a pane or sheet through an event.
  useEffect(() => {
    if (isPreview()) {
      return;
    }
    return subscribe(onSettingsOpen(showTarget));
  }, [showTarget]);

  // ⌘1 to ⌘4 switch panes.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const index = Number(e.key) - 1;
      if (e.metaKey && index >= 0 && index < PANES.length) {
        e.preventDefault();
        switchPane(PANES[index].id);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [switchPane]);

  const context = useMemo<SettingsContextValue | null>(
    () =>
      view
        ? { view, snapshot, reload, setError, saveSettings, readOnly: view.read_only, requestAdd }
        : null,
    [view, snapshot, reload, saveSettings, requestAdd],
  );

  return (
    <OpenSheetsContext.Provider value={openSheets}>
      <main className="settings">
        {preview || !IS_MACOS ? (
          <nav className={IS_MACOS ? "toolbar" : "toolbar toolbar--titled"} aria-label="Settings panes">
            {PANES.map((p) => (
              <button
                key={p.id}
                type="button"
                className={`toolbar__tab ${pane === p.id ? "toolbar__tab--active" : ""}`}
                onClick={() => switchPane(p.id)}
                aria-current={pane === p.id ? "page" : undefined}
              >
                <Icon name={p.icon} size={20} />
                <span>{p.title}</span>
              </button>
            ))}
          </nav>
        ) : null}

        <div className="pane">
          {snapshot?.config_error ? (
            <div className="banner banner--warn" title={snapshot.config_error}>
              <Icon name="warning" size={14} />
              <span className="grow">Vigia couldn’t read its settings file, so changes won’t be saved.</span>
            </div>
          ) : view?.read_only ? (
            <div className="banner banner--warn">
              <Icon name="warning" size={14} />
              <span className="grow">Settings were saved by a newer version of Vigia. Update Vigia to change them.</span>
            </div>
          ) : null}
          {view?.secrets_blocked ? (
            <div className="banner banner--error">
              <Icon name="warning" size={14} />
              <span className="grow">Vigia can’t read its tokens from the {SECRET_STORE.store}.</span>
              <button type="button" onClick={retry}>
                Try Again
              </button>
              <button
                type="button"
                onClick={() => {
                  setResetError(null);
                  setResetOpen(true);
                }}
              >
                Reset {SECRET_STORE.itemTitle}…
              </button>
            </div>
          ) : null}
          {error ? (
            <div className="banner banner--error" role="alert">
              <Icon name="warning" size={14} />
              <span className="grow">{error}</span>
              <button type="button" className="icon" onClick={() => setError(null)} aria-label="Dismiss">
                <Icon name="xmark" size={12} />
              </button>
            </div>
          ) : null}

          {context ? (
            <SettingsContext.Provider value={context}>
              {pane === "accounts" ? <AccountsTab request={request} onRequestHandled={clearRequest} /> : null}
              {pane === "repos" ? <ReposTab /> : null}
              {pane === "branches" ? <BranchesTab /> : null}
              {pane === "general" ? <GeneralTab /> : null}
            </SettingsContext.Provider>
          ) : null}
        </div>

        {resetOpen ? (
          <Sheet
            title={`Reset ${SECRET_STORE.itemTitle}?`}
            message={`Vigia replaces its unreadable ${SECRET_STORE.item} with an empty one. All stored tokens are removed and must be entered again for every account.`}
            submitLabel="Reset"
            destructive
            busy={resetting}
            busyLabel="Resetting…"
            onCancel={() => setResetOpen(false)}
            onSubmit={confirmReset}
          >
            {resetError ? <SheetNote>{resetError}</SheetNote> : null}
          </Sheet>
        ) : null}
      </main>
    </OpenSheetsContext.Provider>
  );
}
