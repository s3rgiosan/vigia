# Changelog

All notable changes to Vigia will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.1] - 2026-10-10

### Changed

- Sheets in Settings slide back up when they close, the way they slide down when they open.
- Repository and run rows, section headings, segmented controls and the Settings pane buttons darken as soon as they are pressed.
- The popup grows out of the edge next to the tray icon when it opens: the top, or the bottom on Windows, where it opens above the taskbar.
- With Reduce Motion on, the popup and sheets fade in and out in place.
- The popup list fades out at an edge where there is more to scroll to, and the separator above the footer is gone.
- Banners in the popup fade in when they appear.
- The status dot in the popup header pulses while runs are in progress, like the dots on the rows.

### Fixed

- The Refresh button keeps its spinning icon at full strength while a refresh runs, and pulses it when Reduce Motion is on.

## [1.2.0] - 2026-10-10

### Added

- The General pane sorts the popup’s repositories by name or by most recent run, and lists them grouped by organization or as one list per status section.

## [1.1.1] - 2026-10-08

### Fixed

- On Windows, opening Settings from the popup or the tray menu no longer freezes Vigia with a blank Settings window.

## [1.1.0] - 2026-10-08

### Added

- Vigia runs on Linux, as an AppImage or a `.deb`, and on Windows, with an NSIS installer, and each release ships them next to the macOS `.dmg`.
- On Linux and Windows the tray icon shows the same status badges, Settings has its own pane switcher, and the in-app updater installs new versions.

### Fixed

- The popup and Settings name the cause of an account’s sign-in problem: a rejected token, a missing token, or a token Vigia can’t read from the Keychain.

## [1.0.0] - 2026-10-08

### Added

- First release.

[Unreleased]: https://github.com/s3rgiosan/vigia/compare/1.2.1...HEAD
[1.2.1]: https://github.com/s3rgiosan/vigia/compare/1.2.0...1.2.1
[1.2.0]: https://github.com/s3rgiosan/vigia/compare/1.1.1...1.2.0
[1.1.1]: https://github.com/s3rgiosan/vigia/compare/1.1.0...1.1.1
[1.1.0]: https://github.com/s3rgiosan/vigia/compare/1.0.0...1.1.0
[1.0.0]: https://github.com/s3rgiosan/vigia/releases/tag/1.0.0
