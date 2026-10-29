# Consistent Control States

Status: implemented across the main window, Settings, tab editor and embedded Herdr chrome, with targeted GUI verification on 2026-10-28. The final slider mouse-focus fix passed its keyboard GUI rerun, formatting and strict Clippy. The rebuilt `target/MonoTerm.app` includes that fix and passed strict/deep signature verification, plist validation and icon-resource comparison (`target/control-final-bundle.log`). The tab close-menu and plus-alignment changes are verified separately.

Shared palette states preserve the local sidebar-tab baseline. Selected controls keep their accent fill on hover; action buttons use momentary feedback. SVG icons and tooltips follow the active palette. Keyboard focus uses a fine accent border, and the editor title border follows actual input focus. Disabled controls retain their displayed value but omit hover/activation and keyboard traversal.

Ctrl-F6 enters main/Herdr chrome, Tab/Shift-Tab traverses enabled controls, Enter/Space uses GPUI click activation, and Escape returns to the terminal. Choice arrows move focus through the window's control order without committing a value; Enter/Space commits. Choice arrows are not confined to individual option groups. Settings sliders use arrows (Shift for 10-step changes), Home and End. Tab menu arrows and Tab wrap within its three items; editor Tab wraps from Save to title and Shift-Tab reverses. Herdr control focus prevents terminal-key and text-input forwarding until terminal focus is restored.

## Verification evidence

- `cargo check --locked --package monoterm`, formatting, **114 application tests** (one opt-in ignored) and strict all-target Clippy passed. Logs: `target/control-{check,tests,clippy,build}.log`. The final one-line slider propagation fix was then checked with formatting, strict Clippy (`target/control-slider-clippy.log`) and the dedicated keyboard GUI run below; the application tests were not repeated for that event-routing fix.
- `python3 -I target/check-control-states.py` passed **20 checks**, including six environment checks, in `target/gui-check/20261028-134340/result.json`. It covers toolbar hover and icon ink, keyboard category activation, category/theme selection, editor icon selection with Space and restored terminal input. Dark/Light, disabled settings, selected/hovered editor icons and independent focus outlines were inspected in the captured PNGs.
- `python3 -I target/check-herdr-control-states.py` passed **16 checks**, including six environment checks, in `target/gui-check/20261028-135856/result.json`. It covers selected-tab hover, acknowledged tab switching with unchanged daemon IDs, Ctrl-F6/Space sidebar toggling and Escape restoring native terminal input. All six Herdr Dark/Light, selected-hover, focus and sidebar PNGs were inspected. Selected/hovered tab samples were `(53,53,64)` and `(53,54,68)`.
- `python3 -I target/check-control-keyboard.py` passed **17 checks**, including six environment checks, in `target/gui-check/20261028-142115/result.json`. It covers menu Down/Enter activation, Escape dismissal, backward Tab/Space activation of background-tab Close, slider one/ten-step arrows and Home/End, disabled Light-mode slider value preservation, and restored terminal input. Menu/editor and Dark slider/Light disabled PNGs were inspected. The preceding attempt (`20261028-140954`) exposed the mouse handler stopping propagation before GPUI's automatic focus listener. Removing that stop fixed the observed failure; after explicit build permission, the final run passed.
- Recorded app/initial-shell PIDs from all four runs exited. The uniquely owned Herdr session was stopped and deleted. Herdr and final keyboard drivers requested restoration of captured workspace `B`; later inspection saw `T`, so those runs' workspace restoration was not independently confirmed. Existing user instances were not restarted.
- This is representative coverage of the changed controls, not a repeat of the full GUI matrix. Native IME composition, macOS system-theme switching and every individual disabled-control transition were not exercised by these runs.

Earlier failures remain recorded. Run `20261028-120823` exposed invisible toolbar SVGs because this GPUI version requires explicit SVG ink; `control_icon` now supplies base and group-hover colors. Run `20261028-133911` exposed an opaque focus shadow, replaced with a 1px border. Toolbar/category hover thresholds were corrected to accept the intended subtle tint while retaining RGB samples and checking tint direction. The Herdr attempt `20261028-135417` failed on OCR's status bullet before the tab label; the fixture now matches the label within the top-strip bounds. The successful runs above include the fixes.

