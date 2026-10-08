# Vigia

Vigia is a macOS menu bar app that shows the state of CI runs across many repositories. It reads GitHub Actions on github.com and GitLab CI on gitlab.com or self-managed instances.

The menu bar icon is a porthole. It has no badge when idle or paused, and a coloured badge with a glyph for the worst state across every watched repository: red × for a failure, orange ! when something could not be checked, yellow ••• while runs are in progress, green ✓ when everything passes. The popup lists each repository with its latest run and opens it in the browser.

## Requirements

macOS 10.15 or later. The popup uses Liquid Glass on macOS 26 and a popover material on earlier versions.

## Install

1. Download the `.dmg` from the latest release and drag Vigia to Applications.
2. The build is ad-hoc signed, not notarized. macOS blocks the first launch:
   try to open the app once, then open System Settings, go to Privacy & Security, and choose **Open Anyway** for Vigia. On macOS 15 and later the button appears only after that first attempt.
   Alternative in a terminal: `xattr -dr com.apple.quarantine /Applications/Vigia.app`.
3. Vigia has no Dock icon. Look for the porthole in the menu bar.

After each update, macOS asks once for the login keychain password, because each ad-hoc signed build has a new code identity that the Keychain item's access list does not match. Enter the password and choose **Always Allow**.

## Setup

1. Click the porthole, then **Add Account…**.
2. GitHub: Vigia accepts fine-grained tokens only (they start with `github_pat_`). Click **Create Token on GitHub** in the form: it opens GitHub with the name, a 90-day expiry, and the two permissions filled in (Actions: Read-only, Metadata: Read-only). Enter the owner in **Organization** first if the repos belong to one. Choose the repository access on GitHub, generate the token, and paste it back. One token covers one owner; add one account per owner. Organizations that enforce SAML single sign-on need the token authorized for SSO, or their repositories will not appear.
   GitLab: enter the server URL and a personal access token with the `read_api` scope. Servers with an internal certificate authority work when it is trusted in the macOS Keychain. A server URL starting with `http://` needs the consent checkbox "This server uses http://. Send the token unencrypted anyway."
3. Use **Test Connection** to check the token, then **Add Account**.
4. Pick repos in the Repositories pane. Each org has a select-all checkbox.
5. Optional: set branch globs such as `release/*` in the Filters pane. The default watches each repo's default branch.

## Using Vigia

- Left click the porthole to open the popup. Right click for the menu: Open Vigia, Refresh Now (⌘R), Pause or Resume, Settings… (⌘,), About Vigia, Quit Vigia.
- The popup groups repos into Failed, Errors, Running, Passing and No runs. Failed, Errors and Running start open; Passing and No runs start collapsed.
- ⌘F or typing starts a filter; Esc clears it. ⌘R refreshes, ⌘, opens Settings.
- ↑/↓ move between rows, → and ← expand and collapse, Return opens the run in the browser. ← on a run row returns to its repository row. Repository and run rows show an open-in-browser icon on hover and keyboard focus.
- The Menu key or Shift+F10 opens the row menu; right click does the same. ⌘C copies the link and ⌘⌫ stops watching, with an **Undo** in the popup.
- Row menu: Open Run, Open Actions (Open Pipelines on GitLab), Open Repository, Copy Run Link (Copy Repository Link when there is no run), Stop Watching.

## Settings

- **Accounts**: **Add Account…**, rename, **Test Connection**, **Replace Token…**, and remove (minus button). The More Actions (…) menu holds **Remove All Accounts…**, available with two or more accounts.
- **Repositories**: search, reload, a select-all checkbox per org, and "Watched via …" on a repo another account of the same instance already covers.
- **Filters**: a sidebar lists **All Repositories** and every organization with a watched repository, grouped by host; an organization with its own filters reads "Custom". **All Repositories** holds the global filters: a branch glob list ("Default branch" when empty), **Ignored workflows** and **Tag runs**. **Ignored workflows** takes case-insensitive globs matched against the GitHub workflow name or the GitLab pipeline name (the pipeline source, such as `schedule` or `web`, when it has no name). `Dependabot*` hides runs of GitHub's "Dependabot Updates" workflow, whose run names look like "composer in /. for …". Ignored workflows do not count toward a repository's status, the menu bar badge or notifications. **Tag runs** includes the latest run of each workflow started by a tag, such as a Release, as one entry per workflow showing the tag name; it is off by default. A list holds up to 50 patterns of up to 200 characters.
- **General**: **Check every** (15 seconds to 10 minutes), **Ignore pull request runs**, the two notification switches, and **Open at login**, which adds a LaunchAgent at `~/Library/LaunchAgents/Vigia.plist`; turning it off removes the file.

