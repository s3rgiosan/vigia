import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { friendlyError } from "../lib/errors";
import { useLatest } from "../lib/hooks/useLatest";
import { getSettings, setWatched, type WatchedRepo } from "../lib/tauri";

/** Picks are saved this long after the last change, so a burst of clicks restarts polling once. */
export const SAVE_DELAY_MS = 700;

export type SaveState = "idle" | "pending" | "saved" | "failed";

/** Repo id to whether the user wants it watched. */
type Edits = ReadonlyMap<number, boolean>;

/** Unsaved picks and the account they belong to. */
interface Pending {
  accountId: string;
  edits: Edits;
}

const NO_EDITS: Edits = new Map();

function applyEdits(base: ReadonlySet<number>, edits: Edits): Set<number> {
  const next = new Set(base);
  for (const [id, watched] of edits) {
    if (watched) {
      next.add(id);
    } else {
      next.delete(id);
    }
  }
  return next;
}

export interface WatchedSave {
  /** The watched repo ids as shown: the server's list with unsaved picks applied. */
  selected: Set<number>;
  saveState: SaveState;
  /** Records the picks that make `next` the selection and saves them after a short pause. */
  commit: (next: Set<number>) => void;
}

/**
 * Keeps the Repositories picker's selection and saves it. The selection is the account's watched
 * repos from the settings with the user's unsaved picks on top, so changes made elsewhere show up
 * while a pick waits. Saves run one at a time and apply the picks to a fresh copy of the server's
 * list. A failed save stays queued and is retried when the pane closes; leaving the account saves
 * its picks and then drops them.
 */
export function useWatchedSave(
  accountId: string,
  repos: WatchedRepo[],
  reload: () => Promise<void>,
  setError: (message: string | null) => void,
): WatchedSave {
  const server = useMemo(
    () => new Set(repos.filter((r) => r.account_id === accountId).map((r) => r.repo.id)),
    [repos, accountId],
  );
  const [pending, setPending] = useState<Pending>({ accountId, edits: NO_EDITS });
  const [saveState, setSaveState] = useState<SaveState>("idle");
  // Saves run outside render, so they read the picks and the dirty flag from refs that change
  // together with the state above.
  const pendingRef = useRef(pending);
  const dirty = useRef(false);
  const timer = useRef<number | undefined>(undefined);
  const queue = useRef<Promise<void>>(Promise.resolve());
  const latest = useLatest({ accountId, reload, setError });

  const writePending = useCallback((next: Pending) => {
    pendingRef.current = next;
    setPending(next);
  }, []);

  const save = useCallback(async () => {
    if (!dirty.current) {
      return;
    }
    dirty.current = false;
    const { accountId: target, edits: sent } = pendingRef.current;
    const current = () => latest.current.accountId === target;
    try {
      const fresh = await getSettings();
      const base = new Set(fresh.repos.filter((r) => r.account_id === target).map((r) => r.repo.id));
      await setWatched(target, [...applyEdits(base, sent)]);
      latest.current.setError(null);
      await latest.current.reload();
      // Picks unchanged since they were sent are part of the server's list now.
      const after = pendingRef.current;
      if (after.accountId === target) {
        const remaining = new Map([...after.edits].filter(([id, watched]) => sent.get(id) !== watched));
        writePending({ accountId: target, edits: remaining });
      }
      if (current()) {
        setSaveState(dirty.current ? "pending" : "saved");
      }
    } catch (e) {
      latest.current.setError(friendlyError(e));
      if (current()) {
        dirty.current = true;
        setSaveState("failed");
      } else if (pendingRef.current.accountId === target) {
        writePending({ accountId: latest.current.accountId, edits: NO_EDITS });
      }
    }
  }, [latest, writePending]);

  /** Runs saves one at a time and resolves when the queue has drained. */
  const flush = useCallback(() => {
    window.clearTimeout(timer.current);
    queue.current = queue.current.then(save);
    return queue.current;
  }, [save]);

  // Leaving an account, or closing the pane, saves what is still pending.
  useEffect(() => {
    return () => {
      void flush();
      setSaveState("idle");
    };
  }, [accountId, flush]);

  const edits = pending.accountId === accountId ? pending.edits : NO_EDITS;
  const selected = useMemo(() => applyEdits(server, edits), [server, edits]);

  const commit = useCallback(
    (next: Set<number>) => {
      const changes = new Map(edits);
      for (const id of new Set([...selected, ...next])) {
        if (selected.has(id) !== next.has(id)) {
          changes.set(id, next.has(id));
        }
      }
      writePending({ accountId, edits: changes });
      dirty.current = true;
      setSaveState("pending");
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => void flush(), SAVE_DELAY_MS);
    },
    [accountId, edits, selected, flush, writePending],
  );

  return { selected, saveState, commit };
}
