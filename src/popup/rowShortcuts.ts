import type { KeyboardEvent } from "react";
import { copyLink, showRepoMenu, stopWatching, type StoppedRepo } from "../lib/contextMenu";
import type { RepoSnapshot, Run } from "../lib/snapshot";

/**
 * Keyboard access to a row's context actions: the Menu key or Shift+F10 opens the menu,
 * ⌘C copies the link and ⌘⌫ stops watching.
 */
export function handleRowShortcut(
  e: KeyboardEvent<HTMLElement>,
  repo: RepoSnapshot,
  run: Run | null,
  isGitHub: boolean,
  onError: (e: unknown) => void,
  onStopped: (stopped: StoppedRepo) => void,
): boolean {
  if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
    e.preventDefault();
    const rect = e.currentTarget.getBoundingClientRect();
    const at = { x: rect.left + 24, y: rect.bottom };
    showRepoMenu(repo, run, isGitHub, { onError, onStopped, at }).catch(onError);
    return true;
  }
  if (e.metaKey && e.key === "c") {
    e.preventDefault();
    copyLink(repo, run).catch(onError);
    return true;
  }
  if (e.metaKey && e.key === "Backspace") {
    e.preventDefault();
    stopWatching(repo, onStopped).catch(onError);
    return true;
  }
  return false;
}