## Visual baseline

Use the local sidebar tab as the reference: a quiet rounded surface, a translucent accent fill for selection, a slightly stronger selected-hover fill, normal foreground text, and an accent icon. Selection should remain visible while hovering. Preserve current layouts and hit areas.

The existing reference is `Palette::tab_background` in `src/appearance.rs`. Its accent is mixed with foreground by 12% to keep very dark and very light custom accents visible. Current nominal alpha values are:

| State | Dark | Light |
| --- | ---: | ---: |
| Idle | 0 | 0 |
| Hover | 12/255 | 15/255 |
| Selected | 32/255 | 36/255 |
| Selected + hover | 42/255 | 46/255 |

These are baseline values, not values to paste onto every background. The current tab helper reduces alpha according to the transparent surround, because the pinned renderer adds layer alpha. Settings and editor panels are opaque and must use their actual surface context instead. Preserve the verified tab appearance when introducing shared helpers.

Use 6px corners for compact custom control backgrounds, matching tab rows. Keep larger cards and overlay containers at their current radii. Icon-only targets remain 28px; text actions retain their current heights. No layout-wide size changes are proposed.

## State contract

| State | Appearance | Behavior |
| --- | --- | --- |
| Idle | Quiet or transparent fill; neutral text/icon | Ready for interaction |
| Hover | Soft accent tint; foreground ink | Does not change the selected value |
| Pressed | Briefly stronger accent tint | Exists only during activation; returns to the appropriate idle/selected state |
| Selected / on | Tab-style accent fill, foreground label, accent icon; medium label weight where it cannot shift layout | Reflects persistent application state |
| Selected + hover | Stronger selected fill | Never replaced by neutral hover fill |
| Keyboard focus | A visible accent outline in addition to the existing fill | Tracks actual focus, independently of selection and hover |
| Disabled | Muted ink and reduced contrast; retained shape/value | No pointer cursor, activation, hover or pressed response; remove from keyboard traversal |

An action button is not selected after it is clicked. New tab, restart, split, reconnect, Save, Cancel, Restore default and Close use idle/hover/pressed feedback. Save may keep restrained primary emphasis; it does not use the semantic selected flag. Persistent choices and on/off controls derive their selected state from the application model, not from the latest click.

Close remains a normal menu action. Reserve error/destructive color for explicit destructive warnings or confirmation surfaces, rather than introducing a red selected tab. Status indicators retain their existing meanings: green follow, amber pin and agent lifecycle colors must not become selection colors.

## Window coverage

| Surface | Controls | Treatment |
| --- | --- | --- |
| Main terminal | Sidebar and Herdr window toggles, Settings entry, Zen controls | Shared palette-aware icon-button states; show persistent on/open state only where there is a corresponding state to display |
| Main terminal | New tab, restart, splits, pane close, Resume, error dismiss | Momentary action feedback using the same accent hover/pressed family |
| Main terminal | Local tabs and tab context menu | Preserve tab reference appearance; menu actions get shared hover and keyboard-highlight treatment |
| Settings | Category navigation, theme cards, language choices | Tab-style selected and selected-hover fill; retain checkmarks on exclusive choices |
| Settings | Accent swatches | Preserve the actual swatch color; indicate selection with a checkmark/ring and a consistent surrounding selection treatment |
| Settings | Steppers, Reset, Reload and font controls | Shared action states and disabled behavior |
| Settings | Switches and sliders | Share accent, focus and disabled tokens; retain switch thumb position and slider value/track geometry |
| Tab editor | Icon options, including Default | Tab-style selection with selected-hover; retain selected-value semantics |
| Tab editor | Save, Cancel and Restore default | Shared action-button states; Save retains primary emphasis |
| Tab editor | Title input | Focus border only when actually focused; independent text-selection colors and caret behavior |
| Embedded Herdr window | Tabs, workspaces and agent rows | Same selected fill and selected-hover contract as local tabs |
| Embedded Herdr window | Back/forward, sidebar, new tab/workspace, close, reconnect and menus | Shared action/toggle/disabled states; retain agent lifecycle indicators |

