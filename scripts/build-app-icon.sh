#!/bin/sh
# Builds the fallback icon files from the Icon Composer document.
#
# src-tauri/icons/AppIcon.icon is the source of truth. This script:
# - compiles it into Assets.car, which macOS 26 and later read to render the glass and the
#   Default, Dark, Clear and Tinted variants. actool only emits the catalog for a deployment
#   target of 26, so it is compiled here; the bundle copies it in (bundle.macOS.files) and
#   Info.plist names it (CFBundleIconName);
# - exports a flat 1024 px render for older macOS versions and for `tauri dev`, then generates
#   the .icns, .ico and PNG sizes from it.
# Requires Xcode 26 or later. Run from the repo root and commit the results.
set -eu

ICTOOL="/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool"
# actool resolves relative paths against the wrong directory, so every path is absolute.
ROOT=$(cd "$(dirname "$0")/.." && pwd)
ICONS="$ROOT/src-tauri/icons"
cd "$ROOT"

OUT=$(mktemp -d)
xcrun actool "$ICONS/AppIcon.icon" --compile "$OUT" --app-icon AppIcon --include-all-app-icons \
  --platform macosx --target-device mac --minimum-deployment-target 26.0 \
  --output-partial-info-plist "$OUT/partial.plist" --output-format human-readable-text --errors --warnings
if [ ! -f "$OUT/Assets.car" ]; then
  echo "actool produced no Assets.car. A stale Xcode helper can cause this: run 'pkill ibtoold' and retry." >&2
  exit 1
fi
cp "$OUT/Assets.car" "$ICONS/Assets.car"
rm -rf "$OUT"

"$ICTOOL" "$ICONS/AppIcon.icon" --export-image --output-file "$ICONS/app-icon.png" \
  --platform macOS --rendition Default --width 512 --height 512 --scale 2

npx tauri icon "$ICONS/app-icon.png"

# Vigia ships for macOS, Linux and Windows; the mobile and Microsoft Store sizes go.
rm -rf "$ICONS/android" "$ICONS/ios" "$ICONS"/Square*.png "$ICONS/StoreLogo.png" \
  "$ICONS/64x64.png" "$ICONS/icon.png" "$ICONS/app-icon.png"