### Organization filters and exceptions

Branches, Ignored workflows and Tag runs resolve in this order: the repository's exception, then its organization, then All Repositories. Select an organization in the Filters pane to set its own filters: **Default** uses the settings for all repositories (the field is disabled and shows the inherited value), **Custom** applies to every repository of the organization. A Custom list that is empty means "nothing": only the default branch for Branches, no ignored workflows for Ignored workflows. Tag runs offers Default (On) or Default (Off), following All Repositories, or On or Off.

Under the organization's filters, **Repository Exceptions** lists the repositories that override them, each with a summary such as "Tag runs: On". Click a row, or press →, to show its three filters; their Default follows the organization. + adds repositories of the organization from a sheet, and − clears the selected repository's overrides.

Example: All Repositories ignores `Dependabot*`, but one organization wants to see Dependabot runs. Select that organization in the Filters pane, set **Ignored workflows** to **Custom** and leave the field empty.

## Notifications

The General pane has two switches: **When a branch fails** and **When it recovers**. **When it recovers** is disabled while failure notifications are off. While macOS has no answer recorded, it asks for notification permission when you add an account or turn on a notification switch. Allow it in the prompt, or later under System Settings, Notifications.

Clicking a notification for a single repository opens its run. A summary notification (more than three failures in one poll) and a token-rejected notification open the popup.

## Known limits

- Vigia reads the first page of recent runs for each repository. On a busy repository, older runs of a quiet branch can fall off that page.
- A rarely run workflow, such as a weekly schedule, can fall off the first page too. Its last failure then stops counting.
- GitLab child pipelines are not checked. A parent pipeline can pass while a child failed.
- Tag runs are detected from the repository's tag list on GitHub and GitLab, refreshed when a run's ref is not the default branch, a known tag or an already checked ref. A branch named like a tag counts as that tag. Vigia reads at most 10 pages of a repository's tag list.
- GitLab pipelines without a name are matched by their source, such as `schedule` or `web`.
- An archived repository keeps its last state until you stop watching it.

## Privacy

No telemetry. Vigia contacts only the hosts of the accounts you add. Tokens live in the macOS Keychain and never leave the machine except in requests to those hosts. Logs never quote tokens or Keychain contents.

## Troubleshooting

- "Vigia can’t read its tokens from the Keychain." Allow access, then choose **Try Again**. **Reset Keychain Item…** deletes all stored tokens; each account then needs its token entered again with **Replace Token…**.
- "Vigia couldn’t read its settings file, so changes won’t be saved." appears when Vigia cannot read its settings file. Fix the file's permissions or delete it, then reopen Vigia.
- "The token for “<account>” was rejected." Choose **Replace Token…** in the banner and enter a new token.
- A repository is Failed by a Dependabot run: add `Dependabot*` to Ignored workflows in the Filters pane, or fix Dependabot's access to the repository.
- A workflow that runs on tags (such as a Release) is missing: turn on **Tag runs** in the Filters pane.
- Polling slows down on its own when steady polling would use more than 40 percent of the API rate limit; the popup footer ends with "slowed to every …". A GitLab server that sends no rate limit headers is never slowed this way.
- Logs are written to `~/Library/Logs/com.s3rgiosan.vigia/Vigia.log`.
- Settings live in `~/Library/Application Support/com.s3rgiosan.vigia/config.json`. A file that is not valid JSON is moved aside as `config.json.corrupt-<timestamp>` and replaced by defaults.

## Uninstall

1. Optional: choose **Remove All Accounts…** in Settings to delete the stored tokens, or run `security delete-generic-password -s com.s3rgiosan.vigia -a tokens` afterwards.
2. Turn off **Open at login** in the General pane, or delete `~/Library/LaunchAgents/Vigia.plist` after quitting.
3. Quit Vigia from the right-click menu.
4. Drag Vigia from Applications to the Trash.
5. Delete `~/Library/Application Support/com.s3rgiosan.vigia/` and `~/Library/Logs/com.s3rgiosan.vigia/`.

## Licences

Vigia is MIT licensed; see `LICENSE`. Third-party notices are in `THIRD-PARTY-LICENSES.md`. The icons use SF Symbols that are generated on the build machine from macOS and are not distributed in this repository.

## Contributing

Building from source or contributing: see [CONTRIBUTING.md](CONTRIBUTING.md).
