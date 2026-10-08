import { useCallback, useEffect, useRef, useState } from "react";
import { friendlyError } from "../lib/errors";
import { useLatest } from "../lib/hooks/useLatest";
import { listPickerRepos, onRepoListProgress, subscribe, type PickerRepo } from "../lib/tauri";

export interface RepoLoader {
  /** The account's repositories, or null until the first list arrives. */
  repos: PickerRepo[] | null;
  loading: boolean;
  /** Repositories received so far by the running load. */
  loaded: number;
  /** Fetches the list again from the server, bypassing the cache. */
  refresh: () => Promise<void>;
}

/**
 * Loads an account's repository list when the account changes, and reports page progress while it
 * loads. A response for an account that is no longer shown, or from a load that a newer one has
 * replaced, is dropped.
 */
export function useRepoLoader(accountId: string, setError: (message: string | null) => void): RepoLoader {
  const [list, setList] = useState<{ accountId: string; repos: PickerRepo[] } | null>(null);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(0);
  const ticket = useRef(0);
  const latest = useLatest({ accountId, setError });

  const load = useCallback(
    async (account: string, refresh: boolean) => {
      ticket.current += 1;
      const mine = ticket.current;
      setLoading(true);
      setLoaded(0);
      try {
        const repos = await listPickerRepos(account, refresh);
        if (mine === ticket.current) {
          latest.current.setError(null);
          setList({ accountId: account, repos });
        }
      } catch (e) {
        if (mine === ticket.current) {
          latest.current.setError(friendlyError(e));
        }
      } finally {
        if (mine === ticket.current) {
          setLoading(false);
        }
      }
    },
    [latest],
  );

  useEffect(() => {
    if (accountId) {
      void load(accountId, false);
    }
  }, [accountId, load]);

  useEffect(() => {
    return subscribe(
      onRepoListProgress((progress) => {
        if (progress.account_id === latest.current.accountId) {
          setLoaded(progress.loaded);
        }
      }),
    );
  }, [latest]);

  const refresh = useCallback(() => load(latest.current.accountId, true), [load, latest]);

  return {
    repos: list?.accountId === accountId ? list.repos : null,
    loading,
    loaded,
    refresh,
  };
}
