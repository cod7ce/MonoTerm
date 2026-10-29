# Herdr Follow Design

Status: first implementation present, 2026-10-24. This document records the design and the implementation notes below; conceptual type names and original line references are not API contracts.

Implemented in `follow.rs`, `herdr.rs`, `process_monitor.rs` and `companion.rs`, with session activity tracking and an Integrations settings category. The adapter supplies only verified local targets; the pure controller uses stable tab IDs, binding revisions, process identities and connection epochs. Source changes detach existing tabs. Splitting also pins follow and marks the tab used. The first UI exposes source selection through Paste path / Use default, retention and Reopen through Settings and the Window menu. Selection is conservatively held for the entire time the primary window is active or Settings is open.

The adapter supports protocol 22 and native local Claude/Codex executables. It prefers the process's native cwd; an agent's internal project selection may differ from OS cwd. A process-query diagnostic prevents automatic retirement. Input callbacks conservatively count terminal protocol replies as use. Used, guarded, uncertain or busy tabs remain retained; there is no delayed close when they later become idle. Live associations are in memory only. GUI verification status and exact evidence are recorded in README rather than implied by this design.

MonoTerm follows the agent selected in one configured Herdr session. The first visit creates a local shell in that agent's directory and shows a short greeting. Later visits restore the same terminal state. Confirmed agent exit can retire an associated terminal, subject to protection of the user's work.

The initial implementation targets macOS, Herdr 0.9.5 / protocol 22, and positively identified local Claude Code and Codex processes. The same terminal state means the same PTYs within the current MonoTerm process; restarting MonoTerm does not restore running shells.

## 1. Product behavior

| Action or condition | Result |
| --- | --- |
| Enable Follow Herdr | Connect to the selected source, synchronize state, and follow its current recognized local agent. |
| Select agent A for the first time | Create one associated MonoTerm tab with one shell, using A's validated directory. Show the greeting once. |
| Select agent B, then A | Select A's existing tab and its last active pane. Preserve every PTY, command, working directory, split and scrollback. |
| Two agents use the same directory | Give them separate associations and tabs. Directory is creation context, not identity. |
| Split an associated tab | New panes belong to that tab's association. Returning to the agent restores the entire layout. |
| Herdr selects a shell or an unsupported/remote agent | Keep the current view. Show a small explanation; do not guess a different agent. |
| MonoTerm is receiving keyboard input, has an open Settings window/folder picker, or has an interaction in progress | Hold the current view. Remember only the latest desired Herdr selection. |
| MonoTerm becomes inactive and interaction guards clear | Synchronize and apply the latest pending selection without activating any window. |
| Manually select another tab, create a manual tab, or open a folder | Pin the user's selection until they explicitly resume following. |
| Pin the current tab | Stop automatic selection. Continue observing the source, but protect the pinned tab from automatic closure. |
| Disable Follow Herdr | Cancel observation and pending automatic actions. Keep existing tabs and dormant association metadata in memory. |
| Agent exits | Confirm the specific process exit and reconcile replacements before considering automatic closure. |
| Close an associated tab manually | Close it locally and suppress automatic recreation for that agent incarnation. Offer an explicit Reopen action. |
| The last associated tab is automatically closed | Keep the app open in a Waiting for an agent state. Do not create a replacement shell. |
| Close the last tab manually | Preserve the current application behavior: quit MonoTerm. |

Enabling Follow does not adopt or dispose of an existing manual tab, even when its directory matches. If Follow is enabled at application startup, use the waiting view until a target is resolved instead of creating an unnecessary initial shell. When disabled, keep the current ordinary-terminal startup behavior.

### Minimal interface

Use a compact control in the existing title bar: `Follow Herdr`, `Following · <agent>`, or `Pinned`. A pending-target indicator explains why selection is held while the user is working. The control offers Resume following and Disable; a tab action offers Keep terminal, which permanently detaches that tab from automatic management.

Add an Integrations category to the existing independent Settings window. It holds the enabled preference, source session/socket selection, and the first-release exit policy. Do not add another sidebar or a separate agent dashboard. In Zen, keep the controls accessible through the application menu without adding persistent terminal chrome.

The greeting is one or two display lines, such as `Companion ready · Claude` followed by the initial directory. Feed sanitized printable text through the terminal display path before draining the shell's first output, never through PTY input. Do not execute `cd`, `echo`, or a shell fragment. A shell startup script may clear the greeting; the tab label remains the durable context indicator. Re-selecting or re-binding a live tab does not print it again.

