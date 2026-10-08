# AGENTS.md

Vigia is a menu bar and tray app (Tauri v2) for macOS, Linux and Windows that shows CI run status for GitHub Actions and GitLab CI across many repositories. The frontend is React and TypeScript; the backend is Rust.

## Layout

- `src/`: frontend (popup, settings, `lib/`, `lib/hooks/`, `App.css` tokens, `dev/mock.ts` browser preview data).
- `src-tauri/src/`: Rust backend (providers, poller, settings, notifications, updates).
- `src-tauri/tests/`: Rust integration tests; `src-tauri/examples/probe.rs` queries one real repository.
- `scripts/`: icon, symbol and licence tooling.
- `docs/architecture.md`: the map of the internals. Read it before larger changes.

## Commands

- Setup: `npm ci`.
- Dev: `npm run tauri dev`; `npm run dev` serves the windows with mock data in a browser.
- `npm run symbols` exports the icons: SF Symbols on macOS, Lucide on Linux and Windows. The `predev`, `prebuild`, `pretest` and `precoverage` hooks run it; with `ignore-scripts=true` they are skipped, so run it first.
- Before finishing, run all of:
  - `npm run typecheck`
  - `npm test`
  - `npm run build`
  - `cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  - `cargo +stable clippy --all-targets -- -D warnings` (CI uses the latest stable Rust; new lints have broken releases before)

## Rules

- GitHub and GitLab behave the same. Add no feature that only one provider can support.
- macOS, Linux and Windows share one codebase. AppKit code stays behind `cfg(target_os = "macos")` with a fallback for the others; per-platform Tauri settings go in `tauri.{macos,linux,windows}.conf.json`; the frontend reads the platform from `src/lib/platform.ts`.
- Filters resolve repository, then organization (host and owner; top-level group on GitLab), then global. `null` inherits and `[]` means "nothing".
- Comments and docs describe the code as it is: present tense, affirmative, no history ("previously", "instead of", "now"), no references to design sources.
- In TypeScript and Rust, use `switch`/`match` with a `default` for 3 or more literal comparisons on one variable. Assign value-producing calls to named locals before comparing them.
- Write allowlist and denylist, never whitelist or blacklist.
- Reuse existing components, hooks (`src/lib/hooks`) and tokens (`src/App.css`) before adding new ones. The UI follows macOS conventions; status is a coloured dot plus a label.
- Tests use fabricated data only (acme, example.com). Never use real organisations, repositories, tokens or personal data. Keep coverage at its current level.
- Never commit secrets. The updater signing key lives outside the repo (`~/.tauri/`). Tokens live in the OS credential store (the Keychain on macOS); debug builds use `tokens.dev.json` in the app config dir, never in the repo.
- `CHANGELOG.md` follows Keep a Changelog 1.1.0. Add entries under `## [Unreleased]` in typed sections (Added, Changed, Fixed, …) as full sentences.
- Tags are plain semver without `v`.
- No Prettier. Match the surrounding formatting; run `cargo fmt` for Rust.
- Do not commit generated files: `src/assets/symbols/*`, `coverage/`, `target/`, `dist/`.
- Commit messages: a lowercase imperative summary of the change. No AI attribution lines or session links.

## Pointers

- [README.md](README.md): user guide.
- [CONTRIBUTING.md](CONTRIBUTING.md): contributor guide (prerequisites, checks, debug builds, release).
- [docs/architecture.md](docs/architecture.md): internals.
