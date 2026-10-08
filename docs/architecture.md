# Architecture

Vigia shows the CI status of watched repositories as a colored badge on a menu bar icon, with a popup for details. This page describes how the app works as the code stands. Each section names the source files it describes.

## 1. Overview and scope

Vigia runs on macOS as a menu bar app, and on Linux and Windows as a tray app. It reads GitHub Actions runs from github.com and GitLab CI pipelines from gitlab.com or a self-managed GitLab instance. The Rust backend in `src-tauri/` owns polling, state, notifications, storage and the tray. The React and TypeScript frontend in `src/` renders two windows, the popup and Settings, and holds no state of its own beyond the latest snapshot.

Stack: Tauri v2, Rust 2021 (`rust-version` 1.94; `[lints.clippy] all = "deny"`; reqwest, tokio, globset, keyring, objc2 for AppKit), React 19, Vite, Vitest.

Every request to a provider is a `GET`, so the app reads status and never changes anything on the server.

Out of scope:

- GitHub Enterprise Server (the GitHub API origin is fixed to `https://api.github.com`).
- OAuth or device-flow sign-in; accounts use personal access tokens.
- Linux and Windows on Arm, and Linux packages other than AppImage and deb.
- Triggering, cancelling or re-running jobs, and viewing logs.
- GitLab child pipelines and other CI systems.
- Per-account pause (pause is global).
- Editing the base URL of an existing GitLab account.
- Developer ID signing, notarization and the Mac App Store.

## 2. Process layout

Entry point: `src-tauri/src/lib.rs` (`run`). Window creation lives in `windows.rs` and `tray.rs`; the window definition is in `tauri.conf.json` and the platform files beside it.

### Platforms

`tauri.conf.json` holds the shared configuration. `tauri.macos.conf.json`, `tauri.linux.conf.json` and `tauri.windows.conf.json` are merged over it (JSON Merge Patch) and set each platform's bundle targets (`dmg`; `appimage` and `deb`; `nsis` for the current user) and its popup window entry. Arrays replace the base value whole, so each platform file that changes the popup repeats its full entry. `macOSPrivateApi` stays in the base file, because `tauri-build` checks it against the `macos-private-api` Cargo feature; it has no effect off macOS.

AppKit code is behind `cfg(target_os = "macos")`, with no-op or plain Tauri fallbacks elsewhere. The frontend learns the platform from `TAURI_ENV_PLATFORM`, which the Tauri CLI sets for `tauri dev` and `tauri build` and Vite exposes through `envPrefix` (`src/lib/platform.ts`). The browser preview and the tests have no platform and follow macOS.

| Area | macOS | Linux and Windows |
| --- | --- | --- |
| Popup window | transparent, with window effects | opaque and square (`popup--opaque`) |
| Tray image | template image plus an AppKit badge view | one finished image per look |
| Tray click | toggles the popup | Windows toggles the popup; Linux opens the tray menu, whose first item is Open Vigia |
| Settings pane switcher | native toolbar | the page's own toolbar (`toolbar--titled`) |
| Icons | SF Symbols | Lucide |
| Token store | Keychain | Credential Manager; Secret Service keyring |
| Notification click | opens the run or the popup | none |

### Windows

| Window | Label | Size | Behavior |
| --- | --- | --- | --- |
| Popup | `popup` | 380 wide, height set by the page | Created hidden at launch. No decorations, always on top. On macOS it is transparent, with the `liquidGlassRegular` and `popover` window effects. Hides when it loses focus. |
| Settings | `settings` | 680 x 560, fixed | Created on demand, focused when it already exists. On macOS it has a native AppKit preferences toolbar (`settings_toolbar.rs`) that the page disables while a sheet is open. |

Both windows load `index.html`; the `view` query parameter (`popup` or `settings`) selects the React view (`src/lib/view.ts`). Settings also accepts `pane` (`accounts`, `repos`, `branches`, `general`; the `branches` pane is titled Filters), `sheet` (`add`, or `replace` with an `account` parameter naming an existing account), and `account`; unknown values are dropped, and a `replace` sheet without a known account is dropped with it (`SettingsTarget`, `SettingsPane` and `SettingsSheet` in `windows.rs`).

### Tray and activation policy

The app starts with the Accessory activation policy, so it has no Dock icon. While Settings is open the policy is Regular, which lets the window take keyboard focus; it returns to Accessory when Settings is destroyed. Closing every window does not exit the app (`ExitRequested` is prevented); Quit in the tray menu exits.

The tray icon (`tray.rs`) is the porthole mark as a template image. A status badge is composited on its lower right corner:

| Look | Condition | Icon |
| --- | --- | --- |
| Failed, Error, Running, Success | tray color red, orange, yellow, green | porthole with a colored badge |
| Idle | tray color gray | plain porthole |
| Paused | paused | dimmed porthole, no badge |

Linux and Windows show tray images untinted and have no status item to lay a badge over, so each look there is one finished image (`solid-<look>@2x.png`): the porthole in white with the dark-menu-bar badge drawn into its notch.

Badge artwork exists for light and dark menu bars. A zero-sized view added to the status item receives `viewDidChangeEffectiveAppearance` when the menu bar turns light or dark, and the icon is redrawn then. The status item's accessibility label is `Vigia, <tooltip>`, kept in step with the tooltip. A left click toggles the popup. A click within 200 ms of a blur-triggered hide does not reopen it. The right-click menu has Open Vigia, Refresh Now (Cmd+R), Pause or Resume, Settings (Cmd+,), About, Check for Updates, and Quit (Cmd+Q). The About panel shows the version once (the build number is hidden) and the description as its credits. Debug builds add a Debug submenu that forces each look.

The popup is placed by the tray icon with `tauri-plugin-positioner` (`tray::place_popup`): under it on macOS and Linux, and above it on Windows, constrained to the screen, where it moves below the icon when the taskbar is at the top. On Windows each resize of the visible popup places it again, so it grows upward from the taskbar. When the tray rectangle is unknown, as always on Linux, it falls back to below the icon, then to the top-right of the primary monitor's work area. Wayland ignores window positions, so there the compositor places the popup.

### Single instance

`tauri-plugin-single-instance` makes a second launch show the popup of the running instance.

### Webview to Rust

The webview calls Rust through the commands in `commands.rs` (30 in `invoke_handler`):

| Group | Commands |
| --- | --- |
| Status | `get_snapshot`, `refresh_now`, `set_paused` |
| Windows and links | `hide_popup`, `open_settings`, `select_settings_pane`, `set_settings_toolbar_enabled`, `open_url`, `open_github_token_page` |
| Settings read | `get_settings` |
| Accounts | `test_connection`, `add_account`, `rename_account`, `replace_token`, `delete_account`, `delete_all_accounts` |
| Organizations | `set_org_filters` |
| Repositories | `list_picker_repos`, `set_watched`, `unwatch_repo`, `restore_watched_repo`, `clear_repo_overrides`, `set_repo_branches`, `set_repo_ignored_workflows`, `set_repo_include_tags` |
| Settings | `update_settings` |
| Keychain | `retry_secrets`, `reset_secrets` |
| Updates | `check_for_updates_now`, `install_update` |

