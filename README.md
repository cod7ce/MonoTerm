# MonoTerm · ONES

A native macOS terminal built with Rust, GPUI and Ghostty. Each local pane owns a real PTY; Ghostty handles terminal state and GPUI renders the terminal and application.

**当前版本：0.3.5。** 适用于 Apple Silicon Mac，要求 macOS 14 或更高版本。

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

在应用菜单中选择 **MonoTerm · ONES → 关于 MonoTerm · ONES**，可查看应用简介、当前版本、运行平台和构建技术，并打开 GitHub 项目或版本下载页面。版本号直接读取构建元数据，与发布版本保持一致；关于窗口支持中英文界面以及浅色、深色主题。

Download application packages from [Releases](https://github.com/cod7ce/MonoTerm/releases).
Open **Settings → Version** to check for updates or view release downloads.
Installing an update restarts MonoTerm and closes local terminal sessions, so finish running jobs first. Herdr-owned sessions continue running independently.

Development windows launched with `MONOTERM_CONFIG_DIR` disable the updater. An unpackaged executable or a bundle missing update configuration reports updates as unavailable. See [Releasing](docs/RELEASING.md) for signing and packaging instructions.

## Build and run

Requires Apple Silicon macOS, Rust/Cargo, Git, curl, Python 3.11 or later and Xcode command-line tools.

```sh
./scripts/bootstrap.sh
bash scripts/build-app.sh
open 'target/MonoTerm · ONES.app'
```

To start in a specific working directory:

```sh
./scripts/run.sh ~/Workspace
```

Bootstrap downloads pinned dependencies and Zig into this checkout. It applies the maintained [terminal patch](patches/README.md) without discarding existing local changes. Build output and tools remain under `target/` and `.tools/`; nothing is installed globally. The first build can take several minutes. Sparkle is fetched from its official release with a pinned checksum.

应用名称为 **MonoTerm · ONES**，开发版和发布版分别输出到 `target/MonoTerm · ONES.app`、`dist/MonoTerm · ONES.app`，包内可执行文件也使用此名称。配置目录仍为 `~/Library/Application Support/MonoTerm/`，应用标识、更新源及签名密钥延续现有值，已有偏好无需迁移。仓库名、Cargo 包名和 `MONOTERM_*` 环境变量保持兼容；历史 Release 的文件名保持发布时的名称。

The app bundle contains its runtime resources and framework. Cargo, Zig and the source tree are not needed to run a packaged application. Locally signed builds use ad-hoc signing; this project does not currently provide Apple notarization.

## Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| New tab | Cmd-T |
| 关闭当前窗口 | Cmd-W |
| Close active tab | Cmd-Shift-W |
| Next / previous tab | Ctrl-Tab / Ctrl-Shift-Tab |
| Split right / below | Cmd-D / Cmd-Shift-D |
| Previous / next pane | Cmd-[ / Cmd-] (also Cmd-Alt-Left / Cmd-Alt-Right) |
| Maximize / restore active pane | Cmd-Shift-Enter |
| Select tab 1–9 | Cmd-1 … Cmd-9 |
| Previous / next agent in Herdr | Cmd-Shift-[ / Cmd-Shift-] |
| Customize tab | Cmd-Shift-E, or its context menu |
| Toggle sidebar | Cmd-B |
| Toggle Zen | Cmd-Shift-Z |
| Open Settings | Cmd-comma |
| Copy / paste / select all | Cmd-C / Cmd-V / Cmd-A |
| 清空当前终端窗格及历史记录 | Cmd-K |
| Increase / decrease / reset font size | Cmd-plus / Cmd-minus / Cmd-0 |
| Restart active shell | Cmd-Shift-R |
| Quit | Cmd-Q |

共享快捷键适用于主窗口和 Herdr 窗口。禅模式、重启 shell 和自定义标签页图标仅适用于主窗口。Cmd-Shift-E 打开本地标签页自定义或 Herdr 标签页重命名；Herdr 标签页也支持双击或右键菜单重命名。Herdr 的数字快捷键选择当前 Space 中的标签页，索引不存在时不执行操作。窗格、标签页和 agent 切换会循环；agent 切换遵循 Agents 列表的顺序和筛选条件，即使侧栏隐藏也一样。关闭按钮和 Cmd-W 均只关闭当前窗口；关闭 Herdr 窗口会解除界面连接，不关闭守护进程中的窗格。Cmd-Shift-W 仍关闭当前标签页；单独关闭窗格可使用窗格关闭按钮或“视图 → 关闭窗格”。

Cmd-Shift-Enter maximizes the active pane within its tab in either window; pressing it again restores the split layout. A single-pane tab is unchanged. A dashed frame around the tab icon identifies this state and shows the restore shortcut on hover. Local zoom keeps the original split tree, ratios and running shells; cycling panes changes the maximized pane, splitting restores the layout, and closing the maximized pane reveals its siblings. Herdr uses the daemon's pane zoom state.

The mouse wheel scrolls the pane under the pointer in either window; its magnitude follows the system gesture, including partial rows. In Herdr, wheel input goes to that daemon pane without moving keyboard focus, and the tab strip reveals the tab for the selected agent.

Double-click or right-click a pane's title to rename that pane in either window. Edit → Rename pane also works when the title is hidden. Pane names are independent of tab names; leaving the field blank restores the automatic name. Local pane names last for the current session; Herdr names are stored by its daemon. Drag a divider to adjust split proportions in either window. Herdr coalesces motion while waiting for acknowledgements and stops the drag if its tab, pane tree, popup state or window geometry changes.

Local limits are 12 tabs, 8 panes per tab and 48 panes total. Closing a pane terminates its shell; closing a tab terminates all its panes. Tabs and pane layouts are not restored after quitting. Preferences are stored under `~/Library/Application Support/MonoTerm/`; `MONOTERM_CONFIG_DIR` selects an isolated test directory.

“设置 → 终端 → 历史记录行数”可以配置本地终端的回滚历史，默认 10000 行，设为 0 时不限制。点击“应用”或在输入框中按回车后立即生效，并保存到 `terminal.json`；现有窗格、新建窗格和重启后的本地终端均使用该设置。Ghostty 按内存页回收历史，因此实际保留行数会在设定值附近浮动；减小上限时，较早的历史可能立即被移除。Herdr 历史上限由其守护进程管理。

“设置 → 终端 → 打开终端的默认目录”支持绝对路径或 `~/` 开头的路径，留空时使用 home 目录。点击“应用”或按回车后保存到 `terminal.json`，之后新建的标签页和下次启动的终端使用该目录；不会改变已有 shell 的目录。启动时显式传入的目录参数优先于设置，已保存目录被删除或不可用时回退到 home 并显示提示。

新建本地标签页始终使用设置中的默认目录，未设置时使用 home。新建分屏则读取当前活动 shell 的实际工作目录。例如在 home 中执行 `cd ABC` 后，新建分屏会打开 ABC，新建标签页仍打开默认目录。分屏读取目录失败时优先使用该会话的启动目录，仍不可用则使用默认目录或 home。Herdr 窗口中的远端窗格由守护进程管理。

`⌘K` 或“编辑 → 清空终端”清空当前活动窗格的画面和回滚历史，不重启 shell，也不影响其他窗格。Herdr 窗口通过守护进程的 `pane.clear` 执行同一操作，需要所连接的守护进程支持此接口。

Font family and size are shared and saved automatically (Menlo, 14px by default; 10–28px). Settings → Terminal searches installed fonts, initially filters to monospace families, and previews English, CJK, code, Nerd symbols and emoji. Show all fonts includes proportional families, which still occupy terminal grid cells. Missing fonts temporarily use Menlo while retaining the chosen name. Install/uninstall font changes are detected after restarting MonoTerm. Settings itself keeps the system font.

`windows.json` keeps independent main/Herdr sidebar visibility and width, main Zen mode, Herdr section choices and split ratio, and repository folding. Window-size auto-hide preserves the explicit sidebar preference. Drag sizes save on release. `typography.json` keeps family and size and accepts earlier font-enum values; existing appearance, language and integration files remain compatible.

Cmd-hover underlines HTTP/HTTPS links and shows their destination; Cmd-click opens the system default browser. Right-click offers Copy link address; use Shift-right-click when the terminal application captures mouse input. OSC 8 destinations take priority over visible text; plain URLs support localhost and wrapped rows. Plain text without wrap metadata uses contiguous full-width rows, so an exact-edge hard newline is ambiguous. Normal selection is preserved, and a drag or changed frame cancels link activation.

## Herdr integration

The Herdr button enables local companion following. Settings → Integrations controls the source socket and whether a separate Herdr window opens automatically. The Herdr window paints the daemon's cell surface; local tabs remain real MonoTerm PTYs.

主窗口、Herdr 窗口和设置窗口可以独立关闭。关闭主窗口会终止其中的本地 shell，Herdr 的连接、输入、输出和偏好设置不受影响。关闭 Herdr 窗口只解除界面连接，不会停止守护进程中的终端或本地 shell。“视图 → 返回本地终端”可重新打开主窗口。禁用跟随只影响本地伴随终端；手动打开 Herdr 窗口不会启用跟随。切换连接来源会在现有 Herdr 窗口中重新连接。关闭最后一个窗口后，MonoTerm · ONES 继续运行；点击 Dock 图标或选择“新建标签页”可重新打开主窗口。Cmd-Q 或“退出 MonoTerm · ONES”才会退出整个应用。

Following respects manual tab selection, verified process identities, idle-shell checks and connection/input fences. Clicking panes or resizing splits does not pin following. Custom tab titles and icons belong to the tab until it closes.

The separate window groups linked worktrees under their repository, shows branch and ahead/behind counts, and offers independently scrollable Spaces and Agents sections. Use the sidebar chevron to show either section or both; right-click a Space for workspace and worktree actions. Removing a linked checkout requires confirmation, and a dirty checkout requires a separate force-removal confirmation. Closing a Space or group keeps its checkout files.

The separate Herdr renderer currently does not display terminal graphics or support integrated SSH transport. This does not limit Kitty graphics in local MonoTerm PTYs. Adapted libraries and source provenance are documented in [vendor/herdr](vendor/herdr/README.md).

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

The 0.3.1 release passed 265 application/component tests (one opt-in ignored), strict all-target/all-feature Clippy, formatting, updater lifecycle fixtures, and a signed release build with Ed25519 verification. Focused changes cover Herdr tab styling and renaming, pane zoom in both windows, pointer-directed wheel routing, gesture-scaled scrolling, and revealing the selected agent's tab. Native GUI checks for these changes remain unverified because Accessibility and Screen Recording access were unavailable; cross-version update installation and notarization are not claimed.

The 0.3.2 release passed 273 application/component tests (one opt-in ignored), strict all-target/all-feature Clippy, formatting, eight updater lifecycle scenarios, and Ed25519 signature tests. The signed release archive passed ZIP integrity, SHA256, appcast metadata, and extracted-bundle signature verification. This release adds independent pane names in both windows, Herdr split-divider dragging, and window-level local resize handling. Native pane editor and divider interactions remain unverified because Accessibility access was unavailable; full cross-version update installation and notarization are not claimed.

0.3.3 发布前通过了 273 项应用及组件测试（另有 1 项需手动启用的测试未运行）、全目标及全特性的严格 Clippy、格式检查、8 项更新器生命周期检查和 Ed25519 签名自测。独立测试实例通过 33 项原生窗口检查，覆盖关闭按钮、⌘W、⌘Q、窗口重新打开，以及关于页面的单窗口复用、中英文和深浅主题；两种主题的关于页面截图均已检查。尚未验证完整的跨版本下载安装与重启流程，也未进行 Apple 公证。

0.3.4 发布前通过了 282 项应用及组件测试（另有 1 项需手动启用的测试未运行）、全目标及全特性的严格 Clippy、格式检查、8 项更新器生命周期检查和 Ed25519 签名自测。22 项原生检查覆盖历史行数设置、无限历史、即时生效、持久化、清屏后 shell 状态保留和分屏隔离，中英文设置截图均已检查，开发版也已通过手动确认。发布安装包通过 ZIP 完整性、SHA256、版本及更新源元数据、Ed25519 和解压后代码签名验证。Herdr 清屏通过协议层回归测试，尚未连接真实守护进程验证；完整跨版本更新安装与 Apple 公证仍未验证。

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
