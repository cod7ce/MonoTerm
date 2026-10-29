# Herdr client libraries

MonoTerm vendors `crates/herdr-client` and `crates/herdr-protocol`, and adapts selected native UI source from
[penso/herdr-gpui](https://github.com/penso/herdr-gpui) at commit
`38ea4248d2c64879f02053b419bdd5386eebce10`.

The crates are stored here without their original `crates/` prefix. Their Cargo
manifests resolve dependencies locally instead of inheriting the upstream
workspace configuration. Native UI adaptations live in MonoTerm's `src/` and
use its GPUI 0.2.2, shared palette, connection fences and local companion owner.
They do not import the full `herdr-gpui` application or upgrade GPUI. No daemon,
remote management, updater or browser implementation is included.

The native header in `src/herdr_chrome.rs` adapts
`crates/herdr-gpui/src/titlebar/{tabs,navigation}.rs`, `tab_layout/classic.rs`
and the classic new-tab control in `tab_layout.rs`: a 34px merged tab/title row,
80px native-control clearance, segmented Back/Forward, flat ruled tabs and
functional new/close controls. `src/herdr_navigation.rs` supplies MonoTerm's
bounded acknowledged-focus history. Only daemon terminal tabs are shown;
browser/code/review tabs belong to the upstream application.

`src/herdr_sidebar.rs` and `src/herdr_sidebar_rows.rs` adapt
`crates/herdr-gpui/src/sidebar/{row,cell,metrics,agents,layout,render}.rs`,
`sidebar/layouts/herdr.rs` and `sidebar/render/agent_rows.rs`. The subset retains
flat workspace rows, branch/git details, left status marks, fairly budgeted
workspace/tab/pane labels and plugin agent order. It uses a 232px sidebar with
independent scroll areas; agents take their content height up to half the sidebar.
GPUI text measurement replaces upstream Unicode-width helpers. Shared MonoTerm
font/palette preferences remain authoritative. Arbitrary configured sidebar rows,
custom token colors and repository icons are not imported; the snapshot provides
no structured version of those GUI-owned settings. Unicode in daemon labels is
preserved. `new` creates a daemon workspace; `menu` opens MonoTerm Settings.

MonoTerm's `assets/agent-claude.svg` and `assets/agent-codex.svg` reuse the
upstream `assets/icons/` SVGs unchanged. These LobeHub marks identify Claude
and Codex (OpenAI); their MIT attribution is included in `NOTICE`, which is
also shipped in the application bundle.

The original root `LICENSE` and `NOTICE` are preserved here. `NOTICE` contains
attributions for the complete upstream project; references to other upstream
crates and assets do not imply those components are included in MonoTerm. The
protocol crate's `NOTICE.md` and `LICENSE-APACHE` retain its additional provenance
from `herdrdev/herdr`.

MonoTerm's connection ownership, reconnect policy, input gating and response
fences are implemented separately in `src/herdr_connection.rs`. The vendored
library source is unchanged.

Build and test MonoTerm from its root manifest. Cargo builds these crates as
normal dependencies; their own `cfg(test)` modules are not enabled by MonoTerm's
unit tests. Running the vendored client's standalone unit tests currently needs
upstream `crates/test-support/executable.rs`, which its source references as
`../../test-support/executable.rs`. That support directory is outside the two
vendored crates and is intentionally not copied. No source-path workaround is
needed for MonoTerm's normal build or tests. Live daemon tests remain opt-in and
must not be used against a user's default daemon as part of routine verification.
