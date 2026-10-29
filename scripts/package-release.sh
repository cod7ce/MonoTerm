#!/usr/bin/env bash
# Never trace a private seed, including when this script is invoked with bash -x.
set +x
set +a
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$#" -lt 1 || "$#" -gt 2 || ( "$#" == 2 && "$2" != --skip-build ) ]]; then
  printf '%s\n' 'Usage: bash scripts/package-release.sh BASE_URL [--skip-build]' >&2
  printf '%s\n' 'BASE_URL is the future HTTPS archive directory. This script does not upload.' >&2
  exit 1
fi
cd "$ROOT"
BASE_URL="$1"
# Capture and unexport in one compound command, including on macOS Bash 3.2.
{ unset PRIVATE_KEY; PRIVATE_KEY="${SPARKLE_PRIVATE_KEY:-}"; unset SPARKLE_PRIVATE_KEY; }
WORK=""
LOCK=""
cleanup() {
  unset PRIVATE_KEY
  if [[ -n "$WORK" ]]; then rm -rf "$WORK"; fi
  if [[ -n "$LOCK" ]]; then rmdir "$LOCK"; fi
}
trap cleanup EXIT
python3 -I - "$BASE_URL" <<'PY'
import sys
from urllib.parse import urlsplit

value = sys.argv[1]
try:
    url = urlsplit(value)
    valid = (url.scheme == "https" and url.hostname and not url.username and not url.password
             and not url.query and not url.fragment
             and not any(c.isspace() or ord(c) < 32 or c == "\\" for c in value))
    url.port  # Reject malformed ports as well.
except ValueError:
    valid = False
if not valid:
    raise SystemExit("BASE_URL must be an HTTPS directory URL without credentials, query, or fragment.")
PY
VERSION="$(python3 -I "$ROOT/scripts/version.py" --release)"
RELEASE_ROOT="$ROOT/dist/releases"
OUTPUT="$RELEASE_ROOT/$VERSION"
if [[ -e "$OUTPUT" ]]; then
  printf '%s\n' "Release output already exists: $OUTPUT" 'Move it aside before packaging this version again.' >&2
  exit 1
fi

# Check key availability and agreement before an expensive build. This command
# never creates a Keychain record. Private input travels only over standard input.
mkdir -p "$ROOT/target/release-module-cache"
SIGN_TOOL=(xcrun swift -module-cache-path "$ROOT/target/release-module-cache" "$ROOT/scripts/update-key.swift")
PUBLIC_KEY="$(/usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$ROOT/scripts/Info.plist")"
if [[ -n "$PRIVATE_KEY" ]]; then
  SIGNING_PUBLIC_KEY="$(printf '%s\n' "$PRIVATE_KEY" | "${SIGN_TOOL[@]}" public --private-key-stdin)"
else
  SIGNING_PUBLIC_KEY="$("${SIGN_TOOL[@]}" public --existing)"
fi
if [[ "$SIGNING_PUBLIC_KEY" != "$PUBLIC_KEY" ]]; then
  printf '%s\n' 'The signing key does not match SUPublicEDKey in scripts/Info.plist.' >&2
  exit 1
fi
mkdir -p "$RELEASE_ROOT"
if ! mkdir "$RELEASE_ROOT/.$VERSION.lock"; then
  printf '%s\n' "A packaging lock already exists: $RELEASE_ROOT/.$VERSION.lock" >&2
  exit 1
fi
LOCK="$RELEASE_ROOT/.$VERSION.lock"
if [[ -e "$OUTPUT" ]]; then
  printf '%s\n' "Release output already exists: $OUTPUT" >&2
  exit 1
fi
if [[ "${2:-}" != --skip-build ]]; then
  bash "$ROOT/scripts/build-app.sh" release
fi
APP="$ROOT/dist/MonoTerm.app"
python3 -I "$ROOT/scripts/version.py" --release --bundle "$APP" >/dev/null
if [[ ! -x "$APP/Contents/MacOS/MonoTerm" || ! -f "$APP/Contents/Resources/SPARKLE-LICENSE.txt" ]]; then
  printf '%s\n' 'The release bundle is missing its executable or Sparkle license.' >&2
  exit 1
fi
python3 -I - "$APP/Contents/Frameworks/Sparkle.framework" <<'PY'
from pathlib import Path
import plistlib
import sys

framework = Path(sys.argv[1])
with (framework / "Resources/Info.plist").open("rb") as source:
    info = plistlib.load(source)
if info.get("CFBundleIdentifier") != "org.sparkle-project.Sparkle" or info.get("CFBundleShortVersionString") != "2.10.0":
    raise SystemExit("The release bundle must embed Sparkle 2.10.0.")
