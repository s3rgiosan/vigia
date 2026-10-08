import { useState } from "react";
import { Icon } from "../components/Icon";
import type { StoppedRepo } from "../lib/contextMenu";
import type { Section } from "../lib/grouping";
import type { AccountSnapshot } from "../lib/snapshot";
import { RepoRow } from "./RepoRow";

/** A collapsible status section whose repositories are grouped by account and owner. */
export function SectionView({
  section,
  now,
  accounts,
  showAccount,
  forceOpen,
  onError,
  onStopped,
}: {
  section: Section;
  now: Date;
  accounts: Map<string, AccountSnapshot>;
  showAccount: boolean;
  forceOpen: boolean;
  onError: (e: unknown) => void;
  onStopped: (stopped: StoppedRepo) => void;
}) {
  const [open, setOpen] = useState(section.open);
  if (section.count === 0) {
    return null;
  }
  const expanded = open || forceOpen;
  return (
    <section className="section">
      <button
        type="button"
        className="section__header"
        data-nav="section"
        onClick={() => setOpen(!open)}
        onKeyDown={(e) => {
          if ((e.key === "ArrowRight" && !open) || (e.key === "ArrowLeft" && open)) {
            e.preventDefault();
            setOpen(!open);
          }
        }}
        aria-expanded={expanded}
      >
        <Icon name="disclosure" size={11} className={`disclosure ${expanded ? "disclosure--open" : ""}`} />
        <span>{section.title}</span>
        <span className="section__count numeric">{section.count}</span>
      </button>
      {expanded
        ? section.groups.map((group) => (
            <div className="group" key={`${group.account.id}-${group.org}`}>
              <div className="group__title">{showAccount ? `${group.account.label} · ${group.org}` : group.org}</div>
              {group.repos.map((repo) => (
                <RepoRow
                  key={`${repo.account_id}-${repo.repo.id}`}
                  repo={repo}
                  now={now}
                  isGitHub={(accounts.get(repo.account_id)?.kind ?? "github") === "github"}
                  onError={onError}
                  onStopped={onStopped}
                />
              ))}
            </div>
          ))
        : null}
    </section>
  );
}
