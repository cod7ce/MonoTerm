# MonoTerm

A native macOS terminal built with Rust, GPUI and Ghostty. Each local pane owns a real PTY; Ghostty handles terminal state and GPUI renders the terminal and application.

**Current version: 0.1.0.** Apple Silicon, macOS 14 or later.

## Features

- A compact native window with a resizable tab sidebar, recursive split panes and Zen mode.
- Unicode text, ANSI styles, scrollback, mouse reporting and bracketed paste.
- Copy on selection, Shift selection in mouse-reporting applications, and reusable copy feedback.
- Ghostty 1.4 Kitty graphics: direct PNG/raw pixels, crops, relative placements, Unicode placeholders, layering, scrolling, animation and deletion.
- Shared Light/Dark appearance, accent colors, transparency, native blur and font preferences.
- English and Simplified Chinese interfaces.
- An optional native Herdr client and companion shells that follow verified local agents.
- A **Version** settings tab with current version, manual update checks and release downloads. Sparkle verifies signed update packages; automatic checking and background downloads are disabled.

## Download and updates

Download application packages from [Releases](https://github.com/cod7ce/MonoTerm/releases).
Open **Settings → Version** to check for updates or view release downloads.
Installing an update restarts MonoTerm and closes local terminal sessions, so finish running jobs first. Herdr-owned sessions continue running independently.

Development windows launched with `MONOTERM_CONFIG_DIR` disable the updater. An unpackaged executable or a bundle missing update configuration reports updates as unavailable. See [Releasing](docs/RELEASING.md) for signing and packaging instructions.

## Build and run

Requires Apple Silicon macOS, Rust/Cargo, Git, curl, Python 3.11 or later and Xcode command-line tools.

```sh
./scripts/bootstrap.sh
bash scripts/build-app.sh
open target/MonoTerm.app
```

To start in a specific working directory:

```sh
./scripts/run.sh ~/Workspace
```

Bootstrap downloads pinned dependencies and Zig into this checkout. It applies the maintained [terminal patch](patches/README.md) without discarding existing local changes. Build output and tools remain under `target/` and `.tools/`; nothing is installed globally. The first build can take several minutes. Sparkle is fetched from its official release with a pinned checksum.

The app bundle contains its runtime resources and framework. Cargo, Zig and the source tree are not needed to run a packaged application. Locally signed builds use ad-hoc signing; this project does not currently provide Apple notarization.

## Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| New tab | Cmd-T |
| Close active pane | Cmd-W |
| Close active tab | Cmd-Shift-W |
| Next / previous tab | Ctrl-Tab / Ctrl-Shift-Tab |
| Split right / below | Cmd-D / Cmd-Shift-D |
| Next / previous pane | Cmd-Alt-Right / Cmd-Alt-Left |
| Customize tab | Cmd-Shift-E, or its context menu |
| Toggle sidebar | Cmd-B |
| Toggle Zen | Cmd-Shift-Z |
| Open Settings | Cmd-comma |
| Copy / paste / select all | Cmd-C / Cmd-V / Cmd-A |
| Increase / decrease / reset font size | Cmd-plus / Cmd-minus / Cmd-0 |
| Restart active shell | Cmd-Shift-R |
| Quit | Cmd-Q |

Limits are 12 tabs, 8 panes per tab and 48 panes total. Closing a pane terminates its shell; closing a tab terminates all its local panes. Tabs and pane layouts are not restored after quitting. Preferences are stored under `~/Library/Application Support/MonoTerm/`; `MONOTERM_CONFIG_DIR` selects an isolated test directory.

## Herdr integration

The Herdr button enables local companion following. Settings → Integrations controls the source socket and whether a separate Herdr window opens automatically. The child window paints the daemon's cell surface and leaves daemon-owned terminals running when detached. Local tabs remain real MonoTerm PTYs.

Following respects manual tab selection, verified process identities, idle-shell checks and connection/input fences. Clicking panes or resizing splits does not pin following. Custom tab titles and icons belong to the tab until it closes.

The separate Herdr renderer currently does not display terminal graphics, support integrated SSH transport or drag daemon split dividers. This does not limit Kitty graphics in local MonoTerm PTYs. Adapted libraries and source provenance are documented in [vendor/herdr](vendor/herdr/README.md).

## Dependency pins

| Dependency | Baseline |
| --- | --- |
| gpui-ghostty fork | `ff9859b3ea0888c2469bdc0cafeddd1076a2e498` plus the checked-in patch |
| Ghostty | v1.4.0, `83fb95624b0439ee660c2679b43b38c8b63769cd` |
| Zed / GPUI | `cff3ac6f93f506330034652f0d2389591bfb45a0` |
| Herdr client / protocol | `38ea4248d2c64879f02053b419bdd5386eebce10` |
| Zig | 0.17.0 |
| Sparkle | 2.10.0 |
| Rust packages | `Cargo.lock` |

Ghostty's VT C API is provisional: keep the exact source pin and adapter together. GPUI uses runtime Metal shaders. Menlo is the default font; Nerd Font symbols use installed fallback families, which bootstrap does not install or bundle.

Kitty supports direct image payloads; file, temporary-file and shared-memory transports are not enabled. Image snapshots and visible textures each have a 64 MiB cache budget, separate from decoder and upstream storage limits. This is not a process-wide memory cap.

## Validation

```sh
cargo fmt --package monoterm -- --check
cargo test --locked -p monoterm -p ghostty_vt -p ghostty_vt_sys -p gpui_ghostty_terminal
cargo clippy --locked -p monoterm -p ghostty_vt -p ghostty_vt_sys -p gpui_ghostty_terminal --all-targets -- -D warnings
bash scripts/test-updater.sh
xcrun swift scripts/update-key.swift self-test
```

The Ghostty 1.4 baseline passed 202 application/component tests and a focused 37-check native graphics flow, with all 12 screenshots inspected. That flow covers image crops/layers/placements, Light appearance, resize, animation without PTY output, stop/delete, Chinese selection and Vim screen restoration. The opt-in real Neovim query test passed separately. Native updater fixtures cover manual-only policy, isolated-window disablement and cancellation/retry around actual AppKit termination; they do not establish a cross-version installation by themselves.

The Version settings revision passed 118 application tests (one opt-in test ignored), strict application Clippy and formatting, eight isolated native updater scenarios, and a 22-check Settings flow. All seven Settings/terminal screenshots were inspected for English/Chinese rendering, Light/Dark contrast, minimum-width controls and retained terminal input. The release archive passed ZIP integrity, SHA256, bundled metadata, nested code-signature and Ed25519 verification. These checks do not establish a cross-version download/install/relaunch.

Native GUI checks require a logged-in, unlocked macOS desktop and Accessibility/Screen Recording access. Use separate test-owned windows and configurations. Never restart an existing user terminal as part of verification. Existing upstream Rust future-compatibility warnings may still appear.

## Scope

MonoTerm is an early local macOS application. Session restoration, search, integrated SSH, per-session palettes and distribution notarization are not implemented. Native IME behavior and the complete live-agent follow workflow need further dedicated coverage. The application can close local jobs when a pane, tab or the app is closed.

## Third-party notices

- [gpui-ghostty](https://github.com/cod7ce/gpui-ghostty): Apache-2.0.
- [GPUI](https://github.com/zed-industries/zed): Apache-2.0; supporting Zed crates retain their own notices.
- [Ghostty](https://github.com/ghostty-org/ghostty): MIT.
- [portable-pty](https://github.com/wezterm/wezterm): MIT.
- [Herdr libraries and adapted UI](vendor/herdr/README.md): Apache-2.0, with preserved upstream notices and LobeHub icon attribution.
- [Sparkle](https://github.com/sparkle-project/Sparkle): its upstream license is included in the application bundle.

Herdr and agent artwork identifies the corresponding integrations and remains subject to its original attribution. The updater design and implementation are adapted from the user's [FourQuadrants-GPUI](https://github.com/cod7ce/FourQuadrants-GPUI) project, with a separate application identity, feed and signing key. Third-party licenses remain with vendored source and packaged resources.