`open_settings` takes an optional `pane`, `sheet` (`add` or `replace`) and `account_id`. `set_settings_toolbar_enabled` enables or disables every toolbar item. `clear_repo_overrides` clears a repo's branch, workflow and tag overrides in one step.

A failed command rejects with `{ kind, message }`. `kind` is one of `unauthorized`, `forbidden`, `not_found`, `rate_limited`, `network`, `server`, `redirected`, `decode`, `keychain`, `read_only`, `invalid_input`, `unknown_account` or `internal`; `message` is the display text and never contains a token. Expected failures are logged at debug level; `decode`, `keychain` and `internal` faults at warn. Typed wrappers for the frontend are in `src/lib/tauri.ts`, and `friendlyError` (section 8) turns a rejection into a sentence by its kind.

Events:

| Event | Direction | Payload |
| --- | --- | --- |
| `snapshot-updated` | Rust to every window | the full `Snapshot` |
| `repo-list-progress` | Rust to the webview | account ID and repos loaded so far, once per fetched page |
| `settings-open` | Rust to the Settings window | `pane`, `sheet` and `account_id`, each possibly absent |
| `settings-pane` | native toolbar to the Settings page | the clicked pane ID |
| `update-progress` | Rust to every window | `{ downloaded, total }` in bytes while an update downloads; `total` is `null` when unknown |
| `update-check-result` | Rust to the popup | `{ update, error }` after Check for Updates in the tray menu: the update found or `null`, and the failure text or `null` |

Every config-changing command ends with `Runtime::restart` (section 5), so the backend re-reads the config and the new snapshot reaches the windows through `snapshot-updated`.

### Capabilities

`src-tauri/capabilities/` grants plugin and core permissions per window:

| File | Window | Permissions |
| --- | --- | --- |
| `popup.json` | `popup` | event listen and unlisten, menu creation and popup (native context menus), resource close, `core:window:allow-set-size` (content-fitted height), clipboard write |
| `settings.json` | `settings` | `core:app:allow-version` (the version row), event listen and unlisten, `core:window:allow-set-title` |

The app's own commands have no per-command permission entries, so both windows can invoke them. Neither window has filesystem, shell, HTTP, updater or process permissions; updates go through the app's own commands.

## 3. Domain model

Files: `model.rs`, `aggregate.rs`, `filters.rs`, `notes.rs`.

A provider turns each workflow run or pipeline into a `Run`: ID, attempt (GitHub `run_attempt`, always 1 on GitLab), normalized `RunState`, branch, group key, group name, display name, URL, `updated_at`, and the flags `pull_request`, `fork` and `tag`. The group key is the workflow ID on GitHub and the pipeline `source` on GitLab. The group name (`group_name`) is the workflow name on GitHub (the run name when the workflow list does not know the workflow, filled in from the cached workflow list after the runs are fetched) and the pipeline name, or its `source` when unnamed, on GitLab. The `fork` flag is set when a GitHub run's head repository differs from the watched repo or is gone. GitLab sets `pull_request` for the `merge_request_event` source. Both providers set `tag` from the repository's tag list (see Tag detection) and a run whose branch is a tag name carries that tag name as its `branch`. Selection skips any run with `fork` set, and any run with `tag` set unless tag runs are included.

### Run states

`RunState`: Failed, Running, Queued, Success, Canceled, Neutral. Rank, lower is worse: Failed 0, Running and Queued 1, Success 2, Canceled and Neutral 3. Canceled and Neutral are never selectable, so they cannot represent a group.

### Repo status

`RepoStatus`, worst first: Failed, Error, Running, Success, None.

| Run state | Repo status |
| --- | --- |
| Failed | Failed |
| Running, Queued | Running |
| Success | Success |
| Canceled, Neutral | None |

Error is set by the poller, never derived from a run. A repo with no selectable group has status None.

### Selecting groups

`select_groups` drops runs that are forks, tag runs when tag runs are off (`include_tags`), not selectable, outside the branch filter, in an ignored workflow, not in the active-group set (GitHub workflows that are still active), or pull request runs when "ignore pull request runs" is on. Of the rest it keeps one run per `(branch, group)`: the one with the highest run ID. Groups on a branch other than the default branch stop counting when their run was last updated more than 30 days ago (`STALE_BRANCH_CUTOFF`).

When tag runs are included, a tag run bypasses only the branch filter; the fork, pull request, ignored-workflow, selectable and active-group rules still apply. The grouping key uses the placeholder branch `refs/tags/*` (`TAG_GROUP_BRANCH`) for every tag run, so each workflow keeps one entry, the run with the highest ID, and that run keeps its real tag name. Tag entries are exempt from the 30-day cutoff. A failure on one tag followed by a success on a later tag is a recovery, because both runs share one group.

### Tag detection

Both providers detect tag runs the same way: a run is a tag run when its ref is in the repository's tag list. The list comes from `GET /repositories/{id}/tags` on GitHub (Metadata read is enough, so existing tokens work) and `GET /projects/{id}/repository/tags` on GitLab. It is cached per repo in `TagCache` with its ETag (kept only when the list fits one page; a 304 keeps the cached list). The list is fetched only when tag runs are on and a run's ref is neither the default branch, a known tag nor a ref already checked and found not to be a tag (`not_tags`; a refresh drops the names that turn out to be tags and keeps the rest). The list is read up to 10 pages (`MAX_TAG_PAGES`); names beyond that count as branches. A 403 or 404 on the tag list is treated as no tags. GitLab's pipeline list has no tag field, so GitLab relies on this list alone. With tag runs off, tag runs are dropped on both providers.

`repo_state` picks the representative run: the chosen run of the worst group, with ties going to the most recently updated. The repo status follows that run. All chosen runs are kept in `groups` for the expanded popup row.

### Filter resolution

Branch patterns, ignored workflows and tag runs resolve in the order repo exception, organization, global settings. `None`/`null` inherits the next level; `Some([])` is an explicit "nothing" (default branch only, or no ignored workflows) at organization and repo level. `include_tags` is `Option<bool>` at both levels. `EffectiveFilters::resolve(settings, org, repo)` in `filters.rs` is the only place the levels are combined; selection, the fetch, restart adoption (`selection_changed`) and the notification rebaseline all go through it.

An organization is keyed by `(host, owner)` (`OrgKey` in `config.rs`):

- `owner` is the first path segment of the repo's full name: the GitHub owner, or the GitLab top-level group (`acme` for `acme/web/app`). Personal namespaces count as organizations.
- `host` is `github.com` for GitHub accounts. For GitLab it is the account base URL's host, lowercase, plus `:port` when the port is not the scheme's default.
- Hosts and owners compare case-insensitively. The owner is stored and shown as the repo's full name spells it.