## 2. Constraints in the current code

- `src/main.rs:53` owns sessions, tabs, selection, and a 16 ms PTY-output poll. Herdr socket I/O and process inspection must not run in that UI loop.
- `src/main.rs:151` creates a shell from the mutable application directory and immediately selects it. Creation needs an explicit directory and selection origin.
- `src/main.rs:165` combines tab selection, terminal focus, title updates and notice clearing. Automatic selection needs a path that does not activate the app or erase connection notices.
- `src/main.rs:213` closes panes only in the active tab. An exit notification may refer to an inactive tab.
- `src/main.rs:239` quits when the last tab is closed. Automatic cleanup must use a distinct last-tab policy.
- `src/main.rs:329` reuses tabs by directory. Follow associations must use a separate lookup.
- `src/session.rs:17` represents one PTY leaf. Its `directory` is the launch directory, not a continuously observed shell cwd.
- `src/session.rs:46` replaces the shell and view on Restart. A pending automatic-close decision must be invalidated when this happens.
- `src/pty.rs:156` synchronously shuts down and waits for a shell. Automatic cleanup must not perform that wait on the GPUI thread.
- `src/layout.rs:179` already provides stable tab IDs and recursive pane layouts. Keep that layout model and associate at tab level.
- `src/settings.rs:39` already provides categories in an independent window. Reuse this settings surface.

The maintained Ghostty renderer and vendor code do not need changes for the core follow behavior.

## 3. Identity and association

Use an internal `BindingId` as the stable identity of one association. The association owns a MonoTerm tab ID, which owns its existing session IDs. Do not use vector positions as asynchronous identifiers.

Three concepts must remain separate:

| Concept | Fields | Purpose |
| --- | --- | --- |
| Source | Stable endpoint ID and configured socket path | Keep identities from different Herdr sessions separate. |
| Logical agent | Native session reference `{source, agent, kind, value}`, when available | Preserve context across selections and recognize an unambiguous resume. |
| Observed incarnation | Source, `terminal_id`, agent kind, validated process identity, local incarnation generation | Distinguish concurrent sessions, restarts, and delayed exit signals. |

A native session UUID is not a process-liveness signal. It also need not be globally unique among simultaneously running terminals: two processes may resume the same conversation. Store native-session aliases as a set of candidates, not a single global mapping.

Suggested conceptual types, not an implemented API:

```rust
struct Binding {
    id: BindingId,
    source: EndpointId,
    tab: TabId,
    native_session: Option<NativeSessionRef>,
    location: AgentLocation,
    incarnation: AgentIncarnation,
    generation: u64,
    state: BindingState,
    retention: RetentionPolicy,
}

struct ProcessIdentity {
    boot_identity: BootIdentity,
    pid: u32,
    start_time: ProcessStartTime,
}

struct ObservationToken {
    connection_epoch: u64,
    binding: BindingId,
    binding_generation: u64,
    incarnation_generation: u64,
}
```

`AgentLocation` contains terminal, pane, tab and workspace IDs. Pane IDs are location handles: a move may change the pane ID while retaining the terminal and process. `generation` changes whenever binding ownership changes; the incarnation generation also changes on verified process replacement. Async results carry the generations on which they were based.

### Matching order

1. Match the same endpoint, terminal and verified process incarnation. Update location and native-session metadata without creating another tab.
2. If metadata arrives late for the same verified process, add the native-session alias to the existing binding. Temporary disappearance of this metadata does not split the binding.
3. If a pane moves but terminal and process identity remain the same, update location and subscriptions. Preserve the association.
4. A new native session in an occupied terminal is a new logical agent. Do not retarget the old tab merely because the pane or directory matches. Retain or retire the old binding under its own lifecycle rules.
5. For a new process with the same native session, reuse an existing retained tab only when the old incarnation is confirmed dead, the match is unique, no manual-close suppression applies, and the tab has not been detached. Increment generations before installing the new owner. A short replacement grace period can preserve the tab across immediate restarts.
6. Two live processes with the same native-session reference receive separate bindings. If an alias is ambiguous, preserve existing bindings and create a separate one rather than merge shells.
7. Without a native session reference, use a verified process incarnation and terminal identity for in-process reuse. After an exit, a new process receives a new binding. Do not infer continuity from an agent name or path.
8. Without reliable local process/terminal correlation, do not auto-create a local companion. Surface an unsupported/unknown target. Existing companions are retained while the identity is uncertain.

