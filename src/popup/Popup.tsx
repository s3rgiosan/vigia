import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Icon } from "../components/Icon";
import type { StoppedRepo } from "../lib/contextMenu";
import { friendlyError } from "../lib/errors";
import { buildBanners, buildSections, formatInterval, relativeTime, summarize } from "../lib/grouping";
import { useDocumentVisible } from "../lib/hooks/useDocumentVisible";
import { IS_MACOS } from "../lib/platform";
import type { Snapshot, UpdateInfo } from "../lib/snapshot";
import {
  getSnapshot,
  hidePopup,
  onSnapshot,
  onUpdateCheckResult,
  openSettings,
  refreshNow,
  restoreWatchedRepo,
  setPaused,
  subscribe,
  type SettingsTarget,
} from "../lib/tauri";
import { BannerView } from "./BannerView";
import { SectionView } from "./SectionView";
import { UpdateBanner } from "./UpdateBanner";
import { shareUnchangedRepos } from "./shareSnapshot";
import { useTransientMessage } from "./useTransientMessage";
import { usePopupShortcuts } from "./usePopupShortcuts";
import { useWindowFit } from "./useWindowFit";
import "./Popup.css";

/** How long an error line stays visible. */
const ERROR_MS = 6000;

/** How long the Undo banner stays visible. */
const UNDO_MS = 8000;

/** Longest the refresh button stays busy when no snapshot arrives. */
const REFRESH_MS = 10_000;

/** How long the "up to date" banner stays visible. */
const UP_TO_DATE_MS = 6000;

/** How often relative times update while the popup is visible. */
const TICK_MS = 30_000;

/** Selector for every keyboard-navigable row, in document order. */
const NAV = "[data-nav]";

const PANEL_CLASS = IS_MACOS ? "popup" : "popup popup--opaque";

