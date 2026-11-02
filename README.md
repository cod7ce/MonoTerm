# MonoTerm

A native macOS terminal built with Rust, GPUI and Ghostty. Each local pane owns a real PTY; Ghostty handles terminal state and GPUI renders the terminal and application.

**Current version: 0.3.0.** Apple Silicon, macOS 14 or later.

## Features

- A compact native window with a resizable tab sidebar, recursive split panes and Zen mode.
- Unicode text, ANSI styles, scrollback, mouse reporting and bracketed paste.
- Copy on selection, Shift selection in mouse-reporting applications, and reusable copy feedback.
- Ghostty 1.4 Kitty graphics: direct PNG/raw pixels and local files, crops, relative placements, Unicode placeholders, layering, scrolling, animation and deletion.
- Shared Light/Dark appearance, accent colors, transparency, native blur and font preferences.
- English and Simplified Chinese interfaces.
- An optional native Herdr client with grouped repository/worktree Spaces, agent activity indicators, named tabs, and Spaces/Agents sidebar views.
- Space menus for renaming, creating/opening worktrees, copying paths/branches, closing groups, and confirmed linked-checkout removal.
- Companion shells that follow verified local agents, with shared window styling and native Chinese input in the Herdr terminal.
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
| Previous / next pane | Cmd-[ / Cmd-] (also Cmd-Alt-Left / Cmd-Alt-Right) |
| Select tab 1–9 | Cmd-1 … Cmd-9 |
| Previous / next agent in Herdr | Cmd-Shift-[ / Cmd-Shift-] |
| Customize tab | Cmd-Shift-E, or its context menu |
| Toggle sidebar | Cmd-B |
| Toggle Zen | Cmd-Shift-Z |
| Open Settings | Cmd-comma |
| Copy / paste / select all | Cmd-C / Cmd-V / Cmd-A |
| Increase / decrease / reset font size | Cmd-plus / Cmd-minus / Cmd-0 |
| Restart active shell | Cmd-Shift-R |
| Quit | Cmd-Q |

Shared shortcuts apply in main and Herdr windows. Zen, shell restart and tab customization are main-window actions. In Herdr, indexed tabs refer to the current Space; an absent index does nothing. Pane/tab cycling wraps, and agent cycling follows the Agents list's ordering and filter even when the sidebar is hidden. The native Herdr close button detaches; Cmd-W closes its focused daemon pane and Cmd-Shift-W closes its focused daemon tab.

Local limits are 12 tabs, 8 panes per tab and 48 panes total. Closing a pane terminates its shell; closing a tab terminates all its panes. Tabs and pane layouts are not restored after quitting. Preferences are stored under `~/Library/Application Support/MonoTerm/`; `MONOTERM_CONFIG_DIR` selects an isolated test directory.

Font family and size are shared and saved automatically (Menlo, 14px by default; 10–28px). Settings → Terminal searches installed fonts, initially filters to monospace families, and previews English, CJK, code, Nerd symbols and emoji. Show all fonts includes proportional families, which still occupy terminal grid cells. Missing fonts temporarily use Menlo while retaining the chosen name. Install/uninstall font changes are detected after restarting MonoTerm. Settings itself keeps the system font.

`windows.json` keeps independent main/Herdr sidebar visibility and width, main Zen mode, Herdr section choices and split ratio, and repository folding. Window-size auto-hide preserves the explicit sidebar preference. Drag sizes save on release. `typography.json` keeps family and size and accepts earlier font-enum values; existing appearance, language and integration files remain compatible.

Cmd-hover underlines HTTP/HTTPS links and shows their destination; Cmd-click opens the system default browser. Right-click offers Copy link address; use Shift-right-click when the terminal application captures mouse input. OSC 8 destinations take priority over visible text; plain URLs support localhost and wrapped rows. Plain text without wrap metadata uses contiguous full-width rows, so an exact-edge hard newline is ambiguous. Normal selection is preserved, and a drag or changed frame cancels link activation.

## Herdr integration

The Herdr button enables local companion following. Settings → Integrations controls the source socket and whether a separate Herdr window opens automatically. The Herdr window paints the daemon's cell surface; local tabs remain real MonoTerm PTYs.

