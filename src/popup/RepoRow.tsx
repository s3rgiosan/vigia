import { memo, useMemo, useRef, useState } from "react";
import { Icon } from "../components/Icon";
import { ciPage, showRepoMenu, type StoppedRepo } from "../lib/contextMenu";
import { NO_ACCESS_NOTE, SAML_HINT } from "../lib/github";
import { relativeTime, sortRuns, statusLabel } from "../lib/grouping";
import type { RepoSnapshot, TrayColor } from "../lib/snapshot";
import { openUrl } from "../lib/tauri";
import { RunRow } from "./RunRow";
import { handleRowShortcut } from "./rowShortcuts";

interface RepoRowProps {
  repo: RepoSnapshot;
  now: Date;
  isGitHub: boolean;
  onError: (e: unknown) => void;
  onStopped: (stopped: StoppedRepo) => void;
}

function colorOf(repo: RepoSnapshot): TrayColor {
  switch (repo.state.status) {
    case "failed":
      return "red";
    case "error":
      return "orange";
    case "running":
      return "yellow";
    case "success":
      return "green";
    default:
      return "gray";
  }
}

/** One repository line, expandable to its workflow runs. */
export const RepoRow = memo(function RepoRow({ repo, now, isGitHub, onError, onStopped }: RepoRowProps) {
  const [expanded, setExpanded] = useState(false);
  const repoRowRef = useRef<HTMLButtonElement>(null);
  const runs = useMemo(() => sortRuns(repo.state.groups), [repo.state.groups]);
  const run = repo.state.representative;
  const url = run?.url ?? ciPage(repo, isGitHub);
  const name = repo.repo.full_name.slice(repo.repo.full_name.lastIndexOf("/") + 1);
  const samlHint = isGitHub && repo.state.note === NO_ACCESS_NOTE;
  const expandable = runs.length > 1 || samlHint;
  const color = colorOf(repo);
  const state = statusLabel(repo.state.status);
  const stale = repo.state.stale;
  const time = stale && repo.state.last_checked ? repo.state.last_checked : run?.updated_at;

  return (
    <div className={`repo ${expanded ? "repo--expanded" : ""}`}>
      <div className="repo__line">
        {expandable ? (
          <button
            type="button"
            className="row__disclosure"
            aria-expanded={expanded}
            aria-label={`Show runs for ${name}`}
            onClick={() => setExpanded(!expanded)}
          >
            <Icon name="disclosure" size={9} className={`disclosure ${expanded ? "disclosure--open" : ""}`} />
          </button>
        ) : (
          <span className="row__disclosure" aria-hidden="true" />
        )}
        <button
          type="button"
          className="row"
          ref={repoRowRef}
          data-nav="row"
          onClick={() => openUrl(repo.account_id, url).catch(onError)}
          onContextMenu={(e) => {
            e.preventDefault();
            showRepoMenu(repo, run, isGitHub, { onError, onStopped }).catch(onError);
          }}
          onKeyDown={(e) => {
            if (expandable && ((e.key === "ArrowRight" && !expanded) || (e.key === "ArrowLeft" && expanded))) {
              e.preventDefault();
              setExpanded(!expanded);
              return;
            }
            handleRowShortcut(e, repo, run, isGitHub, onError, onStopped);
          }}
          title={`${state}: ${repo.repo.full_name}${run ? ` — ${run.name} on ${run.branch}` : ""}`}
        >
          <span className={`dot dot--${color} ${color === "yellow" ? "dot--pulse" : ""}`} aria-hidden="true" />
          <span className="visually-hidden">{state}: </span>
          <span className="row__name">{name}</span>
          <span className="row__detail">
            {repo.state.note ?? (run ? `${run.name} · ${run.branch}` : "No runs")}
          </span>
          <span className="row__time numeric">
            {stale ? (
              <>
                <Icon name="warning" size={11} className="row__stale" />
                <span className="visually-hidden">Stale </span>
              </>
            ) : null}
            {time ? relativeTime(time, now) : ""}
          </span>
          <Icon name="external" size={12} className="open-mark" />
        </button>
      </div>
      {expanded && samlHint ? <p className="repo__hint">{SAML_HINT}</p> : null}
      {expanded && runs.length > 0 ? (
        <ul className="runs">
          {runs.map((r) => (
            <li key={`${r.branch}-${r.group}`}>
              <RunRow
                repo={repo}
                run={r}
                now={now}
                isGitHub={isGitHub}
                repoRowRef={repoRowRef}
                onError={onError}
                onStopped={onStopped}
              />
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
});
