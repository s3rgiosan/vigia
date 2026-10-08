#!/usr/bin/env bash
# Regenerates THIRD-PARTY-LICENSES.md from the Rust and npm production dependencies.
# Requires: cargo-about (cargo install cargo-about --locked --features cli), node, npm ci.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/THIRD-PARTY-LICENSES.md"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if ! command -v cargo-about >/dev/null 2>&1; then
  cargo install cargo-about --locked --features cli
fi

(cd "$root/src-tauri" && cargo about generate -c about.toml about.hbs -o "$tmp/rust.md")

(cd "$root" && npm ls --omit=dev --all --json > "$tmp/npm.json" || true)

node - "$root" "$tmp/npm.json" "$tmp/js.md" <<'JS'
const fs = require('fs');
const path = require('path');
const [root, npmJson, outFile] = process.argv.slice(2);
const tree = JSON.parse(fs.readFileSync(npmJson, 'utf8'));
const pkgs = new Map();
(function walk(deps) {
  for (const [name, info] of Object.entries(deps || {})) {
    if (!pkgs.has(name)) pkgs.set(name, info.version);
    walk(info.dependencies);
  }
})(tree.dependencies);

const groups = new Map();
for (const [name, version] of [...pkgs].sort()) {
  const dir = path.join(root, 'node_modules', name);
  const meta = JSON.parse(fs.readFileSync(path.join(dir, 'package.json'), 'utf8'));
  let lic = meta.license || (meta.licenses && meta.licenses.map((l) => l.type).join(' OR ')) || 'UNKNOWN';
  if (typeof lic === 'object') lic = lic.type;
  const file = fs.readdirSync(dir).find((f) => /^(licen[sc]e|copying)/i.test(f));
  const text = file ? fs.readFileSync(path.join(dir, file), 'utf8').trim() : `Licence: ${lic} (no licence file shipped in the package).`;
  const key = lic + '\n' + text;
  if (!groups.has(key)) groups.set(key, { lic, text, items: [] });
  groups.get(key).items.push(`${name} ${version}`);
}
let md = '## JavaScript packages\n\n';
for (const g of [...groups.values()].sort((a, b) => a.lic.localeCompare(b.lic))) {
  md += `### ${g.lic}\n\nUsed by:\n\n${g.items.map((i) => `- ${i}`).join('\n')}\n\n<details><summary>Licence text</summary>\n\n\`\`\`text\n${g.text}\n\`\`\`\n\n</details>\n\n`;
}
fs.writeFileSync(outFile, md);
JS

{
  echo "# Third-party licences"
  echo
  echo "Vigia bundles the following open-source software. Vigia itself is MIT licensed (see LICENSE)."
  echo
  cat "$tmp/rust.md"
  echo
  cat "$tmp/js.md"
} > "$out"
echo "Wrote $out"
