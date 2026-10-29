// Native pull-down menus for action buttons, built with Tauri's menu API.

import { LogicalPosition } from "@tauri-apps/api/dpi";
import { Menu, MenuItem } from "@tauri-apps/api/menu";
import { isPreview } from "./preview";

export interface MenuAction {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
}

/** Opens a native menu of `actions` at a window-relative position; unavailable items show dimmed. */
export async function showActionMenu(actions: MenuAction[], at: { x: number; y: number }): Promise<void> {
  if (isPreview()) {
    console.info("[preview] pull-down menu", actions.map((a) => a.label));
    return;
  }
  const items = await Promise.all(
    actions.map((a) => MenuItem.new({ text: a.label, enabled: !a.disabled, action: a.onSelect })),
  );
  const menu = await Menu.new({ items });
  try {
    await menu.popup(new LogicalPosition(at.x, at.y));
  } finally {
    // Closing the items a tick later lets the clicked item's action dispatch first.
    setTimeout(() => {
      for (const item of items) {
        item.close().catch((e) => console.error(e));
      }
    }, 0);
    await menu.close();
  }
}
