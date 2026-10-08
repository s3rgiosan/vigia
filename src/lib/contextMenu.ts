// Native right-click menus built with Tauri's menu API, so they look and behave like system menus.

import { LogicalPosition } from "@tauri-apps/api/dpi";
import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import type { RepoSnapshot, Run } from "./snapshot";
import { isPreview } from "./preview";
import { openUrl, unwatchRepo, type RestoreToken } from "./tauri";

/** A repository that was just removed from the watch list, with what Undo needs to bring it back. */
export interface StoppedRepo {
  token: RestoreToken;
  /** Repository name without its owner, for display. */
  name: string;
}

/** CI overview page of a repo: Actions on GitHub, Pipelines on GitLab. */
export function ciPage(repo: RepoSnapshot, isGitHub: boolean): string {
  const base = repo.repo.web_url.replace(/\/+$/, "");
  return isGitHub ? `${base}/actions` : `${base}/-/pipelines`;
}

/** Link that "Copy Link" puts on the clipboard: the run when there is one, else the repository. */
export function linkFor(repo: RepoSnapshot, run: Run | null): string {
  return run ? run.url : repo.repo.web_url;
}

/** Copies the run or repository link. */
export function copyLink(repo: RepoSnapshot, run: Run | null): Promise<void> {
  return writeText(linkFor(repo, run));
}

export interface RepoMenuHandlers {
  onError: (e: unknown) => void;
  /** Called after the repository was removed from the watch list. */
  onStopped: (stopped: StoppedRepo) => void;
  /** Window-relative position to open the menu at; omitted to open at the pointer. */
  at?: { x: number; y: number };
}

/** Stops watching a repo and passes its restore token to the caller once the change is saved. */
export function stopWatching(repo: RepoSnapshot, onStopped: (stopped: StoppedRepo) => void): Promise<void> {
  return unwatchRepo(repo.account_id, repo.repo.id).then((token) => {
    if (token) {
      const fullName = repo.repo.full_name;
      onStopped({ token, name: fullName.slice(fullName.lastIndexOf("/") + 1) });
    }
  });
}

export async function showRepoMenu(
  repo: RepoSnapshot,
  run: Run | null,
  isGitHub: boolean,
  { onError, onStopped, at }: RepoMenuHandlers,
): Promise<void> {
  if (isPreview()) {
    console.info("[preview] context menu for", repo.repo.full_name);
    return;
  }
  const guard = (action: Promise<unknown>) => {
    action.catch(onError);
  };
  const items = [
    ...(run
      ? [await MenuItem.new({ text: "Open Run", action: () => guard(openUrl(repo.account_id, run.url)) })]
      : []),
    await MenuItem.new({
      text: isGitHub ? "Open Actions" : "Open Pipelines",
      action: () => guard(openUrl(repo.account_id, ciPage(repo, isGitHub))),
    }),
    await MenuItem.new({ text: "Open Repository", action: () => guard(openUrl(repo.account_id, repo.repo.web_url)) }),
    await PredefinedMenuItem.new({ item: "Separator" }),
    await MenuItem.new({
      text: run ? "Copy Run Link" : "Copy Repository Link",
      accelerator: "CmdOrCtrl+C",
      action: () => guard(copyLink(repo, run)),
    }),
    await PredefinedMenuItem.new({ item: "Separator" }),
    await MenuItem.new({
      text: "Stop Watching",
      accelerator: "CmdOrCtrl+Backspace",
      action: () => guard(stopWatching(repo, onStopped)),
    }),
  ];
  const menu = await Menu.new({ items });
  try {
    await menu.popup(at ? new LogicalPosition(at.x, at.y) : undefined);
  } finally {
    // Closing the items a tick later lets the clicked item's action dispatch first.
    setTimeout(() => {
      for (const item of items) {
        item.close().catch(onError);
      }
    }, 0);
    await menu.close();
  }
}