export function Popup() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [filter, setFilter] = useState("");
  const [now, setNow] = useState(() => new Date());
  const [error, showError] = useTransientMessage<string>(ERROR_MS);
  const [stopped, showStopped, clearStopped] = useTransientMessage<StoppedRepo>(UNDO_MS);
  const [upToDate, showUpToDate] = useTransientMessage<true>(UP_TO_DATE_MS);
  const [found, setFound] = useState<UpdateInfo | null>(null);
  const [refreshing, showRefreshing, clearRefreshing] = useTransientMessage<true>(REFRESH_MS);
  const panelRef = useRef<HTMLElement>(null);
  const filterRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const visible = useDocumentVisible();

  /** Logs a failed command and shows it briefly in the popup. */
  const report = useCallback(
    (e: unknown) => {
      console.error(e);
      showError(friendlyError(e));
    },
    [showError],
  );

  /** Stores a new snapshot; a refresh counts as finished once one arrives. */
  const receive = useCallback(
    (next: Snapshot) => {
      setSnapshot((prev) => shareUnchangedRepos(prev, next));
      clearRefreshing();
    },
    [clearRefreshing],
  );

  /** Focuses the list itself, with no visible selection, so the arrow keys work right away. */
  const focusList = useCallback(() => {
    if (filterRef.current?.value) {
      return;
    }
    listRef.current?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.add("popup-page");
    return () => root.classList.remove("popup-page");
  }, []);

  // The window is shown and hidden, not recreated, so the open animation restarts on each focus.
  useEffect(() => {
    const onFocus = () => {
      const panel = panelRef.current;
      if (panel) {
        panel.classList.remove("popup--opening");
        void panel.offsetWidth;
        panel.classList.add("popup--opening");
      }
      focusList();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [focusList]);

  // The first snapshot renders the list; focus it once.
  const hasSnapshot = snapshot !== null;
  useEffect(() => {
    if (hasSnapshot) {
      focusList();
    }
  }, [hasSnapshot, focusList]);

  useEffect(() => {
    getSnapshot().then(receive).catch(console.error);
    return subscribe(onSnapshot(receive));
  }, [receive]);

  // The menu bar's "Check for Updates…" reports back here.
  useEffect(
    () =>
      subscribe(
        onUpdateCheckResult(({ update, error }) => {
          if (error !== null) {
            console.error(error);
            showError(friendlyError(error));
          } else if (update !== null) {
            setFound(update);
          } else {
            showUpToDate(true);
          }
        }),
      ),
    [showError, showUpToDate],
  );

  // Relative times update only while the popup is on screen, and once more when it reappears.
  useEffect(() => {
    if (!visible) {
      return;
    }
    setNow(new Date());
    const tick = window.setInterval(() => setNow(new Date()), TICK_MS);
    return () => window.clearInterval(tick);
  }, [visible]);

  const noAccounts = snapshot !== null && snapshot.accounts.length === 0;
  const layout = snapshot === null ? "blank" : noAccounts ? "empty" : "list";
  useWindowFit(panelRef, layout);

  const refresh = useCallback(() => {
    showRefreshing(true);
    refreshNow().catch((e) => {
      clearRefreshing();
      report(e);
    });
  }, [report, showRefreshing, clearRefreshing]);

  const settings = useCallback(
    (target?: SettingsTarget) => {
      openSettings(target).then(hidePopup).catch(report);
    },
    [report],
  );

  const hide = useCallback(() => {
    hidePopup().catch(report);
  }, [report]);

  /** Puts a stopped repo back on the watch list. */
  const undoStop = useCallback(
    (entry: StoppedRepo) => {
      clearStopped();
      restoreWatchedRepo(entry.token)
        .then((restored) => {
          if (!restored) {
            showError("Couldn't restore it. Add it again in Settings.");
          }
        })
        .catch(report);
    },
    [clearStopped, report, showError],
  );

  usePopupShortcuts({ filterRef, setFilter, refresh, openSettings: settings, hide });

  const sections = useMemo(() => (snapshot ? buildSections(snapshot, filter) : []), [snapshot, filter]);
  const banners = useMemo(
    () => (snapshot ? buildBanners(snapshot, Math.floor(now.getTime() / 1000)) : []),
    [snapshot, now],
  );
  const accounts = useMemo(() => new Map((snapshot?.accounts ?? []).map((a) => [a.id, a])), [snapshot]);

  /** ↑ ↓ move focus between rows; ↓ from the filter enters the list. */
  function onListKey(e: KeyboardEvent<HTMLElement>) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") {
      return;
    }
    const rows = Array.from(listRef.current?.querySelectorAll<HTMLElement>(NAV) ?? []);
    if (rows.length === 0) {
      return;
    }
    e.preventDefault();
    const index = rows.indexOf(document.activeElement as HTMLElement);
    if (index === -1) {
      const first = rows.find((row) => row.dataset.nav === "row") ?? rows[0];
      first.focus();
      first.scrollIntoView({ block: "nearest" });
      return;
    }
    const next = e.key === "ArrowDown" ? index + 1 : index - 1;
    if (next < 0) {
      filterRef.current?.focus();
    } else if (next < rows.length) {
      rows[next].focus();
      rows[next].scrollIntoView({ block: "nearest" });
    }
  }

  if (!snapshot) {
    return <main className={PANEL_CLASS} ref={panelRef} />;
  }

  if (noAccounts) {
    return (
      <main className={`${PANEL_CLASS} popup--empty`} ref={panelRef}>
        <div className="empty" data-fit="empty">
          <div className="empty__mark" aria-hidden="true">
            <span className="dot dot--gray" />
          </div>
          <h1>Watch CI from the menu bar</h1>
          <p>Add a GitHub or GitLab account, pick repositories, and Vigia shows their state here.</p>
          <button type="button" className="default" onClick={() => settings({ pane: "accounts", sheet: "add" })}>
            Add Account…
          </button>
        </div>
      </main>
    );
  }

  const summary = summarize(snapshot);
  const updated = relativeTime(snapshot.generated_at, now);
  const stretched = snapshot.accounts.find((a) => a.effective_interval_secs > a.configured_interval_secs);
  const update = snapshot.update ?? found;
  const multipleAccounts = snapshot.accounts.length > 1;
  const nothingShown = sections.every((s) => s.count === 0);
  const noneWatched = snapshot.repos.length === 0;

  return (
    <main className={PANEL_CLASS} ref={panelRef} onKeyDown={onListKey}>
      <header className="popup__header">
        <span className={`dot dot--lg dot--${snapshot.color}`} aria-hidden="true" />
        <div className="popup__title" aria-live="polite">
          <strong className="numeric">{summary.headline}</strong>
          {summary.detail ? <span className="numeric">{summary.detail}</span> : null}
        </div>
        <div className="popup__actions">
          <button
            type="button"
            className={`icon ${refreshing ? "icon--spinning" : ""}`}
            onClick={refresh}
            disabled={snapshot.paused || refreshing !== null}
            title="Refresh Now (⌘R)"
            aria-label="Refresh now"
            aria-busy={refreshing !== null}
          >
            <Icon name="refresh" />
          </button>
          <button
            type="button"
            className="icon"
            onClick={() => setPaused(!snapshot.paused).catch(report)}
            title={snapshot.paused ? "Resume" : "Pause"}
            aria-label={snapshot.paused ? "Resume" : "Pause"}
          >
            <Icon name={snapshot.paused ? "play" : "pause"} />
          </button>
          <button type="button" className="icon" onClick={() => settings()} title="Settings (⌘,)" aria-label="Settings">
            <Icon name="gear" />
          </button>
        </div>
      </header>

      <label className="search">
        <Icon name="search" size={15} />
        <input
          ref={filterRef}
          type="search"
          placeholder="Filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          aria-label="Filter repositories"
          spellCheck={false}
          autoCorrect="off"
        />
      </label>

      {banners.length > 0 || error || stopped || upToDate || update ? (
        <div className="popup__banners">
          {error ? (
            <div className="banner banner--error" role="alert">
              <Icon name="warning" size={14} />
              {error}
            </div>
          ) : null}
          {stopped ? (
            <div className="banner banner--info" role="status">
              <span className="banner__text">Stopped watching {stopped.name}.</span>
              <button type="button" className="banner__action" onClick={() => undoStop(stopped)}>
                Undo
              </button>
            </div>
          ) : null}
          {update ? <UpdateBanner update={update} onError={report} /> : null}
          {upToDate ? (
            <div className="banner banner--info" role="status">
              <Icon name="check" size={14} />
              Vigia is up to date.
            </div>
          ) : null}
          {banners.map((banner) => (
            <BannerView
              key={"account" in banner ? `${banner.kind}-${banner.account.id}` : banner.kind}
              banner={banner}
              onOpenSettings={settings}
            />
          ))}
        </div>
      ) : null}

      <div className="popup__list" ref={listRef} tabIndex={-1} data-fit="list">
        <div className="popup__list-inner" data-fit="inner">
          {nothingShown && noneWatched ? (
            <div className="popup__none">
              <p>No repositories watched.</p>
              <button type="button" onClick={() => settings({ pane: "repos" })}>
                Choose Repositories…
              </button>
            </div>
          ) : nothingShown ? (
            <p className="popup__none">{`No repositories match “${filter}”.`}</p>
          ) : (
            sections.map((section) => (
              <SectionView
                key={section.id}
                section={section}
                now={now}
                accounts={accounts}
                showAccount={multipleAccounts}
                forceOpen={filter !== ""}
                onError={report}
                onStopped={showStopped}
              />
            ))
          )}
        </div>
      </div>

      <footer className="popup__footer numeric" aria-live="polite">
        {snapshot.paused ? "Paused · " : ""}
        {updated ? `Updated ${updated}` : "Not updated yet"}
        {stretched ? (
          <span title="Slowed down to stay within the API rate limit">
            {" "}
            · slowed to every {formatInterval(stretched.effective_interval_secs)}
          </span>
        ) : null}
      </footer>
    </main>
  );
}
