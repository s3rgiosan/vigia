import { useState } from "react";
import { Icon } from "../components/Icon";
import { Sheet } from "../components/Sheet";
import type { WatchedRepo } from "../lib/tauri";
import { repoKey, repoName } from "./filters";

/**
 * Picks repositories of one organization to give their own filters. Each row shows the name
 * without the owner, and the account in muted text when `labelOf` is given. The sheet widens to fit
 * the longest name.
 */
export function AddExceptionsSheet({
  repos,
  labelOf,
  onCancel,
  onAdd,
}: {
  repos: WatchedRepo[];
  labelOf?: (repo: WatchedRepo) => string;
  onCancel: () => void;
  onAdd: (keys: string[]) => void;
}) {
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<string[]>([]);
  const needle = query.trim().toLowerCase();
  const shown = repos.filter((repo) => repoName(repo).toLowerCase().includes(needle));

  function toggle(key: string) {
    setChosen((current) => (current.includes(key) ? current.filter((k) => k !== key) : [...current, key]));
  }

  return (
    <Sheet
      title="Add Exceptions"
      submitLabel="Add"
      submitDisabled={chosen.length === 0}
      width="fit"
      onCancel={onCancel}
      onSubmit={() => onAdd(repos.map(repoKey).filter((key) => chosen.includes(key)))}
    >
      <label className="search">
        <Icon name="search" size={15} />
        <input
          type="search"
          aria-label="Search repositories"
          placeholder="Search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          spellCheck={false}
        />
      </label>
      <ul className="exceptions__picker form-group" aria-label="Repositories">
        {shown.length === 0 ? <li className="exceptions__empty">No matching repositories.</li> : null}
        {shown.map((repo) => {
          const key = repoKey(repo);
          return (
            <li key={key} className="picker-row">
              <label className="exceptions__choice">
                <input type="checkbox" checked={chosen.includes(key)} onChange={() => toggle(key)} />
                <span className="exceptions__name">{repoName(repo)}</span>
                {labelOf ? <span className="picker-row__meta">{labelOf(repo)}</span> : null}
              </label>
            </li>
          );
        })}
      </ul>
    </Sheet>
  );
}