Every account that reaches the same host and owner shares that organization's overrides. A repo that moves to another owner resolves to the new owner's organization once the repo list refresh (section 5) saves its new name; repo exceptions are keyed by repo ID and stay with it.

`set_org_filters(host, owner, filters)` writes the three organization fields in one save. It cleans and checks the pattern lists like the other filter commands, refuses a read-only config, and rejects an organization with no watched repo and no saved entry as `invalid_input` ("No watched repositories in that organization."). `get_settings` returns `organizations`, one entry per organization with at least one watched repo (`host`, `owner`, `filters` with `null` for each inherited value, `repo_count`), sorted by host, then owner, case-insensitively with numbers compared by value; each entry of `repos` carries its `host` and `owner`.

### Branch filters

`BranchFilter` is either the repo's default branch or a list of glob patterns (globset, with `/` as a literal separator, so `release/*` does not match `release/1.0/hotfix`). The patterns come from the repo's own list, else its organization's, else the global list; an empty list at repo or organization level is an override that means the default branch. A pattern that does not compile puts the repo in Error with a note naming the pattern.

### Workflow filters

`WorkflowFilter` holds case-insensitive glob patterns matched against `Run.group_name`; no patterns ignores nothing. The list comes from the repo's own list, else its organization's, else the global list; an empty list at repo or organization level is an override that ignores nothing. For example, with `Dependabot*` ignored globally, an organization set to ignore nothing shows its Dependabot runs, and one of its repos with its own `["Dependabot*"]` ignores them again. The tag choice (`include_tags`) resolves the same way. Ignored runs are dropped in `select_groups`, so they do not affect the repo status, the tray badge or notifications. A pattern that does not compile puts the repo in Error with the note "Workflow filter isn't valid: …".

### Repo state flags

`RepoState` carries the status, representative run, groups, `last_checked`, a `stale` flag and a `note`. `stale` marks a state that was not refreshed by the latest poll: network failure below the error threshold, a 401 or rate limit, a paused pool, or the global pause. `notes.rs` holds the note strings shown for errors, including the invalid branch and workflow filter notes, and the CI-disabled case.

### Worst-state aggregation

`aggregate::overall` counts repos per status and keeps the worst. The tray color is gray when paused or when nothing is watched; otherwise Failed is red, Error orange, Running yellow, Success green, None gray. The tooltip lists non-zero counts worst first (for example `1 failed, 3 passing`), or reads `Paused` or `No watched repositories`.

## 4. Providers

Files: `providers/mod.rs` (the `Provider` trait), `providers/github.rs`, `providers/gitlab.rs`, `http.rs`.

The trait has `validate`, `list_repos`, `is_member` and `fetch`. Each provider keeps a per-repo `RepoCache` with ETags, the last parsed runs and the active-workflow set.

### HTTP client (`http.rs`)

One client per account. Requests send `Authorization: Bearer <token>` (marked sensitive), `User-Agent: vigia/<version>` and `Accept: application/json` unless the provider sets another. The total timeout is 30 s and the connect timeout 10 s. Redirects are never followed: any 3xx becomes a `Redirected` error. Logs record the URL without its query string. A response body is read up to 10 MB (`MAX_BODY_BYTES`); a larger declared or streamed body fails as `Decode`.

Pagination follows the `Link: rel="next"` header, accepted only when the next URL has the same scheme, host and port as the base URL; otherwise the call fails as `Redirected`.

Rate limit headers: `x-ratelimit-limit`, `-remaining` and `-reset` (GitHub), or `RateLimit-Limit`, `-Remaining` and `-Reset` (GitLab).

| Response | Error |
| --- | --- |
| 200, 304 | success |
| 3xx | `Redirected` |
| 401 | `Unauthorized` |
| 403 or 429 with 429, a `Retry-After` header, zero remaining requests, or a body mentioning "secondary rate limit" | `RateLimited` (carries retry-after, reset time, remaining) |
| other 403 | `Forbidden` |
| 404 | `NotFound` |
| 5xx | `Server` |
| other | `Decode` ("unexpected status") |
| connection or timeout failure | `Network` |

### GitHub

Base URL `https://api.github.com`, with `Accept: application/vnd.github+json` and `x-github-api-version: 2026-03-10`.

| Purpose | Request |
| --- | --- |
| Validate | `GET /user` |
| List repos | `GET /user/repos?per_page=100&affiliation=owner,collaborator,organization_member`, archived repos dropped |
| Active workflows | `GET /repositories/{id}/actions/workflows?per_page=100`, keeping `state == active` |
| Runs | `GET /repositories/{id}/actions/runs?per_page=100&exclude_pull_requests=true`, plus `branch=<default>` when the filter is the default branch and tag runs are off |

Token: a fine-grained personal access token (prefix `github_pat_`) with read-only Actions and Metadata permissions. Settings rejects other tokens when an account is added or a token replaced. `open_github_token_page` opens GitHub's token form prefilled with the name, a 90-day expiry, the optional owner, and `actions=read` and `metadata=read`; the owner must be a valid login (1 to 39 alphanumerics or single hyphens).

The runs request sends the cached ETag and a 304 reuses the cached runs. The workflow list is reused for 6 hours (`WORKFLOWS_TTL`). It is fetched again on Refresh now and when a run belongs to a workflow outside the cached set, once per unknown workflow ID so a disabled workflow does not loop. Its ETag is sent and kept only when the list fits one page. Requests that return 304 on the runs endpoint do not count toward the pool's observed usage.

With tag runs on, the runs are fetched without the branch restriction (100 per page) and filtered locally. The tag list costs one request on the first poll and after a run appears on an unchecked ref; later polls send its ETag.

Status mapping (`RunState::from_github`):

| `status` | `conclusion` | State |
| --- | --- | --- |
| queued, requested, pending | any | Queued |
| waiting | any | Neutral |
| in_progress | any | Running |
| completed | success | Success |
| completed | failure, timed_out, startup_failure | Failed |
| completed | cancelled | Canceled |
| completed | skipped, neutral, stale, action_required, absent | Neutral |
| other value | | Neutral, logged once per launch |

### GitLab

Base URL is the account's normalized URL (`normalize_base_url`): `https` or `http` scheme, lowercase host, no user info, query or fragment, trailing slashes stripped, a path prefix kept. The API root is `<base>/api/v4`.

| Purpose | Request |
| --- | --- |
| Validate | `GET /user` |
| List repos | `GET /projects?membership=true&archived=false&simple=true&per_page=100&pagination=keyset&order_by=id&sort=asc`. If the first page lacks `default_branch` or `web_url`, the list is fetched again without `simple`. |
| Pipelines | `GET /projects/{id}/pipelines?order_by=id&sort=desc&per_page=100`, plus `ref=<default>` when the filter is the default branch and tag runs are off |
| Membership check | `GET /projects/{id}`; the account is a member when `permissions.project_access` or `permissions.group_access` is set |

