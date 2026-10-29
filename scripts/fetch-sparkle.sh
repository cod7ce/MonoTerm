#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="2.10.0"
SHA256="c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c"
URL="https://github.com/sparkle-project/Sparkle/releases/download/$VERSION/Sparkle-$VERSION.tar.xz"
CACHE="$ROOT/target/sparkle"
PATHFILE="$CACHE/sdk-path"
# Never run interpreters from a downloaded or extracted directory.
cd "$ROOT"

validate_sdk() {
  python3 -I - "$1" "$VERSION" <<'PY'
from pathlib import Path
import plistlib
import sys

sdk = Path(sys.argv[1])
try:
    with (sdk / "Sparkle.framework/Resources/Info.plist").open("rb") as source:
        info = plistlib.load(source)
    valid = (info.get("CFBundleIdentifier") == "org.sparkle-project.Sparkle"
             and info.get("CFBundleShortVersionString") == sys.argv[2]
             and (sdk / "Sparkle.framework/Sparkle").is_file()
             and (sdk / "LICENSE").is_file())
except (OSError, ValueError, plistlib.InvalidFileException):
    valid = False
if not valid:
    raise SystemExit(f"Expected the Sparkle {sys.argv[2]} framework and LICENSE in {sdk}.")
PY
}

# An explicit local SDK is a trusted developer input, not a verified download.
# It is read only; build-app.sh copies its framework before modifying anything.
if [[ -n "${SPARKLE_SDK_DIR:-}" ]]; then
  if [[ "$SPARKLE_SDK_DIR" != /* || ! -d "$SPARKLE_SDK_DIR" ]]; then
    printf '%s\n' 'SPARKLE_SDK_DIR must be an absolute path to an existing SDK directory.' >&2
    exit 1
  fi
  SDK="$(cd "$SPARKLE_SDK_DIR" && pwd -P)"
  validate_sdk "$SDK"
  printf '%s\n' "Using the explicit local Sparkle SDK (archive digest not reverified): $SDK" >&2
  printf '%s\n' "$SDK"
  exit 0
fi

mkdir -p "$CACHE"
if [[ -f "$PATHFILE" ]]; then
  SDK="$(<"$PATHFILE")"
  if [[ "$SDK" == "$CACHE"/sdk.* && -f "$SDK/.verified-sha256" ]] &&
    [[ "$(<"$SDK/.verified-sha256")" == "$SHA256" ]] && validate_sdk "$SDK"; then
    printf '%s\n' "$SDK"
    exit 0
  fi
fi

# Every download and extraction starts in its own new, empty directory.
DOWNLOAD="$(mktemp -d "$CACHE/download.XXXXXX")"
SDK="$(mktemp -d "$CACHE/sdk.XXXXXX")"
PENDING_PATHFILE=""
COMPLETE=false
cleanup() {
  rm -rf "$DOWNLOAD"
  if [[ "$COMPLETE" != true ]]; then
    rm -rf "$SDK"
  fi
  if [[ -n "$PENDING_PATHFILE" ]]; then
    rm -f "$PENDING_PATHFILE"
  fi
}
trap cleanup EXIT
ARCHIVE="$DOWNLOAD/Sparkle-$VERSION.tar.xz"
curl --fail --location --silent --show-error --retry 3 \
  --proto '=https' --proto-redir '=https' --output "$ARCHIVE" "$URL"
ACTUAL_SHA256="$(shasum -a 256 "$ARCHIVE")"
ACTUAL_SHA256="${ACTUAL_SHA256%% *}"
if [[ "$ACTUAL_SHA256" != "$SHA256" ]]; then
  printf '%s\n' "Sparkle $VERSION archive SHA256 mismatch; refusing to extract." >&2
  exit 1
fi

# Validate entries as data before tar extracts. No downloaded tools are run.
python3 -I - "$ARCHIVE" <<'PY'
import pathlib
import sys
import tarfile

with tarfile.open(sys.argv[1], "r:xz") as archive:
    for member in archive:
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise SystemExit(f"Unsafe archive path: {member.name!r}")
        if not (member.isfile() or member.isdir() or member.issym() or member.islnk()):
            raise SystemExit(f"Unsupported archive entry: {member.name!r}")
        if member.issym() or member.islnk():
            target = pathlib.PurePosixPath(member.linkname)
            if target.is_absolute():
                raise SystemExit(f"Unsafe archive link: {member.name!r}")
            parts = list(path.parent.parts) if member.issym() else []
            for part in target.parts:
                if part == "..":
                    if not parts:
                        raise SystemExit(f"Archive link escapes SDK: {member.name!r}")
                    parts.pop()
                elif part != ".":
                    parts.append(part)
PY
/usr/bin/tar -xf "$ARCHIVE" -C "$SDK"
validate_sdk "$SDK"
printf '%s\n' "$SHA256" > "$SDK/.verified-sha256"
PENDING_PATHFILE="$(mktemp "$CACHE/sdk-path.XXXXXX")"
printf '%s\n' "$SDK" > "$PENDING_PATHFILE"
mv -f "$PENDING_PATHFILE" "$PATHFILE"
COMPLETE=true
printf '%s\n' "$SDK"
