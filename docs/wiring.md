# Wiring

Every menu item, context-menu item and keyboard shortcut, with the code that handles it and the event it emits, and every permission with the file that declares it. `docs/architecture.md` describes the parts; this page is the checklist.

## Status item menu

Built in `tray::build` (`src-tauri/src/tray.rs`). IDs map to actions in `MenuAction::from_id`; the handler is registered as a global menu listener, so the same IDs from other menus run it too.

| Item | ID | Shortcut | Handler | Emits |
| --- | --- | --- | --- | --- |
| About Vigia | predefined | | AppKit About panel (`about_metadata`) | |
| Open Vigia | `open` | | `tray::show_popup` | |
| Refresh Now | `refresh` | Cmd+R | `Actions::refresh`, `Runtime::refresh_now` | `snapshot-updated` |
| Pause / Resume | `pause` | | `Actions::toggle_pause`, `Runtime::set_paused` | `snapshot-updated` |
| Settings… | `settings` | Cmd+, | `Actions::open_settings`, `windows::open_settings` | `settings-open` when Settings is already open |
| Check for Updates… | `check-updates` | | `Actions::check_for_updates` | `update-check-result` to the popup |
| Quit Vigia | `quit` | Cmd+Q | `AppHandle::exit` | |
| Debug → each look (debug builds) | `debug-look-*` | | `tray::set_look` | |

A left click on the status item toggles the popup (`tray::toggle_popup`). Opening the app again from Finder or Spotlight shows the popup (`RunEvent::Reopen` in `lib.rs`); a second process does the same through `tauri-plugin-single-instance`.

## Popup More menu

Built in `tray::more_menu`, opened by the footer's More button through the `show_more_menu` command (`src/popup/Popup.tsx`). Its items reuse the status item IDs.

| Item | ID | Handler |
| --- | --- | --- |
| About Vigia | predefined | AppKit About panel |
| Settings… | `settings` | `Actions::open_settings` |
| Quit Vigia | `quit` | `AppHandle::exit` |

## macOS menu bar

Built in `app_menu::build` (`src-tauri/src/app_menu.rs`), shown while Settings is open. `app_menu::handle_event` runs the items this module owns; `settings` and `quit` go to the status item handler.

| Menu | Item | ID | Shortcut | Handler |
| --- | --- | --- | --- | --- |
| Vigia | About Vigia | predefined | | AppKit About panel |
| Vigia | Settings… | `settings` | Cmd+, | `Actions::open_settings` |
| Vigia | Services, Hide Vigia, Hide Others, Show All | predefined | Cmd+H, Option-Cmd+H | AppKit |
| Vigia | Quit Vigia | `quit` | Cmd+Q | `AppHandle::exit` |
| File | Close Window | predefined | Cmd+W | AppKit |
| Edit | Undo, Redo, Cut, Copy, Paste, Select All | predefined | standard | AppKit |
| Edit | Delete | `edit-delete` | | `document.execCommand('delete')` in the focused window |
| View | Actual Size | `zoom-reset` | Cmd+0 | `zoom::apply` on the focused window |
| View | Zoom In | `zoom-in` | Cmd+= | `zoom::apply` on the focused window |
| View | Zoom Out | `zoom-out` | Cmd+- | `zoom::apply` on the focused window |
| Window | Minimize, Zoom, Bring All to Front | predefined | Cmd+M | AppKit |
| Help | Vigia Help | `help` | | opens `app_menu::HELP_URL` |

## Row context menu

Built in `showRepoMenu` (`src/lib/contextMenu.ts`) with `@tauri-apps/api/menu`; each item carries its action. The menu shows no shortcuts.

| Item | Shown | Handler | Emits |
| --- | --- | --- | --- |
| Open Run | rows with a run | `open_url` command | |
| Open Actions / Open Pipelines | always | `open_url` command | |
| Open Repository | always | `open_url` command | |
| Copy Run Link / Copy Repository Link | always | `copyLink`, clipboard plugin `write_text` | |
| Stop Watching | always | `stopWatching`, `unwatch_repo` command | `snapshot-updated` |