Token: a personal access token with the `read_api` scope, per the Add Account sheet. An `http://` base URL needs an explicit confirmation in the sheet because the token travels unencrypted.

The pipelines request sends the cached ETag; a 304 reuses cached runs. With tag runs on, pipelines are fetched without the `ref` restriction and the tag list (`GET /projects/{id}/repository/tags`) is cached with its ETag as on GitHub. GitLab's per-minute `RateLimit-*` values are multiplied by 60 to match the pool's per-hour budget. Every GitLab request counts toward the pool. A project membership answer is reused for 6 hours (`MEMBERSHIP_TTL`).

Status mapping (`RunState::from_gitlab`):

| Pipeline `status` | State |
| --- | --- |
| created, waiting_for_resource, waiting_for_callback, preparing, pending, scheduled | Queued |
| running | Running |
| success | Success |
| failed | Failed |
| canceled, canceling | Canceled |
| skipped, manual | Neutral |
| other value | Neutral, logged once per launch |

A pipeline's display name is its `name`, or its `source` when no name is set.

## 5. Polling

Files: `poller.rs` (per-account cycle, scheduling), `pool.rs` (rate budget), `app.rs` (task management, wake detection, refresh, pause, publishing).

### Tasks and pools

`Runtime::restart` runs at launch and after any config change. It starts one task per configured account. An account whose token is missing or unreadable is shown as an auth error (`auth_reason` `missing_token`, or `keychain` while the Keychain is blocked) with every repo in Error and gets no task. A 401 from the provider sets `auth_reason` `rejected`. A restart bumps a generation counter, and a worker from an older generation neither publishes nor notifies.

An account whose kind, base URL and token are unchanged keeps its poll state. The token is compared by hash, so it is never stored for the comparison. The new worker adopts the old one's repo states, caches, schedule and failure counts (`AccountWorker::adopt`), together with its reachability, backoff and rate limit state unless the token was rejected. A worker parked between cycles is taken over at once. A worker in the middle of a cycle finishes it and hands itself to its successor; the successor shows the last published states meanwhile. Only repos whose selection changed drop their cached runs and become due at once: a change of effective branch patterns, ignored workflows or tag choice (from the repo, organization or global setting), pull request exclusion or default branch (`selection_changed` in `filters.rs`). Changing an organization's filters therefore refetches only that organization's repos whose effective values changed, across every account that watches them, and `Notifier::apply_config` resets the baseline of the same repos. Workflow and tag lists stay cached. A changed poll interval brings later due times forward. A replaced token starts fresh.

Rate pools (`pool.rs`) are keyed by `github:<user ID>` and `gitlab:<account ID>`, so all GitHub accounts of one user share a budget and each GitLab account has its own. Pools survive restarts and drop the repo counts and fast-poll slots of accounts that no longer exist. A pool starts with an assumed hourly limit (5000 GitHub, 2000 GitLab) and replaces it with the limit the server reports.

### Locks

The runtime's config store and the `Notifier` are reached only through `with_config`, `with_config_mut`, `update_config` and `with_notifier`. Locks are taken in this order and only while holding the ones before them: `tasks`, `publish_state`, `config`, `applied_config`, `notifier`, `pools`, one pool, a parked worker slot, then the snapshot store's maps. The publisher and the notification sink run with no lock held.

### Intervals

| Setting or constant | Value |
| --- | --- |
| Default interval | 60 s |
| Allowed range | 15 s to 3600 s; the config value is clamped on save and on use |
| Interval choices in General | 15 s, 30 s, 1, 2, 5, 10 min (plus a saved value outside the list) |
| Fast polling interval | 15 s (`FAST_POLL_SECS`) |
| Jitter | up to 10 percent either way, at least 1 s |
| Due-time rounding | up to the next 5 s of Unix time (`DUE_BUCKET_SECS`), so repos that fall due together share one wakeup |
| Backoff cap | 300 s |
| `Retry-After` | clamped to 1 s to 3600 s |
| Rate limit reset time | at most 1 hour ahead |
| Concurrent requests per account | 6 |

### Budget and stretching

Every request is classed (`RequestClass`):

| Class | Requests | Effect on the base interval |
| --- | --- | --- |
| Base | steady polling at the base interval | stretches it |
| Fast | polling of a repo with a moving run | none; bounded by the fast-poll slots |
| Fill | filling an empty cache: a repo's first fetch, workflow lists, tag lists and repo lists | none |

Only Base requests stretch the interval. The base interval is stretched so projected usage stays within 40 percent of the hourly limit (`BASE_SHARE`). GitHub projects from the counted Base requests observed in the last hour, so quiet repos answered by 304 cost nothing. GitLab projects `repos x 3600 / interval`, and only after the server has sent `RateLimit` headers: an instance without them may have no limit, so its interval is never stretched from the assumed one (a 429 still pauses the pool). When projected usage exceeds the budget the interval becomes `configured x projected / budget`, rounded up and capped at 3600 s. The popup footer shows the stretched interval; the snapshot carries both the configured and the effective interval per account.

When a response reports fewer remaining requests than 15 percent of the limit (`PAUSE_SHARE`), the pool pauses until the reported reset time, at most an hour ahead.

### Fast polling

A repo polls every 15 s while one of its chosen runs is Running or Queued and was updated within the last hour. Fast slots are capped per pool at `floor(0.3 x limit x 15 / 3600)`, at most 10 repos: 6 for a 5000 limit, 2 for a 2000 limit. Candidates are ranked by the most recent run update. Fast polling is skipped for a repo whose last poll failed and for every repo of an account that is unreachable or backing off.

### Cycle

The branch and workflow filters are compiled once when a worker is built (`FilterCompiler`); repos of one organization that inherit its patterns (its own, or the global ones) share one compiled set, and the resolved tag choice is stored with them. Each worker holds the settings and the organization overrides it was built from. `AccountWorker::cycle` polls the repos that are due, or all repos when forced, with at most 6 requests in flight. After each cycle the next due time is computed per polled repo and the task sleeps until the earliest due time, at least 1 s, or until woken.

A cycle stops sending requests when any of these holds:

- the account has an auth error;
- the pool is paused by a rate limit;
- the global pause flag is set;
- any response in the cycle was a 401 or a rate limit.

Repos that were skipped keep their state and stay due.

### Error handling

| Outcome | Repo result | Account effect |
| --- | --- | --- |
| 401 | Error, "Token rejected" on every repo | Auth error; the account stops polling until a restart |
| Rate limited | previous state, stale | Pool paused until the wait ends |
| 404, 403 | Error, "Not found, or the token can't access it" | none |
| GitLab 403 while still a member | status None, note "CI/CD is turned off" | none |
| Redirect | Error, "The server redirected Vigia. Check the base URL in Settings." | none |
| Unreadable response | Error, "The server sent a response Vigia can't read" | none |
| Network error or 5xx | previous state, stale; Error ("Can't reach the server") after 3 failures in a row | counted toward unreachable |
| Invalid branch filter | Error with the pattern named | none |
| Invalid workflow filter | Error, "Workflow filter isn't valid: …" | none |

