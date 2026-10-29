#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${1:-debug}"
if [[ "$#" -gt 1 || ( "$PROFILE" != debug && "$PROFILE" != release ) ]]; then
  printf '%s\n' 'Usage: bash scripts/build-app.sh [debug|release]' >&2
  exit 1
fi
cd "$ROOT"
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  printf '%s\n' 'MonoTerm bundles currently require an Apple Silicon Mac.' >&2
  exit 1
fi
if [[ -n "${CARGO_BUILD_TARGET:-}" ]]; then
  printf '%s\n' 'Unset CARGO_BUILD_TARGET; this script builds for the native arm64 macOS host.' >&2
  exit 1
fi
export ZIG="$ROOT/scripts/zig.sh"
export MACOSX_DEPLOYMENT_TARGET=14.0
if [[ ! -x .tools/zig-0.17.0/zig || ! -f vendor/zed/crates/gpui/Cargo.toml ]]; then
  printf '%s\n' 'Run bash scripts/bootstrap.sh first.' >&2
  exit 1
fi
# Cargo is authoritative; a local bundle may have no update key configured yet.
python3 -I "$ROOT/scripts/version.py" >/dev/null
SPARKLE_SDK="$(bash "$ROOT/scripts/fetch-sparkle.sh")"
export SPARKLE_SDK
CARGO_ARGS=(build --locked --package monoterm --manifest-path "$ROOT/Cargo.toml" --target-dir "$ROOT/target")
if [[ "$PROFILE" == release ]]; then
  CARGO_ARGS+=(--release)
  OUTPUT_DIR="$ROOT/dist"
else
  OUTPUT_DIR="$ROOT/target"
fi
cargo "${CARGO_ARGS[@]}"

mkdir -p "$OUTPUT_DIR"
STAGING="$(mktemp -d "$OUTPUT_DIR/.bundle.XXXXXX")"
DESTINATION="$OUTPUT_DIR/MonoTerm.app"
PREVIOUS="$STAGING/previous.app"
cleanup() {
  # Restore the prior bundle if publication failed after moving it aside.
  if [[ -d "$PREVIOUS" && ! -e "$DESTINATION" ]]; then
    mv "$PREVIOUS" "$DESTINATION"
  fi
  rm -rf "$STAGING"
}
trap cleanup EXIT
APP="$STAGING/MonoTerm.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" \
  "$APP/Contents/Resources/licenses/herdr" "$APP/Contents/Resources/licenses/ghostty" \
  "$APP/Contents/Resources/licenses/gpui-ghostty" "$APP/Contents/Resources/licenses/gpui"
cp "$ROOT/target/$PROFILE/monoterm" "$APP/Contents/MacOS/MonoTerm"
chmod 755 "$APP/Contents/MacOS/MonoTerm"
cp "$ROOT/scripts/Info.plist" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :MonoTermBuildProfile string $PROFILE" "$APP/Contents/Info.plist"
cp "$ROOT/assets/AppIcon.icns" "$APP/Contents/Resources/AppIcon.icns"
cp "$ROOT/vendor/gpui-ghostty/vendor/ghostty/LICENSE" "$APP/Contents/Resources/licenses/ghostty/"
cp "$ROOT/vendor/gpui-ghostty/LICENSE" "$APP/Contents/Resources/licenses/gpui-ghostty/"
cp "$ROOT/vendor/zed/LICENSE-APACHE" "$APP/Contents/Resources/licenses/gpui/"
cp "$ROOT/vendor/herdr/LICENSE" "$ROOT/vendor/herdr/NOTICE" \
  "$ROOT/vendor/herdr/README.md" "$APP/Contents/Resources/licenses/herdr/"
cp "$ROOT/vendor/herdr/herdr-protocol/LICENSE-APACHE" \
  "$ROOT/vendor/herdr/herdr-protocol/NOTICE.md" "$APP/Contents/Resources/licenses/herdr/"