Main, Herdr and Settings windows close independently. Closing the main window terminates its local shells while Herdr keeps its connection, input, output and preferences. Closing Herdr detaches its surface without stopping daemon-owned terminals or local shells. View → Return to local terminals reopens the main window when needed. Disabling follow affects only local companion following; explicitly opening Herdr does not enable it. Source changes reconnect the existing Herdr window in place. Closing the last native window exits MonoTerm, and Cmd-Q exits the whole application.

Following respects manual tab selection, verified process identities, idle-shell checks and connection/input fences. Clicking panes or resizing splits does not pin following. Custom tab titles and icons belong to the tab until it closes.

The separate window groups linked worktrees under their repository, shows branch and ahead/behind counts, and offers independently scrollable Spaces and Agents sections. Use the sidebar chevron to show either section or both; right-click a Space for workspace and worktree actions. Removing a linked checkout requires confirmation, and a dirty checkout requires a separate force-removal confirmation. Closing a Space or group keeps its checkout files.

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

Kitty supports direct image payloads and local regular-file transport, including the default PNG path used by Snacks in Neovim. Temporary-file and shared-memory transports are not enabled. Snacks may still require ImageMagick for image metadata and format conversion. Image snapshots and visible textures each have a 64 MiB cache budget, separate from decoder and upstream storage limits. This is not a process-wide memory cap.

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

The 0.1.1 release passed 207 application/component tests (one opt-in ignored), strict all-target Clippy, formatting, eight native updater scenarios and Ed25519 signature tests. Focused native checks passed for Neovim local-file images and input scrolling (15), the original-position translucent Zen indicator (25), and viewport cursor visibility (14); all corresponding screenshots were inspected. Option-key GUI behavior remains unverified.

The 0.2.0 release passed 231 application/component tests (one opt-in ignored), strict all-target/all-feature Clippy, formatting, eight native updater scenarios and Ed25519 signature tests. Focused native checks covered the worktree menu (33), sidebar views/tabs (22), WeType composition through metadata updates (12), Dark/Light sidebar colors and spacing (13 each), and inline branch counts at wide/narrow sidebar widths (11); the corresponding screenshots were inspected. The complete live-agent follow regression remains incomplete after a Codex bootstrap timeout. Cross-version download/install/relaunch is not claimed as verified.

The 0.3.0 release passed 246 application/component tests (one opt-in ignored), strict all-target/all-feature Clippy, formatting, eight native updater scenarios, and Ed25519 signature tests. The component patch reproduces all 20 modified source files exactly. Focused native checks covered local shortcuts/preferences (37), Herdr shortcuts/layout (24), Herdr link actions (18), Space rename typing and Pinyin (14), and independent window lifecycles (27); key screenshots were inspected. Local links were also checked through an actual browser request. Full cross-version update installation and system-theme switching remain unverified.

Native GUI checks require a logged-in, unlocked macOS desktop and Accessibility/Screen Recording access. Use separate test-owned windows and configurations. Never restart an existing user terminal as part of verification. Existing upstream Rust future-compatibility warnings may still appear.

## Scope

MonoTerm is an early local macOS application. Session restoration, search, integrated SSH, per-session palettes and distribution notarization are not implemented. Herdr Chinese composition has focused native WeType coverage, including metadata refreshes and commit/cancel; other input methods and the complete live-agent follow workflow need further dedicated coverage. The application can close local jobs when a pane, tab or the app is closed.

## Third-party notices

- [gpui-ghostty](https://github.com/cod7ce/gpui-ghostty): Apache-2.0.
- [GPUI](https://github.com/zed-industries/zed): Apache-2.0; supporting Zed crates retain their own notices.
- [Ghostty](https://github.com/ghostty-org/ghostty): MIT.
- [portable-pty](https://github.com/wezterm/wezterm): MIT.
- [Herdr libraries and adapted UI](vendor/herdr/README.md): Apache-2.0, with preserved upstream notices and LobeHub icon attribution.
- [Sparkle](https://github.com/sparkle-project/Sparkle): its upstream license is included in the application bundle.

Herdr and agent artwork identifies the corresponding integrations and remains subject to its original attribution. The updater design and implementation are adapted from the user's [FourQuadrants-GPUI](https://github.com/cod7ce/FourQuadrants-GPUI) project, with a separate application identity, feed and signing key. Third-party licenses remain with vendored source and packaged resources.