Rate limit wait: `Retry-After` when present, clamped to 1 to 3600 s; the reset time when the primary window is exhausted, kept between now and an hour ahead; otherwise 60 s, doubling on each repeat up to 300 s.

Network backoff: when every repo polled in a cycle failed, the account is unreachable and its interval becomes the larger of the base interval and a backoff that starts at the configured interval and doubles to at most 300 s. A repo's own interval after repeated failures is `base x 2^(failures-1)`, capped at 300 s and never below base. A cycle that polled nothing leaves reachability unchanged.

### Wake, pause, refresh

A heartbeat ticks every 5 s. A wall-clock gap above 35 s counts as a wake from sleep: every account worker polls all its repos at once, resets its backoff and failure counts, and for 30 s network failures do not count toward the Error threshold.

Pause (`set_paused`, from the popup button or the tray) stops all polling. The snapshot marks every repo stale and the tray shows the Paused look. Resuming polls every repo at once.

Refresh now (popup button, Cmd+R, tray menu) is ignored while paused. Otherwise it forces a full poll and refreshes GitHub workflow lists.

At launch, each account's first cycle polls its repos before anything else. Afterwards the account refreshes its repo list so renamed repos and changed default branches are saved to the config, and hands the list to the Repositories list cache, which the Repositories pane reads without another fetch. A repo whose selection changes with the list, because it moved to an organization with other filters or its default branch changed, is recompiled, drops its cached runs, starts a new notification baseline and is polled at once (`AccountWorker::apply_repo_list`). The new names are also written into the last applied config, so the next restart compares against them. Later restarts do not refresh the list again.

### Snapshot and publishing

`SnapshotStore` holds account and repo states. A `Snapshot` has the generation time, `paused`, `secrets_blocked`, `config_read_only`, `config_error`, `update` (`{ version, notes }` of a newer release found by an update check, or `null`), the tray color and tooltip, accounts (auth error, `auth_reason` (`rejected`, `missing_token`, `keychain` or `null`), unreachable, rate-limited-until, configured and effective interval, Keychain denial, error text) and repos sorted by name. After every cycle the runtime stores the results of the repos the cycle touched and asks to publish.

A publish updates the tray and emits `snapshot-updated`. It happens only when the stored states, the pause flag, the app flags or the found update changed; the generation time alone is not a change. Publishes requested by workers are at most 1 s apart across all accounts (`PUBLISH_INTERVAL`); a change inside that window is published when it ends. Restarts and pause toggles publish at once.

### Updates

`updates.rs` checks for a newer release through `tauri-plugin-updater`. The manifest is `https://github.com/s3rgiosan/vigia/releases/latest/download/latest.json` (`plugins.updater.endpoints`), and every archive must carry a minisign signature that verifies against `plugins.updater.pubkey`. While `check_for_updates` is on, a scheduled task checks 60 s after launch and then every 24 h, reading the setting before each check; a failed scheduled check is logged at debug. A successful check, scheduled or not, stores the update it found (or none) and puts it on the snapshot as `update`; a failed one keeps the update found before.

`check_for_updates_now` returns `{ version, notes }` or `null`, rejecting as `network` when the server cannot be reached and `internal` otherwise. `install_update` downloads the pending update, emitting `update-progress`, installs it over the running app and relaunches; it rejects as `invalid_input` when no update is pending or one is already installing. Check for Updates in the tray menu shows the popup and sends it `update-check-result`. The checker sits behind the `UpdateSource` and `PendingUpdate` traits, so tests drive the schedule and the install with fakes and paused tokio time.

## 6. Notifications

Files: `notify.rs` (rules), `notification_center.rs` (delivery), `app.rs` (`apply_cycle`), `lib.rs` (click handling).

### Transitions

The `Notifier` remembers, per watched repo and per `(branch, group)`, whether the group is failed and which run attempt it last notified about. After each cycle it compares each repo's chosen runs with that memory:

- A group whose run is Failed produces a failure transition unless that attempt was already notified.
- A group whose run is Success after being failed produces a recovery transition.
- Other states produce nothing.

The settings "notify when a branch fails" and "notify when it recovers" filter the transitions afterwards, so memory stays current while a kind is switched off. The General pane disables the recovery switch while failure notifications are off.

### Identity

An attempt's identity is `<run ID>#<attempt>` on GitHub, where a re-run keeps the ID and raises the attempt, and `<pipeline ID>@<updated_at unix time>` on GitLab, where a retry keeps the ID and changes `updated_at`.

### Baselines

A repo's first successful state, and the first successful state after an Error, set a baseline and produce no notification. A group that first appears in a repo that already has a baseline notifies normally. Baselines reset:

- at launch;
- for a newly watched repo;
- for a repo whose selection changed: its effective branch patterns, ignored-workflow list or tag setting (repo override, account override or global value), or the "ignore pull request runs" setting, which affects every repo (`repos_to_rebaseline`, using `selection_changed`).

Removing a repo or account drops its memory. After each successful state, groups that left the repo's selection, such as merged feature branches, are forgotten. A repo whose last poll was an Error does not update its group memory, so the status after recovery is observed silently.

### Coalescing

More than 3 transitions in one cycle become one summary notification per account, titled with the counts of repositories that failed and recovered (a repo with several failing groups counts once) and the body "Open Vigia for the list." With 3 or fewer, each transition gets its own notification: the title is the repo name and the verb, the body is the branch and workflow name.

### Auth notification

When an account's token is rejected, one notification (`<label>: token rejected`) is sent. The account is not notified again until its error clears. Replacing the token forgets the memory so a later rejection notifies again. An account that has no stored token or an unreadable Keychain shows the error state without a notification.

### Click behavior

| Target | Used for | Click action |
| --- | --- | --- |
| Run | a single transition | opens the run URL in the default browser after the URL guard (section 7); shows the popup when the account is gone or the URL is rejected |
| Popup | summaries and account problems | shows the popup |

Linux and Windows deliver through `tauri-plugin-notification`, which reports no clicks. Delivery on macOS uses `mac-notification-sys`. Each notification waits for its click on its own thread. At most 8 notifications wait at once; past that a notification is still shown but a click only brings the app forward. A notification that is dismissed or removed from Notification Center never reports a click. Debug builds attribute notifications to Terminal because they have no registered bundle. The app asks for notification permission after an account is added and when a notification switch is turned on, but only while macOS has not yet recorded an answer (`notifications::ensure_permission`).

## 7. Storage and security

Files: `config.rs`, `secrets.rs`, `links.rs`, `tauri.conf.json`.

### Config file