If a new process appears after its old associated tab was already closed, create a fresh shell and greeting. No claim is made that shell state survives destruction of its PTY.

### Manual lifecycle

Keep a suppression record when the user closes or detaches a binding. Repeated focus events and reconciliation snapshots must not recreate it. Suppression lasts for that logical session while it is still observed, including an immediate same-session restart; an explicit Reopen clears it. A different native session is eligible to create a new tab. A detached tab becomes manual and cannot be automatically reclaimed or closed.

If the user exits a companion shell, retain the exited view and offer Restart. Follow selection does not silently respawn it. Restart increments the local session generation and cancels any outstanding close decision. If the source owner is already gone, restart the tab as a detached manual terminal.

## 4. Directory resolution

Resolve a directory only when creating the first companion pane. Prefer the independently verified local agent process cwd, then a locally validated `foreground_cwd`, then `cwd`. Require an absolute, existing directory; use `CommandBuilder::cwd` with a `Path`, never shell interpolation. If no directory can be resolved, retain the current UI and offer a directory choice or retry. Do not silently fall back to HOME.

The existence of the same path on this Mac does not prove that a reported remote path is local. A Unix socket alone also does not prove an agent is local. The first release only creates companions when process identity can be positively correlated to a local Herdr-owned shell/TTY using OS information. SSH/proxy targets and ambiguous correlations are unsupported until a host-aware protocol is available.

After creation, preserve the user's actual shell state. Changing the selected agent's cwd does not inject another `cd` into an existing companion. Label the stored path as the initial directory until separate shell-cwd reporting exists.

## 5. State and selection policy

Keep independent state dimensions rather than one combined enum:

- Mode: `Off`, `Following`, `Pinned`.
- Connection: `Disconnected`, `Connecting`, `Synchronizing`, `Ready`, `Unsupported`.
- Binding: `Attached`, `Uncertain`, `ExitPending`, `RetainedAfterExit`, `Closing`.
- UI selection: `desired_binding`, `displayed_tab`, and an optional latest pending target.

A pure controller accepts observations and user actions and emits effects. It never owns GPUI entities, sockets, or process handles.

```text
Herdr events -> request fresh state -> resolve identity -> controller
                                                        |
                              CreateTab / SelectTab / Inspect / Retain / CloseCandidate
                                                        |
                               UI and background adapters -> result with generation token
```

One pending creation per target is allowed. A rapid A -> B -> A selection burst must not spawn duplicates. Coalesce selection for approximately 100 ms as an initial tuning value, validate the latest target before scheduling a spawn, and tag the result. If a spawn finishes after selection changed, retain the created association without stealing selection. Record failed creation as retryable instead of reopening on every snapshot.

Automatic selection is allowed only when Follow is active and the user-interaction guards are clear. Monitor native window activation through GPUI. An open Settings window, folder picker, terminal selection drag, split drag, or active main window prevents automatic switching and automatic closing of the affected view. On explicit Resume following, apply the latest synchronized target; this is a deliberate user action.

Separate selecting content from focusing controls. Herdr-driven updates must not call `activate_window`, `cx.activate`, AeroSpace, or Herdr focus commands. Internal terminal focus may be prepared when the main window is inactive; activation then gives keyboard input to the displayed pane. No automatic cross-application focus handoff is needed.

Pinning holds selection; detaching removes lifecycle ownership. The pinned tab is protected from auto-close, while other unpinned associated tabs can still be reconciled.

## 6. Herdr client and synchronization

The application uses a small read-only Unix-socket adapter. It does not launch a Herdr TUI, send agent prompts, install integrations, or control Herdr pane lifecycle. The controller's CLI skill requirement to run inside Herdr is not a runtime requirement for the standalone MonoTerm application.

Configure one source initially. Discover existing sessions with the installed CLI only for source selection, or use a configured socket directly. Prefer the conventional default session when no source is chosen; never silently switch to another running session after a disconnect. Support explicit socket selection when the CLI is unavailable. Do not persist inherited caller pane IDs as source selection.

### Connection sequence

