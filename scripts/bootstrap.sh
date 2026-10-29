#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  printf 'This prototype bootstrap currently targets Apple Silicon macOS.\n' >&2
  exit 1
fi
command -v cargo >/dev/null
xcrun --sdk macosx --show-sdk-path >/dev/null
component_repository=https://github.com/cod7ce/gpui-ghostty.git
component_revision=ff9859b3ea0888c2469bdc0cafeddd1076a2e498
zed_revision=cff3ac6f93f506330034652f0d2389591bfb45a0
mkdir -p vendor .tools
if [[ ! -e vendor/gpui-ghostty/.git ]]; then
  git clone --no-checkout "$component_repository" vendor/gpui-ghostty
  git -C vendor/gpui-ghostty checkout --detach "$component_revision"
fi
if [[ "$(git -C vendor/gpui-ghostty rev-parse HEAD)" != "$component_revision" ]]; then
  printf 'Unexpected gpui-ghostty revision; inspect vendor/gpui-ghostty before proceeding.\n' >&2
  exit 1
fi
# The maintained fork baseline predates the local VT 1.4 adapter. Keep the
# upstream pin explicit instead of resetting it to the baseline's gitlink.
ghostty_dir=vendor/gpui-ghostty/vendor/ghostty
ghostty_revision=83fb95624b0439ee660c2679b43b38c8b63769cd
if [[ ! -e "$ghostty_dir/.git" ]]; then
  git -C vendor/gpui-ghostty submodule update --init --depth 1 -- vendor/ghostty
fi
if [[ "$(git -C "$ghostty_dir" rev-parse HEAD)" != "$ghostty_revision" ]]; then
  if [[ -n "$(git -C "$ghostty_dir" status --porcelain)" ]]; then
    printf 'Ghostty has local changes; inspect %s before changing its pin.\n' "$ghostty_dir" >&2
    exit 1
  fi
  git -C "$ghostty_dir" fetch --depth 1 https://github.com/ghostty-org/ghostty.git tag v1.4.0
  if [[ "$(git -C "$ghostty_dir" rev-parse 'v1.4.0^{commit}')" != "$ghostty_revision" ]]; then
    printf 'Unexpected Ghostty v1.4.0 revision.\n' >&2
    exit 1
  fi
  git -C "$ghostty_dir" checkout --detach "$ghostty_revision"
fi

if [[ ! -f vendor/zed/crates/gpui/Cargo.toml ]]; then
  download_dir="$(mktemp -d "${TMPDIR:-/tmp}/terminal-zed.XXXXXX")"
  curl -fL --retry 2 "https://codeload.github.com/zed-industries/zed/tar.gz/$zed_revision" -o "$download_dir/zed.tar.gz"
  mkdir vendor/zed
  tar -xf "$download_dir/zed.tar.gz" -C vendor/zed --strip-components=1
fi
if [[ ! -x .tools/zig-0.17.0/zig ]]; then
  if [[ -e .tools/zig-0.17.0 ]]; then
    printf 'Incomplete Zig installation; inspect .tools/zig-0.17.0 before retrying.\n' >&2
    exit 1
  fi
  download_dir="$(mktemp -d "${TMPDIR:-/tmp}/terminal-zig-download.XXXXXX")"
  extract_dir="$(mktemp -d "$project_dir/.tools/zig-extract.XXXXXX")"
  curl -fL --retry 2 https://ziglang.org/download/0.17.0/zig-aarch64-macos-0.17.0.tar.xz -o "$download_dir/zig.tar.xz"
  printf '%s  %s\n' b607e9b9234790a008116ae5bdb71c6243b84b9fb42a53a9e70fde41c06c536a "$download_dir/zig.tar.xz" | shasum -a 256 -c -
  tar -xf "$download_dir/zig.tar.xz" -C "$extract_dir" --strip-components=1
  mv "$extract_dir" .tools/zig-0.17.0
fi
python3 -I "$project_dir/scripts/prepare-vendor.py" "$project_dir"
printf '\nDependencies ready. Run ./scripts/run.sh\n'