for relative in ("Sparkle", "Versions/B/Updater.app/Contents/MacOS/Updater",
                 "Versions/B/XPCServices/Downloader.xpc/Contents/MacOS/Downloader",
                 "Versions/B/XPCServices/Installer.xpc/Contents/MacOS/Installer"):
    if not (framework / relative).is_file():
        raise SystemExit(f"Missing Sparkle component: {relative}")
PY
# The appcast declares arm64-only hardware; inspect every embedded Mach-O.
while IFS= read -r -d '' FILE; do
  case "$(/usr/bin/file -b "$FILE")" in
    *Mach-O*)
      if [[ "$(xcrun lipo -archs "$FILE")" != arm64 ]]; then
        printf '%s\n' "Release packaging requires arm64-only code: $FILE" >&2
        exit 1
      fi
      ;;
  esac
done < <(find "$APP" -type f -print0)
codesign --verify --deep --strict "$APP"

WORK="$(mktemp -d "$RELEASE_ROOT/.package.XXXXXX")"
ARCHIVE_NAME="MonoTerm-$VERSION-arm64.zip"
ARCHIVE="$WORK/$ARCHIVE_NAME"
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$APP" "$ARCHIVE"
# Sign the archive bytes using repository code, never a downloaded executable.
if [[ -n "$PRIVATE_KEY" ]]; then
  SIGNATURE="$(printf '%s\n' "$PRIVATE_KEY" | "${SIGN_TOOL[@]}" sign "$ARCHIVE" --private-key-stdin)"
else
  SIGNATURE="$("${SIGN_TOOL[@]}" sign "$ARCHIVE")"
fi
unset PRIVATE_KEY
PUBLIC_KEY="$(/usr/libexec/PlistBuddy -c 'Print :SUPublicEDKey' "$APP/Contents/Info.plist")"
"${SIGN_TOOL[@]}" verify "$ARCHIVE" "$SIGNATURE" "$PUBLIC_KEY"

python3 -I - "$APP/Contents/Info.plist" "$BASE_URL" "$ARCHIVE" "$SIGNATURE" "$WORK/appcast.xml" "$WORK/SHA256SUMS" <<'PY'
import datetime
import email.utils
import hashlib
from pathlib import Path
import plistlib
import sys
from urllib.parse import quote
import xml.etree.ElementTree as ET

plist_path, base_url, archive_path, signature, appcast_path, checksums_path = sys.argv[1:]
with open(plist_path, "rb") as source:
    info = plistlib.load(source)
archive = Path(archive_path)
sparkle = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", sparkle)
rss = ET.Element("rss", {"version": "2.0"})
channel = ET.SubElement(rss, "channel")
ET.SubElement(channel, "title").text = "MonoTerm Updates"
ET.SubElement(channel, "link").text = info["SUFeedURL"]
ET.SubElement(channel, "description").text = "MonoTerm for Apple Silicon Macs"
ET.SubElement(channel, "language").text = "en"
item = ET.SubElement(channel, "item")
ET.SubElement(item, "title").text = f"MonoTerm {info['CFBundleShortVersionString']}"
ET.SubElement(item, "pubDate").text = email.utils.format_datetime(datetime.datetime.now(datetime.timezone.utc))
ET.SubElement(item, f"{{{sparkle}}}version").text = info["CFBundleVersion"]
ET.SubElement(item, f"{{{sparkle}}}shortVersionString").text = info["CFBundleShortVersionString"]
ET.SubElement(item, f"{{{sparkle}}}minimumSystemVersion").text = info["LSMinimumSystemVersion"]
ET.SubElement(item, f"{{{sparkle}}}hardwareRequirements").text = "arm64"
ET.SubElement(item, "enclosure", {
    "url": base_url.rstrip("/") + "/" + quote(archive.name),
    "length": str(archive.stat().st_size),
    "type": "application/octet-stream",
    f"{{{sparkle}}}edSignature": signature,
    f"{{{sparkle}}}os": "macos",
})
ET.indent(rss, space="  ")
ET.ElementTree(rss).write(appcast_path, encoding="utf-8", xml_declaration=True)
digest = hashlib.sha256()
with archive.open("rb") as source:
    for block in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(block)
Path(checksums_path).write_text(f"{digest.hexdigest()}  {archive.name}\n", encoding="utf-8")
PY

mv "$WORK" "$OUTPUT"
WORK=""
printf '%s\n' "Created local release files: $OUTPUT" "Archive: $ARCHIVE_NAME" 'Appcast: appcast.xml' 'Checksums: SHA256SUMS'