`config.json` lives in the app config directory (named after the bundle identifier). It holds `schema_version` (currently 1), accounts without tokens (ID, kind, label, GitLab base URL, provider user ID and login), watched repos (account ID, repo info, optional branch patterns, optional ignored-workflow patterns, optional `include_tags` override), `organizations` (each with `host`, `owner` and optional `branch_patterns`, `ignored_workflows` and `include_tags`; the list is omitted when empty, and an entry with no override is dropped on every save) and settings (interval, exclude pull requests, global branch patterns, global ignored-workflow patterns, global `include_tags` (default off), the two notification switches, launch at login, `check_for_updates` (default on, also for files saved before the setting existed)).

Settings are validated when saved: the poll interval is clamped to 15 to 3600 s, and a pattern list holds at most 50 patterns of at most 200 characters (`MAX_PATTERNS`, `MAX_PATTERN_CHARS`); longer input is rejected as `invalid_input`.

Writes are atomic: a temporary file in the same directory is written, synced and renamed over the target. A failed write leaves the in-memory config unchanged.

The config is read-only, and saves are refused, in these cases:

- the file's `schema_version` is newer than the build's; it is left untouched, and the popup and Settings show a banner;
- the file cannot be read for an I/O reason, or an unparsable file cannot be moved aside; the snapshot carries `config_error`.

The ignored-workflow, tag and organization fields default to empty, absent or off, so existing files load unchanged and `schema_version` stays 1.

A file that does not parse is renamed to `config.json.corrupt-<unix time>` and the app starts with defaults. Older schema versions pass through `migrate` on load.

Debug builds use `config.dev.json` and keep tokens in `tokens.dev.json` (a plain file with owner-only permissions) so a rebuilt, re-signed app does not hit Keychain denials and never touches a release install. `VIGIA_DEBUG_SETUP` (JSON of accounts and repos, with tokens read from named environment variables) adds accounts at launch; `VIGIA_DEBUG_SHOW_POPUP` and `VIGIA_DEBUG_OPEN_SETTINGS` open a window at launch. These exist only in debug builds.

### Credential store

The startup log records only the load state (`loaded`, `not found`, or `failed` with a reason). A JSON parse failure is described by its category and position, never by quoting the stored content.

Tokens live in one item of the OS credential store, through the `keyring` crate: the Keychain on macOS, Credential Manager on Windows and the Secret Service on Linux. Service is the bundle identifier, account is `tokens`, and the value is a JSON map from account ID to token. The map is loaded once and kept in memory. User-facing text names the store as the OS does (`SECRET_STORE` in `src/lib/platform.ts`, `notes::KEYCHAIN_DENIED`); the examples below use the macOS names.

A failed load (access denied or unparsable JSON) puts the store in a blocked state: every token write fails, the snapshot has `secrets_blocked`, accounts show "Vigia can't read the Keychain", and both windows show a banner. "Try Again" (`retry_secrets`) reloads and restarts polling when the load works. "Reset Keychain Item…" (`reset_secrets`) overwrites the item with an empty map after a confirmation sheet, discarding every token, and clears the blocked state.

At launch, tokens whose account is not in the config are pruned, but only when the config came from an existing file, is writable, and the Keychain is not blocked. Deleting an account removes its token on a best-effort basis.

`secrets::MemoryStore`, an in-memory store with a failing variant, is compiled for unit tests and for integration tests through the `test-support` Cargo feature; release builds do not include it.

### What stays in the app

- Tokens are sent only as a bearer header to the account's own API host. They are never serialized to the webview; `SettingsView` has no token field and `NewAccount` redacts the token in debug output.
- The config file contains no tokens.
- The only network clients are the Rust HTTP client, which talks to GitHub and the configured GitLab hosts, and the updater, which fetches `latest.json` and the update archive from the GitHub release assets on github.com and its release download host. The webview has no network access beyond the Tauri IPC.
- Logs drop query strings from URLs and mark the authorization header sensitive.
- The app has no telemetry.

### Stop Watching and restore

`unwatch_repo` removes the repo from the config and keeps the removed entry, overrides included, in Rust memory (`UnwatchedRepos`) for 10 minutes (`RESTORE_TTL`). It returns a token, `{ account_id, repo_id }`. `restore_watched_repo` takes that token, never an entry, and watches the repo again with its overrides when the token is known and unexpired; otherwise it reports that nothing was restored. An entry whose restore could not be saved goes back with a fresh expiry.

### Content security policy

`default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src ipc: http://ipc.localhost`. Scripts and connections are limited to the bundled app and Tauri IPC.

### URL guard

URLs that come from API responses reach the default browser only through `links::check_url`. It accepts a URL when its scheme, host and port equal the account's web origin: `https://github.com` for GitHub, the account's base URL for GitLab. The `open_url` command and notification clicks both use it; a rejected URL is not opened. GitHub's token page opens from a URL built in Rust from a fixed origin.

## 8. UI

Files: `src/popup/`, `src/settings/`, `src/components/`, `src/lib/`.

### Frontend structure

`App.tsx` loads the popup and Settings views lazily and wraps each in an `ErrorBoundary`. A render error in a view shows "Something went wrong in this view." with a **Reload** button that remounts the view. The control gallery loads only on the dev server.

Payloads from the backend pass runtime shape checks (`lib/guards.ts`: `isSnapshot`, `isSettingsView`) before they reach state. A rejected command arrives as `{ kind, message }` and `friendlyError` (`lib/errors.ts`) chooses the sentence by `kind`: for example `unauthorized` reads "The server rejected this token. Check it's copied in full and hasn't expired.", `keychain` reads "Vigia can’t read its tokens from the Keychain.", and `invalid_input` shows the backend message. An `internal` error, or a plain string, shows a shortened copy of its text after "Something went wrong:".

`Icon` draws each exported icon (SF Symbols on macOS, Lucide elsewhere) as a CSS mask through the class `symbol symbol--<name>` (`src/assets/symbols/symbols.css`).

The build target is `safari13`, the oldest WebKit the app supports (`vite.config.ts`). Opening links goes through the Rust `open_url` command; the frontend has no opener plugin.

### Popup

Structure, top to bottom:

- Header: a status dot, a headline with the most urgent count (`2 failed`, `All passing`, `Paused`, `No repositories`) and the remaining counts as detail, and three icon buttons for Refresh, Pause or Resume, and Settings.
- Filter field: matches repo name, branch and workflow name, case-insensitively. While a filter is set every section is expanded.
- Banners: transient errors (6 s), the Undo banner after Stop Watching (8 s), and persistent banners (`BannerView`):

| Condition | Text | Action |
| --- | --- | --- |
| Keychain blocked | "Vigia can’t read its tokens from the Keychain." | **Open Settings** |
| Config unreadable | "Vigia couldn’t read its settings file, so changes won’t be saved." | none |
| Config from a newer version | "Settings were saved by a newer version of Vigia. Update Vigia to change them." | none |
| Token rejected | "The token for “<account>” was rejected." | **Replace Token…**, which opens Settings on the Replace Token sheet for that account |
| No token saved | "No token is saved for “<account>”." | **Replace Token…** |
| Account token unreadable | "Vigia can’t read the token for “<account>” from the Keychain." | **Open Settings** |