if [[ -f "$ROOT/LICENSE" ]]; then
  cp "$ROOT/LICENSE" "$APP/Contents/Resources/LICENSE.txt"
fi
# Keep framework symlinks intact and never modify the SDK input.
/usr/bin/ditto "$SPARKLE_SDK/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"
cp "$SPARKLE_SDK/LICENSE" "$APP/Contents/Resources/SPARKLE-LICENSE.txt"
if [[ "$(xcrun lipo -archs "$APP/Contents/MacOS/MonoTerm")" != arm64 ]]; then
  printf '%s\n' 'Expected an arm64-only MonoTerm executable.' >&2
  exit 1
fi

# Sparkle's downloader runs sandboxed even though the host app is not sandboxed.
# Some upstream builds ship empty entitlements; supply its required pair explicitly.
DOWNLOADER_ENTITLEMENTS="$STAGING/downloader-entitlements.plist"
python3 -I - "$DOWNLOADER_ENTITLEMENTS" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "wb") as destination:
    plistlib.dump({"com.apple.security.app-sandbox": True,
                  "com.apple.security.network.client": True}, destination)
PY
SIGNING_ARGS=(--force --sign "${CODE_SIGN_IDENTITY:--}")
if [[ "${CODE_SIGN_IDENTITY:--}" != - ]]; then
  SIGNING_ARGS+=(--options runtime --timestamp)
fi
sign_code() {
  case "$1" in
    */Downloader.xpc|*/Downloader.xpc/Contents/MacOS/Downloader)
      codesign "${SIGNING_ARGS[@]}" --entitlements "$DOWNLOADER_ENTITLEMENTS" "$1"
      ;;
    *)
      if codesign --display "$1" >/dev/null 2>&1; then
        codesign "${SIGNING_ARGS[@]}" --preserve-metadata=entitlements "$1"
      else
        codesign "${SIGNING_ARGS[@]}" "$1"
      fi
      ;;
  esac
}
# Thin and sign Mach-O leaves, then sign nested bundles from deepest to outermost.
while IFS= read -r -d '' FILE; do
  case "$(/usr/bin/file -b "$FILE")" in
    *Mach-O*)
      ARCHES="$(xcrun lipo -archs "$FILE")"
      if [[ " $ARCHES " != *" arm64 "* ]]; then
        printf '%s\n' "Missing arm64 architecture: $FILE" >&2
        exit 1
      fi
      if [[ "$ARCHES" != arm64 ]]; then
        xcrun lipo "$FILE" -thin arm64 -output "$FILE.thin"
        chmod "$(stat -f '%Lp' "$FILE")" "$FILE.thin"
        mv -f "$FILE.thin" "$FILE"
      fi
      sign_code "$FILE"
      ;;
  esac
done < <(find "$APP" -type f -print0)
while IFS= read -r -d '' BUNDLE; do
  sign_code "$BUNDLE"
done < <(find "$APP" -depth -type d \( -name '*.app' -o -name '*.xpc' -o -name '*.framework' \) -print0)
codesign --verify --deep --strict "$APP"
python3 -I "$ROOT/scripts/version.py" --bundle "$APP" >/dev/null

# Publish by rename on the same filesystem. Never overwrite or re-sign a Mach-O
# inode used by an existing process, including its loaded Sparkle framework.
if [[ -e "$DESTINATION" ]]; then
  mv "$DESTINATION" "$PREVIOUS"
fi
mv "$APP" "$DESTINATION"
# Refresh this generated bundle's icon registration without launching the app.
touch "$DESTINATION"
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
if [[ -x "$LSREGISTER" ]]; then
  "$LSREGISTER" -f "$DESTINATION" || printf '%s\n' 'Could not refresh the MonoTerm application registration.' >&2
fi
printf '%s\n' "Built: $DESTINATION ($PROFILE, arm64, signing identity: ${CODE_SIGN_IDENTITY:--})"