1. Open a dedicated event connection and send `events.subscribe`.
2. Wait for `subscription_started`, then open a separate request connection for `session.snapshot`.
3. Buffer invalidations while synchronizing. If relevant events arrive during a query, request another snapshot; apply only generation-current results.
4. Resolve focused target and known bindings from snapshots. Events trigger reconciliation rather than directly assigning ownership or deleting terminals.
5. Resolve process information for new or changed bound owners, then establish local process watchers.

Ordinary requests may close their connection after one response, as observed in testing. Each snapshot or process query must tolerate that behavior.

Initially subscribe to global pane focus, creation, update, move, agent detection, exit and close events, plus tab/workspace focus, move and removal events. Per-pane agent-status subscriptions are unnecessary for initial follow/exit correctness; get status from snapshots. This avoids having to replace the global subscription whenever the live agent set changes.

Protocol 22 uses dotted subscription names and mixed emitted names. In particular:

| Subscription | Observed emitted event |
| --- | --- |
| `pane.agent_detected` | `pane_agent_detected` |
| `pane.closed` | `pane_closed` |
| `pane.exited` | `pane_exited` |
| `pane.agent_status_changed` | `pane.agent_status_changed` |

Use typed deserialization for the subset needed, tolerate unknown fields, and handle unknown events explicitly. Nullable identities and directories are normal. Do not infer process success from `final_status`. Never replay opaque event payloads as commands or terminal output.

### Ordering and recovery

The inspected API exposes no global event replay cursor or atomic snapshot/event revision. A pane's `revision` and an agent's `state_change_seq` are not global stream offsets. Do not claim exactly-once or lossless replay.

Use one serialized reconciliation coordinator, connection epochs, and per-binding generations. Coalesce event bursts for roughly 50 ms with a maximum deferral around 250 ms. Maintain a low-rate snapshot reconciliation, initially every two seconds while enabled, to repair missed metadata updates. These intervals are implementation defaults to measure, not Herdr guarantees.

On disconnect, mark bindings uncertain, retain all PTYs, and cancel automatic-close intents. Retry with bounded backoff, approximately 0.5, 1, 2, 4, then 10 seconds. After reconnect, subscribe first and fully reconcile. Do not treat an empty/failed snapshot or a changed server as proof that agents died. Socket paths are stable configuration, not server-incarnation IDs; old pane IDs alone cannot reattach old bindings after server replacement.

A full bounded queue, oversized frame, malformed required field, or unsupported protocol disables automatic actions and requests a fresh synchronization. Keep newest selection state; never silently discard a process-exit notification and continue as if synchronized. Set reasonable size/time bounds and make diagnostics visible without printing terminal contents or process arguments.

For the first release, protocol 22 is the supported adapter. Other protocol versions retain ordinary terminal use and show an integration compatibility notice rather than attempting destructive lifecycle decisions.

## 7. Confirming source-agent exit

Live tests established that normal Claude Code and Codex exits emit `pane_agent_detected` with `released: true` while their outer shells remain alive. Task completion, turn interruption, and process suspension do not establish death. Explicit pane closure and natural shell exit emit different events and need not be paired.

Treat release, pane closure, pane exit, a changed owner snapshot, or a native process-exit notification as reasons to reconcile. They are not unconditional close commands.

For a supported local agent:

1. Identify the actual executable among process-info members. Claude's process `name` can be its version string, and helpers may precede it. Never choose the first member or assume PGID equals the agent PID.
2. Independently verify UID, executable, process start identity, and relation to the local shell/TTY. A reported PID by itself is insufficient to distinguish local and remote processes or PID reuse. Validate the required OS correlation in an implementation spike; failed correlation yields Unknown.
3. Read start identity, register `kqueue(EVFILT_PROC, NOTE_EXIT)` while the process is alive, and verify identity again. If the process disappeared during registration or identity changed, reconcile; do not transfer an old watcher to a reused PID.
4. Associate the watcher token with the source, binding and incarnation generations. A queued event for an obsolete incarnation cannot close a rebound tab.
5. On actual exit, request fresh source state and allow a short replacement grace, initially one second. Rebind a unique supported same-session replacement if found. Otherwise retire the old owner and assess the companion.

An absent process in `foreground_processes` means only that it is not currently reported in the foreground group. Preserve an existing watcher across suspension/backgrounding. Signal delivery, permission errors, missing metadata, remote targets and API disconnection are not confirmation.