There is currently no native or custom select dropdown in the inspected app: language uses choice buttons, and font family uses a stepper. For future selects, the closed control displays its committed value; the open list uses a checkmark and selected fill for that value, with an independent pointer/keyboard highlight. Opening or navigating the list must not commit a new value. Enter/click commits; Escape dismisses and preserves the value.

Native macOS traffic lights and application menus keep platform behavior. Terminal text selection, title-input text selection and Herdr terminal cell rendering are content-selection surfaces, not button states. Use separate text-selection colors with readable foreground contrast. Herdr daemon-owned TUI menus are rendered from cell frames and are outside this local chrome plan.

## Original implementation sequence

The sequence below records the rollout plan. The status and evidence above describe the delivered implementation.

1. **Shared appearance helpers.** Add semantic state colors in `appearance.rs`, with an explicit surface context for transparent surround versus opaque panel. Keep `tab_background` as a compatibility wrapper initially. Define hover, pressed, selected, selected-hover, focus and disabled colors without changing the existing tab pixels.
2. **Shared controls and main window.** Add small palette-aware action-button, selectable-control and menu-row helpers in `ui.rs`. Replace fixed dark `MUTED` / `SELECTED` / `TEXT` styling in custom controls and tooltips. Apply state colors to the actual child icon as well as its parent. Migrate main toolbar actions and context-menu rows first.
3. **Settings.** Migrate category, theme and language choices; then actions, steppers, switches and sliders. Retain exclusive-choice checkmarks, disabled action guards, live preference updates and all existing hit geometry.
4. **Tab editor.** Migrate icon choices and action buttons. Tie the title-field focus border to real input focus. Keep Unicode editing, IME, terminal-shortcut suppression and draft Save/Cancel semantics unchanged.
5. **Embedded Herdr chrome.** Migrate local tabs, workspace/agent rows and navigation/actions. Ensure hovering a selected row does not erase its selection. Preserve daemon identity, topology and command behavior.
6. **Keyboard interaction.** Add real focus handles/traversal and Enter/Space activation for applicable controls as each surface is migrated. Choice groups use arrow navigation; menus support arrows/Enter/Escape. Do not draw focus without implementing focus behavior. Preserve terminal shortcuts outside a focused control or overlay.

Relevant code: `src/appearance.rs`, `src/ui.rs`, `src/main.rs`, `src/settings.rs`, `src/tab_editor.rs`, `src/text_input.rs`, `src/herdr_chrome.rs`, `src/herdr_sidebar.rs` and `src/herdr_sidebar_rows.rs`.

## Acceptance and verification

- Selected, selected-hover and keyboard focus remain distinguishable in Dark and Light.
- Selected fills remain visible on opaque panels without flattening transparent tab surfaces.
- A momentary action never remains visually selected after activation.
- Disabled controls cannot activate by mouse or keyboard and do not gain hover feedback.
- Actual child icons follow the state color; a parent's foreground change must not be blocked by a fixed child color.
- Menu navigation, dropdown highlight and committed value remain separate.
- Layout, hit targets, PTYs, follow pinning, saved preferences and native window behavior remain unchanged.
- Validate each migration with the affected surface's targeted GUI checks and inspected screenshots. Start with Dark/Light, selected-hover, one keyboard flow and disabled behavior. Expand to the full material or window matrix only if shared rendering/layout changes justify it.

## Implemented sidebar adjustments

- Local tab rows no longer reserve space for a close glyph. The former 20px target and 8px gap become title space.
- Right-click offers Customize tab, Restore default and Close tab. Close resolves the stable tab ID at activation and closes every pane in that tab through the existing lifecycle path.
- Background close preserves the selected stable tab. Active close uses the existing next/previous neighbor behavior. Cmd-Shift-W remains available.
- The new-tab plus and sidebar toggle share the x center `sidebar_width - 22`, including after resizing. The heading label keeps its left inset.