## Settings pull-down menu

The "•••" button on the Accounts pane (`ActionPopUpButton` in `src/components/PopUpButton.tsx`) opens a native menu built by `showActionMenu` (`src/lib/actionMenu.ts`).

| Item | Handler |
| --- | --- |
| Remove All Accounts… | `AccountsTab` `removeAll`: `confirm_destructive`, then `delete_all_accounts` |

## Native alerts

`confirm_destructive` (`src-tauri/src/commands.rs`) shows an alert sheet on the calling window with the confirming button and Cancel.

| Alert | Opened from | On confirm |
| --- | --- | --- |
| Remove “<account>”? | Accounts pane, remove button | `delete_account` |
| Remove all <n> accounts? | Accounts pane, Remove All Accounts… | `delete_all_accounts` |
| Reset Keychain Item? (Credential, Keyring Item) | Settings banner, Reset … Item… | `reset_secrets` |

## Keyboard shortcuts

⌘ on macOS; Ctrl on Linux and Windows (`hasCommandModifier` in `src/lib/platform.ts`).

| Window | Shortcut | Handler |
| --- | --- | --- |
| Popup | Cmd+R | `usePopupShortcuts`: refresh |
| Popup | Cmd+, | `usePopupShortcuts`: open Settings and hide the popup |
| Popup | Cmd+F | `usePopupShortcuts`: focus the filter |
| Popup | Escape | `usePopupShortcuts`: clear the filter, else hide the popup |
| Popup | typing | `usePopupShortcuts`: start filtering |
| Popup | Up, Down | `Popup` `onListKey`: move between rows |
| Popup row | Cmd+C | `handleRowShortcut`: copy the link |
| Popup row | Cmd+Backspace | `handleRowShortcut`: stop watching |
| Popup row | Menu key, Shift+F10 | `handleRowShortcut`: open the context menu |
| Settings | Cmd+1 to Cmd+4 | `Settings`: switch panes |
| Settings (Linux, Windows) | Ctrl+0, Ctrl+=, Ctrl+- | `Settings`: `zoom_window` |
| Settings sheets | Escape, Cmd+. | `Sheet`: cancel |
| Settings sheets | Return | `Sheet`: the default button |

## Events

| Event | Emitted by | Listened to by |
| --- | --- | --- |
| `snapshot-updated` | `lib.rs` `publish` | `Popup`, `Settings` |
| `settings-open` | `windows::open_settings_on_main` | `Settings` |
| `settings-pane` | `settings_toolbar.rs` | `Settings` |
| `repo-list-progress` | `commands::list_picker_repos` | `useRepoLoader` |
| `update-progress` | `commands::install_update` | `useInstallUpdate` |
| `update-check-result` | `Actions::check_for_updates` | `Popup` |

## Permissions

| Permission | Declared in | Used by |
| --- | --- | --- |
| `core:event:allow-listen`, `core:event:allow-unlisten` | `capabilities/popup.json`, `capabilities/settings.json` | event subscriptions in both windows |
| `core:menu:allow-new`, `core:menu:allow-popup`, `core:resources:allow-close` | `capabilities/popup.json` | row context menu |
| `core:menu:allow-new`, `core:menu:allow-popup`, `core:resources:allow-close` | `capabilities/settings.json` | Accounts pull-down menu |
| `core:window:allow-set-size` | `capabilities/popup.json` | `useWindowFit` |
| `clipboard-manager:allow-write-text` | `capabilities/popup.json` | Copy Run Link, Cmd+C |
| `core:app:allow-version` | `capabilities/settings.json` | General pane version row |
| `core:window:allow-set-title` | `capabilities/settings.json` | window title follows the pane |
| App commands | no entries; both windows may call them | `src/lib/tauri.ts` |
| Notifications | `tauri-plugin-notification` from Rust; no window permission | `lib.rs` `send_notification` |
| Launch at login | `tauri-plugin-autostart` from Rust | `windows::sync_autostart` |
| Keychain items | `KeychainStore` from Rust | `secrets.rs` |