On reconnection, a previously verified process identity that is now conclusively absent can establish old-incarnation death after the source has synchronized. Ambiguous process inspection remains Unknown. Native watchers may record death while disconnected, but automatic closing waits for source recovery so a replacement is not missed.

## 8. Automatic-close policy

The requested behavior is optional and must protect terminal work. The first release exposes `Keep terminals` and `Close unused terminals` as exit policies, with the latter as the proposed default when Follow is enabled.

A companion tab is eligible for automatic closure only when all conditions hold:

- The bound source incarnation has confirmed exit and no valid replacement owns the binding.
- The source is synchronized; binding and local session generations still match the close intent.
- The tab is associated, not manually detached, pinned, or protected by an active interaction.
- Every pane is unused since creation: no delivered terminal input, local Restart, or explicit layout modification has occurred. Mark input conservatively before dispatch; failed delivery may still mark the tab used. Protocol response traffic may conservatively count as use until it can be distinguished without changing the renderer.
- A fresh local inspection for every companion PTY finds the original shell foreground and no other live/stopped jobs or child/TTY processes. Unavailable or ambiguous inspection means retain. Already exited companion shells with preserved output are retained rather than silently discarded.
- No user action or input has invalidated the close intent since inspection.

Use an activity epoch per session and a binding generation in each close intent. Recheck on the UI thread immediately before removing the tab. Reject stale decisions. Avoid any filesystem/process/network waiting on that thread; detach the view and hand owned PTY shutdown to a background task after the close has been committed. Report shutdown failures.

Process inspection is a conservative, best-effort safeguard, not an atomic proof that a shell cannot launch an asynchronous startup job. Initial and final inspection must be fresh and close together. Never label an observation failure as Idle. The default retention behavior limits the effect of this unavoidable observation race.

Once a tab has been used, a first-release agent exit produces `RetainedAfterExit` and keeps its PTYs and contents. The UI offers Close and Keep terminal. Do not kill it later merely because a foreground job finishes. A unique immediate same-session resume may rebind the retained tab unless the user detached it.

### Why not close every apparently idle shell?

The current app knows shell liveness and PTY bytes, but not whether the shell is at a prompt with an empty editable command line. `foreground_pgid == shell_pid` also holds while the user has half a command typed, and background jobs can coexist with a prompt. Output silence is not an idle signal.

A later `Close when safely idle` policy requires session-scoped shell integration reporting prompt/pre-command state, editable-input activity, and jobs, with fallback to retention. That integration must not modify the user's global shell configuration. Until implemented and verified, the UI must not promise automatic closure after arbitrary terminal use.

## 9. Implementation boundaries

Keep the implementation in the application crate with three new logical components:

| File | Responsibility |
| --- | --- |
| `src/follow.rs` | Plain-data controller, identity indexes, state transitions, suppression, generations and effects. No GPUI or OS I/O. |
| `src/herdr.rs` | Endpoint selection, protocol-22 types, request/event framing, reconnect and snapshot reconciliation. Read-only commands only. |
| `src/process_monitor.rs` | macOS process identity, local ownership correlation, exit watchers and companion activity inspection. Unknown is an explicit result. |
| `src/main.rs` | Own controller and workers, execute UI effects, handle automatic vs user selection/close, waiting view and compact controls. |
| `src/session.rs` | Creation options, one-time greeting, input/activity generation, local restart generation and staged shutdown. |
| `src/pty.rs` | Expose owned PTY foreground group/TTY inspection and hand-off for nonblocking shutdown. Preserve environment isolation. |
| `src/settings.rs` | Integrations category, source preference, enable/disable and exit policy. |
| `src/appearance.rs` or a small shared settings helper | Reuse config-directory resolution and atomic-write behavior without mixing integration fields into the appearance schema. |

The installed `portable-pty` already exposes `process_group_leader()` and `tty_name()`; its Unix group method uses `tcgetpgrp`. The existing macOS `libc` dependency is sufficient for a focused FFI layer, subject to verifying Darwin process-inspection APIs during implementation. Do not add a generic platform abstraction or a large async runtime solely for this feature.

One dedicated I/O coordinator can multiplex the long-lived stream, short-lived requests, wakeups and exit watches with bounded work. A bounded channel delivers normalized observations to GPUI; the UI drains it with a per-frame budget alongside existing output polling, or via a dedicated foreground task. No blocking syscall runs inside `Render` or the existing 16 ms session loop. Worker cancellation is owned by the app, and closing the application stops watchers and sockets before cleaning up local PTYs.

