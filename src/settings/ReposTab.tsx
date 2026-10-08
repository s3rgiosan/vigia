import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon } from "../components/Icon";
import { PopUpButton } from "../components/PopUpButton";
import { SAML_HINT } from "../lib/github";
import { useRovingListbox } from "../lib/hooks/useRovingListbox";
import { buildPickerGroups, flattenGroups, toggleGroup } from "../lib/picker";
import { ROW_HEIGHT, visibleWindow } from "../lib/virtual";
import { useSettings } from "./SettingsContext";
import { useRepoLoader } from "./useRepoLoader";
import { useWatchedSave, type SaveState } from "./useWatchedSave";

/** Vertical padding of the scrolling list, which offsets every row. */
const LIST_PADDING = 4;

/** Rows rendered beyond each edge of the visible area. */
const OVERSCAN = 8;

function saveLabel(state: SaveState): ReactNode {
  switch (state) {
    case "pending":
      return "Saving…";
    case "saved":
      return (
        <>
          <span className="dot dot--green" aria-hidden="true" /> Saved
        </>
      );
    case "failed":
      return (
        <>
          <span className="dot dot--red" aria-hidden="true" /> Not saved
        </>
      );
    default:
      return null;
  }
}

export function ReposTab() {
  const { view, reload, setError, readOnly, requestAdd } = useSettings();
  const [chosenId, setChosenId] = useState(view.accounts[0]?.id ?? "");
  // The chosen account, or the first one when the chosen account was removed.
  const account = view.accounts.find((a) => a.id === chosenId) ?? view.accounts[0];
  const accountId = account?.id ?? "";
  const { repos, loading, loaded, refresh } = useRepoLoader(accountId, setError);
  const { selected, saveState, commit } = useWatchedSave(accountId, view.repos, reload, setError);
  const [search, setSearch] = useState("");
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(400);
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const noAccounts = view.accounts.length === 0;

  useEffect(() => {
    const el = listRef.current;
    if (!el) {
      return;
    }
    const measure = () => setViewport(el.clientHeight);
    measure();
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [noAccounts]);

  const groups = useMemo(() => buildPickerGroups(repos ?? [], search, selected), [repos, search, selected]);
  const rows = useMemo(() => flattenGroups(groups), [groups]);
  const { start, end } = visibleWindow(rows.length, ROW_HEIGHT, scrollTop, viewport, OVERSCAN);
  const activeIndex = Math.min(active, Math.max(rows.length - 1, 0));
  const activeVisible = rows.length > 0 && activeIndex >= start && activeIndex < end;

  /** Makes a row the active one and scrolls it into the visible area. */
  function moveTo(index: number) {
    setActive(index);
    const el = listRef.current;
    if (el) {
      const top = LIST_PADDING + index * ROW_HEIGHT;
      if (top < el.scrollTop) {
        el.scrollTop = top;
      } else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) {
        el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
      }
      setScrollTop(el.scrollTop);
    }
  }

  function toggleRow(index: number) {
    const row = rows[index];
    if (!row || readOnly) {
      return;
    }
    if (row.kind === "org") {
      commit(toggleGroup(row.group, selected));
    } else if (row.item.watched_via === null) {
      const next = new Set(selected);
      if (next.has(row.item.repo.id)) {
        next.delete(row.item.repo.id);
      } else {
        next.add(row.item.repo.id);
      }
      commit(next);
    }
  }

  const onListKey = useRovingListbox({
    count: rows.length,
    activeIndex,
    onMove: moveTo,
    onActivate: toggleRow,
    pageSize: Math.max(1, Math.floor(viewport / ROW_HEIGHT) - 1),
  });

  if (noAccounts) {
    return (
      <div className="empty-pane">
        <Icon name="folder" size={28} />
        <h2>No accounts</h2>
        <p>Add an account, then choose its repositories here.</p>
        <button type="button" className="default" onClick={requestAdd} disabled={readOnly || view.secrets_blocked}>
          Add Account…
        </button>
      </div>
    );
  }

  const loadingText = (
    <>
      <span className="spinner" aria-hidden="true" />
      {`Loading repositories…${loaded > 0 ? ` ${loaded} loaded` : ""}`}
    </>
  );

  return (
    <div className="repos">
      <div className="repos__bar">
        <PopUpButton
          label="Account"
          value={accountId}
          onChange={setChosenId}
          options={view.accounts.map((a) => ({ value: a.id, label: a.label }))}
        />
        <label className="search grow">
          <Icon name="search" size={15} />
          <input
            type="search"
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setActive(0);
              setScrollTop(0);
              if (listRef.current) {
                listRef.current.scrollTop = 0;
              }
            }}
            placeholder="Search"
            aria-label="Search repositories"
            spellCheck={false}
          />
        </label>
        <button
          type="button"
          className="icon"
          onClick={() => void refresh()}
          disabled={loading}
          title="Reload the repository list"
          aria-label="Reload repositories"
        >
          <Icon name="refresh" />
        </button>
      </div>

      <div
        className="repos__list form-group"
        ref={listRef}
        onScroll={(e) => {
          setScrollTop(e.currentTarget.scrollTop);
          setViewport(e.currentTarget.clientHeight);
        }}
      >
        {repos === null ? <p className="repos__status">{loadingText}</p> : null}
        {repos !== null && groups.length === 0 ? (
          <p className="repos__status">{search ? `No repositories match “${search}”.` : "This account has no repositories."}</p>
        ) : null}
        {/*
          One listbox holds every row so the list is a single Tab stop and a screen reader
          announces the full count through aria-setsize and aria-posinset even though only a
          window of rows is mounted. Organisation headers are options too: selecting one means
          "select all of its repositories", and its label states the partial case.
        */}
        <div
          role="listbox"
          aria-label="Repositories"
          aria-multiselectable="true"
          tabIndex={rows.length > 0 ? 0 : -1}
          aria-activedescendant={activeVisible ? `picker-opt-${activeIndex}` : undefined}
          onKeyDown={onListKey}
          style={{ paddingTop: start * ROW_HEIGHT, paddingBottom: (rows.length - end) * ROW_HEIGHT }}
        >
          {rows.slice(start, end).map((row, offset) => {
            const index = start + offset;
            const activeClass = index === activeIndex ? "picker-row--active" : "";
            const select = () => {
              setActive(index);
              toggleRow(index);
            };
            if (row.kind === "org") {
              const { group } = row;
              const partial = group.state === "some" ? ", some selected" : "";
              return (
                <div
                  key={`org:${group.org}`}
                  id={`picker-opt-${index}`}
                  role="option"
                  aria-selected={group.state === "all"}
                  aria-disabled={readOnly || undefined}
                  aria-label={`${group.org}, ${group.repos.length} repositories${partial}`}
                  aria-setsize={rows.length}
                  aria-posinset={index + 1}
                  className={`picker-row picker-row--org ${row.first ? "" : "picker-row--sep"} ${activeClass}`}
                  style={{ height: ROW_HEIGHT }}
                  onClick={select}
                >
                  <input
                    type="checkbox"
                    tabIndex={-1}
                    aria-hidden="true"
                    checked={group.state === "all"}
                    ref={(el) => {
                      if (el) {
                        el.indeterminate = group.state === "some";
                      }
                    }}
                    readOnly
                    disabled={readOnly}
                  />
                  <span className="grow">{group.org}</span>
                  <span className="picker-row__meta numeric">{group.repos.length}</span>
                </div>
              );
            }
            const { item } = row;
            const name = item.repo.full_name.slice(item.repo.full_name.lastIndexOf("/") + 1);
            const unavailable = readOnly || item.watched_via !== null;
            return (
              <div
                key={item.repo.id}
                id={`picker-opt-${index}`}
                role="option"
                aria-selected={selected.has(item.repo.id)}
                aria-disabled={unavailable || undefined}
                aria-setsize={rows.length}
                aria-posinset={index + 1}
                className={`picker-row ${item.watched_via ? "picker-row--disabled" : ""} ${activeClass}`}
                style={{ height: ROW_HEIGHT }}
                onClick={select}
              >
                <input
                  type="checkbox"
                  tabIndex={-1}
                  aria-hidden="true"
                  checked={selected.has(item.repo.id)}
                  readOnly
                  disabled={unavailable}
                />
                <span className="grow">{name}</span>
                {item.watched_via ? <span className="picker-row__meta">Watched via {item.watched_via}</span> : null}
              </div>
            );
          })}
        </div>
      </div>

      {account?.kind === "github" ? <p className="repos__hint muted">{SAML_HINT}</p> : null}

      <div className="repos__footer">
        <span className="numeric">{repos !== null && loading ? loadingText : `${selected.size} watched`}</span>
        <span className={`save-state save-state--${saveState}`} aria-live="polite">
          {saveLabel(saveState)}
        </span>
      </div>
    </div>
  );
}
