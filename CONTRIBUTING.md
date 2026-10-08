# Contributing to Vigia

Vigia is a Tauri v2 app: a React and TypeScript frontend in `src/` and a Rust backend in `src-tauri/`. [docs/architecture.md](docs/architecture.md) describes how the parts fit together. The user guide is [README.md](README.md).

## Prerequisites

- macOS, Linux or Windows. Each platform builds its own bundles: `.dmg` on macOS, AppImage and `.deb` on Linux, an NSIS installer on Windows.
- Linux: the Tauri system packages. On Debian and Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev patchelf build-essential`.
- Windows: the Microsoft C++ Build Tools and WebView2 (preinstalled on Windows 11).
- Node 22, the version CI uses.
- A Rust toolchain at or above `rust-version` in `src-tauri/Cargo.toml`. CI runs the latest stable release.
- Xcode 26 or later, only to rebuild the app icon with `scripts/build-app-icon.sh`.
- Python with Pillow, only to regenerate the menu bar icons with `scripts/gen-tray-icons.py`.

## Getting started

```sh
npm ci
npm run tauri dev
```

`npm run symbols` exports the icons. On macOS it exports SF Symbols, skipping the work when the output is current. On Linux and Windows it copies Lucide icons, because the SF Symbols licence covers Apple platforms only. `VIGIA_SYMBOLS=lucide npm run symbols` exports the Lucide set on macOS, to check those builds; run `npm run symbols` again to switch back. It runs through the `predev`, `prebuild`, `pretest` and `precoverage` hooks. When npm is configured with `ignore-scripts=true` the hooks are skipped, so run `npm run symbols` first. The output in `src/assets/symbols/` is generated and not tracked.

`npm run tauri build` produces the bundles for the current platform. `tauri.conf.json` holds the shared configuration, and `tauri.macos.conf.json`, `tauri.linux.conf.json` and `tauri.windows.conf.json` add each platform's bundle targets and popup window. Arrays such as `app.windows` replace the base value whole, so a platform file repeats the full window entry.

## Checks

CI runs these on macOS, Ubuntu 22.04 and Windows for every pull request and push to `main`:

```sh
npm ci
npm run build
npm test
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`npm run typecheck` runs `tsc --noEmit` on its own. CI installs the latest stable Rust, so also run `cargo +stable clippy --all-targets -- -D warnings`; new lints in a newer toolchain fail CI.

## Coverage

- Frontend: `npm run coverage`.
- Rust: `cargo cov` from `src-tauri/`. It needs `cargo install cargo-llvm-cov` and `rustup component add llvm-tools-preview`.

## Debug builds

Debug builds skip the fine-grained check for tokens passed through `VIGIA_DEBUG_SETUP`, so `gh auth token` works there. They keep their settings in `config.dev.json` and their tokens in `tokens.dev.json`, both in the app config folder (`~/Library/Application Support/com.s3rgiosan.vigia/` on macOS), so they do not touch a release install or the system credential store. Three environment variables apply to them:

- `VIGIA_DEBUG_SETUP`: JSON describing accounts and repos to add at launch. See `src-tauri/src/debug_setup.rs`.
- `VIGIA_DEBUG_SHOW_POPUP`: open the popup at launch.
- `VIGIA_DEBUG_OPEN_SETTINGS`: open Settings at launch.

The tray menu of a debug build also has a Debug submenu that forces each icon state.

## Browser preview with mock scenarios

`npm run dev` serves the windows with fake data from `src/dev/mock.ts`:

- `http://localhost:1420/?view=popup`
- `http://localhost:1420/?view=settings`
- `http://localhost:1420/?view=gallery` (every control)

Add `&scenario=` with one of these values to preview a state:

- `empty`: no accounts or repositories.
- `banners`: a rejected token and an unreachable server.
- `missing_token`: an account without a stored token.
- `config_error`: an unreadable settings file.
- `keychain`: Keychain access blocked.
- `many`: a long repository picker.
- `update`: an available update.

## Probe example

The probe checks one real repository without the app. `VIGIA_INCLUDE_TAGS=1` counts tag runs.

```sh
cd src-tauri
VIGIA_GITHUB_TOKEN=... VIGIA_INCLUDE_TAGS=1 cargo run --example probe -- github owner/name
VIGIA_GITLAB_URL=https://gitlab.example.com VIGIA_GITLAB_TOKEN=... cargo run --example probe -- gitlab group/project
```

## Scripts

- `scripts/symbols.mjs` exports the icons into `src/assets/symbols/`: SF Symbols through `scripts/export-symbols.swift` on macOS, Lucide SVGs elsewhere; run it with `npm run symbols`.
- `scripts/gen-tray-icons.py` generates the menu bar icons, and the finished per-state images for Linux and Windows (needs Pillow).
- `scripts/build-app-icon.sh` builds the app icon from the Icon Composer document `src-tauri/icons/AppIcon.icon` (needs Xcode 26 or later).
- `scripts/third-party-licenses.sh` regenerates `THIRD-PARTY-LICENSES.md`.
- `scripts/webkit-snapshot.swift <url> <width> <height> <out.png> <light|dark> [script]` renders a page in WebKit, the engine the app uses, so system colors resolve as they do in the app.
- `scripts/appkit-reference.swift <dir>` renders the matching real AppKit controls for comparison.

## Licences

Run `sh scripts/third-party-licenses.sh` when dependencies change to regenerate `THIRD-PARTY-LICENSES.md`.

## Release

1. Set the same version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`.
2. In `CHANGELOG.md` (Keep a Changelog), move the `## [Unreleased]` entries under a new `## [x.y.z] - YYYY-MM-DD` heading and update the link references at the bottom of the file.
3. Push a tag with the same plain semver number and no `v` prefix, for example `1.0.0`.

The release workflow verifies that the tag and the three versions match, runs the checks on macOS, Ubuntu and Windows, and creates one draft release. A build matrix then attaches a universal ad-hoc signed `.dmg`, a Linux AppImage and `.deb` built on Ubuntu 22.04, and an unsigned Windows NSIS installer, signs each platform's updater bundle, and merges each platform into `latest.json`. It needs the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; the signing key itself stays outside the repository.

Before publishing, test the bundles and edit the release notes of the draft. The updater's `latest.json` embeds the release body, so the notes must be final before the release is published. Then publish the draft.