The account banners and the Settings status text follow the snapshot's `auth_reason` (`lib/auth.ts`); a missing or unknown reason reads as a rejected token.
| Account unreachable | "<account> is unreachable. Showing the last known state." | none |
| Rate-limit pause | "<account>: paused until <time> to stay within the GitHub API limit." (GitLab for a GitLab account) | none |
- Sections: Failed, Errors and Running start expanded; Passing and No runs start collapsed. Within a section, repos are grouped by account (when there is more than one) and then by owner or namespace.
- Footer: `Updated <relative time>`, a `Paused` prefix, and the stretched interval when the pool slows polling.

A repo row shows the status dot, the repo name, the workflow and branch (or the note), the relative time of the run, and, when the state was not refreshed, a warning symbol before the time of the last check (with the hidden text "Stale"), and an open-in-browser icon that appears on hover and keyboard focus (its space is always reserved). Repo rows are 28 px high with 13 px names; run rows are 24 px high; secondary text is 12 px. A repo with several groups, or a GitHub access error that may come from SAML enforcement, expands to list its runs worst first, then newest first. Clicking a row opens the run (or the repo's Actions or Pipelines page) in the browser. Relative times refresh every 30 s while the popup is visible. Within a section, repos sort by name with numeric-aware comparison, so `app-2` comes before `app-10`. The popup is split into `Popup`, `SectionView`, `RepoRow`, `RunRow` and `BannerView`, with keyboard and window-fit logic in hooks; `RepoRow` and `RunRow` are memoized.

Right-click menu (native menu): Open Run, Open Actions or Open Pipelines, Open Repository, Copy Run Link or Copy Repository Link (Cmd+C), Stop Watching (Cmd+Backspace).

| Keys | Action |
| --- | --- |
| Cmd+R | Refresh now |
| Cmd+, | Open Settings |
| Cmd+F | Focus the filter |
| Esc | Clear the filter, or hide the popup when it is empty |
| Typing | Focuses the filter and starts filtering |
| Up and Down | Move between rows; Up from the first row returns to the filter |
| Right and Left | Expand and collapse a row or section; Left on a run row focuses its repository row |
| Menu key or Shift+F10 | Open the row's context menu |
| Cmd+C, Cmd+Backspace | Copy link, stop watching the focused row |

### Sizing

The popup is 380 pt wide. The page measures its content and calls `setSize` so the window height follows it between 200 and 520 pt. With a list, the height is the window's current height with the list's visible area replaced by its full content height. A `ResizeObserver` and every render trigger the measurement. The list scrolls inside the panel at the maximum height.

### Settings

Four panes, switched from the native toolbar (the page's own toolbar off macOS and in the browser preview), with Cmd+1 to Cmd+4, or by `settings-open` events. The window title names the current pane. The last pane is remembered through `useSafeStorage`, which uses `localStorage` and falls back to memory when storage is missing or throws.

| Pane | Contents |
| --- | --- |
| Accounts | Source list of accounts with add and remove buttons and a More Actions (…) menu holding **Remove All Accounts…** (enabled with two or more accounts), a detail form (name, signed-in login, server, connection status), and Replace Token |
| Repositories | Per account picker: search, org-level toggles, checkbox rows, reload. The list is virtualized (26 px rows, 8 rows of overscan) and exposes the full count through `aria-setsize` and `aria-posinset`. Keyboard: arrows, Home, End, Page Up, Page Down, Space and Enter (`useRovingListbox`). Repos sort by name with numeric-aware comparison. A repo already watched through another account of the same instance is shown as unavailable. |
| Filters | Master–detail like Accounts. The sidebar lists All Repositories, then organizations grouped under host headers (sorted by host and owner, numeric-aware), each marked Custom when it overrides a filter; the selection is remembered (`vigia.settings.filters.scope`). All Repositories shows the global Branches, Ignored workflows and Tag runs rows and the Known limits list. An organization shows its name, host and watched-repository count, a Filters group (Branches and Ignored workflows with a Default/Custom menu and a field whose Default placeholder is the global value; Tag runs Default (On or Off)/On/Off), saved through `set_org_filters`, and a Repository Exceptions group: one disclosure row per repo of that organization with an override, summarised in one line, expanding in place to the same three rows (Default shows the organization-resolved value), with + (Add Exceptions sheet listing that organization's repos without an exception, sized to the longest name between 440 px and the window width less 48 px) and − (`clear_repo_overrides`). |
| General | Check interval, ignore pull request runs, notify on failure, notify on recovery, open at login. Every control saves on change. |

When the config is read-only, every control is disabled.

Sheets (`components/Sheet.tsx`) are modal dialogs: Add Account (provider selector, GitLab server URL, token, name, and a confirmation checkbox for `http://`), Replace Token, Remove Account, Remove All Accounts, Add Exceptions, Reset Keychain Item. A sheet traps Tab focus, makes the rest of the page `inert`, restores focus to the previous element when it closes, and cancels on Esc. A destructive sheet (Remove Account, Remove All Accounts, Reset Keychain Item) is an `alertdialog` described by its message; its Cancel is the default button with initial focus, so Return does not confirm. While any sheet is open the page calls `set_settings_toolbar_enabled(false)` and ignores pane switches (toolbar clicks, Cmd+1 to Cmd+4) and `settings-open` requests, so typed input is kept; closing the last sheet enables the toolbar again.

### Status pattern

State is never color alone. Every status dot is `aria-hidden` and is paired with text: a visually hidden label before the row text (`Failed: `, `Passing: `, `No result: `), a `title` tooltip, and visible text for connection status in Settings. The dot color is red, orange, yellow, green or gray, matching the tray; running states pulse.

### Accessibility

- Rows, section headers and buttons are keyboard focusable with `:focus-visible` rings; disclosure buttons carry `aria-expanded` and labels.
- Live regions: the popup headline and footer use `aria-live="polite"`; error banners use `role="alert"`.
- The refresh button exposes `aria-busy`.
- The repository picker is a multi-select listbox with `aria-activedescendant`.
- CSS honors `prefers-reduced-motion`, `prefers-contrast: more`, `prefers-reduced-transparency`, and light and dark color schemes.

## 9. Build, test and release

### Commands

| Command | Purpose |
| --- | --- |
| `npm ci` | Install JavaScript dependencies |
| `npm run tauri dev` | Run the app in development (Vite on port 1420) |
| `npm run dev` | Vite alone: with no Tauri runtime, a fake backend serves the windows at `?view=popup` and `?view=settings` (`&scenario=empty` for the empty state), and `?view=gallery` shows every control |
| `npm run symbols` | Export the icons and `symbols.css` into `src/assets/symbols` (`scripts/symbols.mjs`): SF Symbols as PNGs on macOS (skipped when the output is current), Lucide SVGs on Linux and Windows or with `VIGIA_SYMBOLS=lucide`. A `.source` file names the exported set, and a switch of set clears the directory first. Runs through the `predev`, `prebuild`, `pretest` and `precoverage` hooks; the output is not tracked in git. When npm runs with `ignore-scripts=true` the hooks are skipped, so run `npm run symbols` first. |
| `npm run build` | `tsc` then `vite build` (runs `npm run symbols` first) |
| `npm run typecheck` | `tsc --noEmit` |
| `npm test` | Vitest |
| `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` | Rust checks, run from `src-tauri/` |
| `npm run tauri build` | Produce the current platform's bundles; `beforeBuildCommand` runs `npm run build`, so the symbols are exported first |

`cargo run --example probe -- <github|gitlab> <owner/name>` (from `src-tauri/`) prints the state of one real repo using tokens from environment variables.

### Tests

`cargo test` runs the Rust unit tests in `src/` and the integration tests in `src-tauri/tests/`; `npm test` runs the Vitest suites in `src/`. The provider tests run against a local mock HTTP server (wiremock). Frontend tests use jsdom and Testing Library. `tests/app.rs` drives the runtime with an injected provider factory, clock and paused tokio time. `vigia` is a dev-dependency of itself with the `test-support` feature, which exposes `secrets::MemoryStore` to the integration tests.

Coverage:

| Command | Measures |
| --- | --- |
| `npm run coverage` | Vitest with V8 coverage for `src/` (excluding `src/dev/`), report in `coverage/` |
| `cargo cov` (from `src-tauri/`) | `cargo llvm-cov --summary-only`, leaving out `main.rs`, `lib.rs`, `settings_toolbar.rs` and `debug_setup.rs`, which only wire up a running app |

`cargo cov` needs `cargo-llvm-cov` and the `llvm-tools-preview` rustup component. The Rust lines that stay uncovered are Tauri command wrappers and AppKit calls in `commands.rs`, `tray.rs`, `windows.rs` and `notification_center.rs`; their decision logic lives in pure helpers that the tests cover.

### Scripts

| File | What it does |
| --- | --- |
| `scripts/build-app-icon.sh` | Compiles `src-tauri/icons/AppIcon.icon` with `actool` into `Assets.car`, exports a flat render, runs `tauri icon` to generate the `.icns`, `.ico` and PNG sizes, and removes the mobile and Microsoft Store outputs. Needs Xcode 26 or later. |
| `scripts/gen-tray-icons.py` | Renders the porthole, paused and badged template images, the light and dark status badges, and the finished Linux and Windows image of each look into `src-tauri/icons/tray`. Needs Pillow. `BADGE_CENTER` and `BADGE_SIZE` match the constants in `tray.rs`. |
| `scripts/symbols.mjs` | Exports the icons used by `src/components/Icon.tsx` into `src/assets/symbols`: SF Symbols through `export-symbols.swift` on macOS, Lucide SVGs from `lucide-static` elsewhere. |
| `scripts/export-symbols.swift` | Exports the SF Symbols used by `src/components/Icon.tsx` as PNGs into `src/assets/symbols`. |
| `scripts/third-party-licenses.sh` | Regenerates `THIRD-PARTY-LICENSES.md`, the notices for the npm and Cargo dependencies of every shipped target (`targets` in `src-tauri/about.toml`). |
| `scripts/webkit-snapshot.swift` | Renders a URL in `WKWebView` and saves a PNG for light or dark appearance, so system colors resolve as in the app. |
| `scripts/appkit-reference.swift` | Renders real AppKit controls as a visual reference for the webview's controls. |

### App icon

`src-tauri/icons/AppIcon.icon` is the Icon Composer source. `Assets.car` is bundled into `Resources/` (`bundle.macOS.files`) and `Info.plist` names it through `CFBundleIconName`, so macOS 26 renders the layered icon. The `.icns` and PNG files are the fallback for older systems and for development. Linux bundles use the PNG files and Windows the `.ico`.

### CI

`.github/workflows/ci.yml` runs on pull requests and pushes to `main`, on `macos-latest`, `ubuntu-22.04` (after installing the WebKitGTK, AppIndicator and other Tauri system packages) and `windows-latest`, with Node 22 and stable Rust: `npm ci`, `npm run build`, `npm test`, then `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` in `src-tauri/`. Every action is pinned to a commit SHA with its version in a comment. The workflow has `contents: read` permission and a concurrency group per ref that cancels superseded runs except on `main`.

### Release

`.github/workflows/release.yml` triggers on a tag of plain semver form, `[0-9]+.[0-9]+.[0-9]+`, with no `v` prefix. Its `preflight` job verifies that the tag equals the version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`, and that the updater signing secrets are set. Its `check` job runs `npm run typecheck`, `npm test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` on macOS, Ubuntu 22.04 and Windows. After both, `create-release` finds or creates the tag's draft release, so the parallel builds share one. The `release` matrix then builds with `tauri-apps/tauri-action` (its `beforeBuildCommand` exports the icons) and uploads to that draft by `releaseId`: `universal-apple-darwin` (aarch64 and x86_64) on macOS, x86_64 Linux on `ubuntu-22.04` so the binary runs with glibc 2.35 and later, and x86_64 Windows. Only `create-release` and `release` have `contents: write`; checkouts do not persist credentials. The release jobs restore no build caches.

Updater artifacts are built only in CI. `tauri.conf.json` keeps `bundle.createUpdaterArtifacts` off, so a local `npm run tauri build` needs no signing key. Each release build passes `--config '{"bundle":{"createUpdaterArtifacts":true}}'` (macOS adds `"targets":["app","dmg"]`), which signs the updater bundles with the `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repository secrets: the `.app.tar.gz` on macOS, the AppImage and `.deb` on Linux, and the NSIS installer on Windows. With `uploadUpdaterJson`, each job merges its entries into the release's `latest.json`, retrying on conflicts: `darwin-aarch64`, `darwin-x86_64` and their `-app` variants all point at the universal archive, `linux-x86_64` at the AppImage, and `windows-x86_64` at the NSIS installer (`updaterJsonPreferNsis`). The URLs are GitHub API asset URLs, which the updater downloads with `Accept: application/octet-stream`. `notes` is the `releaseBody` the jobs pass. The `releases/latest` URL resolves once the draft is published.

### Signing

The private updater key stays outside the repository; only its public key is in `tauri.conf.json`. The bundle is signed ad hoc (`signingIdentity` is `-`). It carries no Developer ID signature and is not notarized, so the first launch of a downloaded build needs the user to allow it in macOS. Minimum macOS version in the bundle is 10.15; the glass popup effect and layered icon need macOS 26. The Windows installer is not code signed, so SmartScreen warns on its first run; `bundle.windows.signCommand` is where a signing service would plug in.