Suggested application operations:

```text
create_tab(directory, origin, optional_greeting) -> TabId
select_tab(tab_id, User | Herdr)
close_tab(tab_id, User | AgentExit(validated_close_intent))
close_pane(tab_id, pane_id, origin)
apply_observation(token, observation)
```

User-driven and automatic paths share layout manipulation but apply different focus, suppression and last-tab policies. Closing an inactive associated tab must preserve the currently selected tab ID. Removing a tab before the active vector index must not accidentally select a different tab.

Persist only preferences to `follow.json` under the existing MonoTerm config directory, honoring `MONOTERM_CONFIG_DIR`. Defaults: integration disabled, source default, exit policy close-unused. Do not persist PID associations, active shell state, connection epochs or manual-close suppression across application restarts. Runtime pin state can remain session-only.

Retain current limits: 12 tabs, 8 panes per tab, 48 panes total. On a follow creation that reaches a limit, preserve existing terminals and report capacity. Do not evict, reuse a manual tab, repeatedly retry a failed spawn, or kill a background association to make room.

## 10. Implementation sequence and acceptance

### A. Association and lifecycle foundation

Implement the pure controller, explicit tab creation/selection/close origins, stable-ID operations, waiting view, creation deduplication and manual suppression. Drive the controller with recorded fixtures before connecting it to real Herdr.

Acceptance: A -> B -> A restores identical session IDs; two agents in one directory remain separate; duplicate observations create one tab; manual close does not reopen; inactive removal preserves selection; automatic final-tab removal keeps the application alive.

### B. Read-only follow

Add socket transport, source settings, subscription-first synchronization, local-target correlation, directory resolution and the nonactivating selection path. Add the greeting and minimal Follow/Pin UI. Keep automatic closure disabled during this phase.

Acceptance: a real isolated Herdr agent selection switches the MonoTerm content while Herdr retains OS keyboard focus; typing/IME, selection, split dragging, Settings and the folder picker prevent involuntary switching. Reconnection preserves PTYs. Missing cwd, unknown protocol and capacity failures do not create guessed shells.

### C. Confirmed exit and conservative cleanup

Add validated process identities and kqueue watchers; implement retained-after-exit and the close-unused gate. Introduce generation-tagged close intents and background PTY shutdown.

Acceptance: normal quit and actual termination retire the correct owner; `done`, interrupted turns, SIGSTOP, missing metadata and connection loss retain tabs; stale process events cannot close a replacement; input arriving during inspection cancels cleanup; startup/background jobs and uncertain inspection retain the tab; cleanup never signals Herdr processes.

### D. Broader identity and safe-idle support

Before broadening guarantees, exercise pane movement, same-session resume, duplicate native identities, rapid same-kind replacement, server restart, event gaps and source changes. Add session-local shell integration only if automatic cleanup of used terminals is needed. Remote agents and selecting a particular Herdr UI client/window remain separate follow-up work because the inspected snapshot exposes no client-specific focus identity.

### Verification method

Use unit tests for controller decisions and stale-result rejection, recorded real protocol fixtures plus a fake socket server for framing/reconnect races, and native process tests for identity/exit handling. Then use the project's run-terminal driver for real MonoTerm interaction and inspect screenshots. Use a uniquely named Herdr test session, explicit parsed IDs, an isolated config directory, and test-owned floating windows under AeroSpace. Test cleanup must preserve the user's default session and existing windows.

This design-only change does not require a build or rerunning the renderer suite. Implementation must run the existing appropriate Rust/Clippy checks and focused GUI verification in addition to the new lifecycle tests.

## 11. Evidence and limits

Live lifecycle evidence is stored in `target/herdr-exit-probe/20261024-170105/result.json`, `observations.jsonl`, `actions.jsonl`, and `native-exit-confirmed.json`. The tests used Herdr 0.9.5, Codex 0.163.0, and Claude Code 2.1.310. They confirmed release events, preserved shells, interruption/completion behavior, suspension, pane closure, shell exit, and a native process-exit notification. They did not implement or validate MonoTerm automatic closing.

The available local Herdr source tree was older than the installed binary and is not used as the contract. A stable server-generation ID, replay sequence, host-qualified agent PID and client-specific focus are absent from the inspected snapshot contract. Conservative retention and fresh reconciliation are required where those gaps prevent a confident decision.
