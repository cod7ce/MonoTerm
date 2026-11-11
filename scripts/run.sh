#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash "$project_dir/scripts/build-app.sh"
app_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$project_dir/scripts/Info.plist")"
executable="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$project_dir/scripts/Info.plist")"
exec "$project_dir/target/$app_name.app/Contents/MacOS/$executable" "$@"
