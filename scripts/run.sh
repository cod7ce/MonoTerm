#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash "$project_dir/scripts/build-app.sh"
exec "$project_dir/target/MonoTerm.app/Contents/MacOS/MonoTerm" "$@"
