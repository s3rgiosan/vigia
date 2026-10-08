import { memo, type RefObject } from "react";
import { Icon } from "../components/Icon";
import { showRepoMenu, type StoppedRepo } from "../lib/contextMenu";
import { relativeTime, runColor, runStateLabel } from "../lib/grouping";
import type { RepoSnapshot, Run } from "../lib/snapshot";
import { openUrl } from "../lib/tauri";
import { handleRowShortcut } from "./rowShortcuts";

interface RunRowProps {
  repo: RepoSnapshot;
  run: Run;
  now: Date;
  isGitHub: boolean;
  /** The repository row to refocus on Left arrow. */
  repoRowRef: RefObject<HTMLButtonElement | null>;
  onError: (e: unknown) => void;
  onStopped: (stopped: StoppedRepo) => void;
}

/** One workflow run under an expanded repository. */
export const RunRow = memo(function RunRow({ repo, run, now, isGitHub, repoRowRef, onError, onStopped }: RunRowProps) {
  const state = runStateLabel(run.state);
  const pulsing = run.state === "running" || run.state === "queued";
  return (
    <button
      type="button"
      className="run"
      data-nav="run"
      onClick={() => openUrl(repo.account_id, run.url).catch(onError)}
      onContextMenu={(e) => {
        e.preventDefault();
        showRepoMenu(repo, run, isGitHub, { onError, onStopped }).catch(onError);
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowLeft") {
          e.preventDefault();
          repoRowRef.current?.focus();
          return;
        }
        handleRowShortcut(e, repo, run, isGitHub, onError, onStopped);
      }}
      title={`Open ${run.name} run in browser`}
      aria-label={`Open ${run.name} run in browser, ${state}, ${run.branch}`}
    >
      <span className={`dot dot--sm dot--${runColor(run.state)} ${pulsing ? "dot--pulse" : ""}`} aria-hidden="true" />
      <span className="visually-hidden">{state}: </span>
      <span className="run__name">{run.name}</span>
      <span className="run__branch">{run.branch}</span>
      <span className="run__time numeric">{relativeTime(run.updated_at, now)}</span>
      <Icon name="external" size={12} className="open-mark" />
    </button>
  );
});
