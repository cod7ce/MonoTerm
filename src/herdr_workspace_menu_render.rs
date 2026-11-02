//! Space popover and its bounded, keyboard-accessible dialogs.
use super::{Action, HerdrWindow, Mode};
use crate::ui;
use gpui::{AnyElement, Context, FontWeight, MouseButton, Window, div, prelude::*, px, rgb, rgba};

impl HerdrWindow {
    pub(in crate::herdr_view) fn render_workspace_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.workspace_menu.as_ref()?;
        let p = self.palette.herdr_sidebar();
        let t = |label| self.language.text(label);
        let danger = if !self.palette.dark {
            0xb33131
        } else {
            0xef8888
        };
        let item = |index: usize, label: &'static str, enabled: bool| {
            ui::keyboard_control(div().id(("space-menu-item", index)), p)
                .when(enabled, |item| {
                    item.track_focus(&menu.items[index])
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgba(p.control_hover())))
                        .active(move |style| style.bg(rgba(p.control_pressed())))
                })
                .when(!enabled, |item| item.opacity(0.4))
                .h(px(30.0))
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .rounded(px(5.0))
                .text_size(px(13.0))
                .child(t(label))
        };
        let is_menu = matches!(menu.mode, Mode::Menu);
        let busy = menu.pending.is_some();
        let mut card = div()
            .id("space-menu-card")
            .p_2()
            .rounded(px(10.0))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .text_color(rgb(p.text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
        match &menu.mode {
            Mode::Menu => {
                let actions = menu.actions();
                let height = 150.0 + (actions.len() - 3) as f32 * 32.0;
                card = card
                    .absolute()
                    .w(px(330.0))
                    .max_h(window.viewport_size().height - px(48.0))
                    .overflow_y_scroll()
                    .left(
                        menu.position
                            .x
                            .min(window.viewport_size().width - px(338.0))
                            .max(px(8.0)),
                    )
                    .top(
                        menu.position
                            .y
                            .min(window.viewport_size().height - px(height + 8.0))
                            .max(px(40.0)),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_2()
                            .mb_2()
                            .border_b_1()
                            .border_color(rgb(p.border))
                            .child(
                                div()
                                    .whitespace_normal()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(herdr_client::text::safe_text(
                                        &menu.target.workspace.label,
                                        160,
                                    )),
                            )
                            .when_some(menu.target.workspace.branch.as_ref(), |header, branch| {
                                header.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .mt_1()
                                        .text_size(px(12.0))
                                        .text_color(rgb(p.muted))
                                        .child(
                                            ui::icon("branch")
                                                .size(px(13.0))
                                                .text_color(rgb(p.muted)),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .child(herdr_client::text::safe_text(branch, 160)),
                                        ),
                                )
                            }),
                    );
                let mut tiles = div().flex().gap_2().mb_2();
                for (index, &(action, label, icon, enabled)) in actions.iter().take(3).enumerate() {
                    let tile = ui::keyboard_control(div().id(("space-menu-tile", index)), p)
                        .flex_1()
                        .min_w_0()
                        .h(px(70.0))
                        .rounded(px(6.0))
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_size(px(12.0))
                        .bg(rgba(p.control_hover()))
                        .when(index == 0 && enabled, |tile| tile.text_color(rgb(p.accent)))
                        .child(ui::icon(icon).size(px(20.0)).text_color(rgb(
                            if index == 0 && enabled {
                                p.accent
                            } else {
                                p.text
                            },
                        )))
                        .child(t(label))
                        .when(enabled, |tile| {
                            tile.track_focus(&menu.items[index])
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgba(p.control_pressed())))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.workspace_menu_action(action, window, cx)
                                }))
                        })
                        .when(!enabled, |tile| tile.opacity(0.4));
                    tiles = tiles.child(tile);
                }
                card = card.child(tiles);
                for (index, (action, mut label, icon, enabled)) in
                    actions.into_iter().enumerate().skip(3)
                {
                    if matches!(action, Action::Fold)
                        && menu
                            .group
                            .as_ref()
                            .is_some_and(|key| self.collapsed_repos.contains(key))
                    {
                        label = "Expand group";
                    }
                    let destructive = matches!(action, Action::DeleteWorktree);
                    if destructive {
                        card = card.child(div().mt_2().mb_1().h(px(1.0)).bg(rgb(p.border)));
                    }
                    // Keep the leading icon before the caption for every secondary row.
                    let row = ui::keyboard_control(div().id(("space-menu-row", index)), p)
                        .track_focus(&menu.items[index])
                        .h(px(32.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(5.0))
                        .cursor_pointer()
                        .text_size(px(13.0))
                        .hover(move |style| style.bg(rgba(p.control_hover())))
                        .active(move |style| style.bg(rgba(p.control_pressed())))
                        .text_color(rgb(if destructive { danger } else { p.text }))
                        .child(
                            ui::icon(icon)
                                .size(px(16.0))
                                .text_color(rgb(if destructive { danger } else { p.muted })),
                        )
                        .child(t(label))
                        .when(enabled, |row| {
                            row.on_click(cx.listener(move |view, _, window, cx| {
                                view.workspace_menu_action(action, window, cx)
                            }))
                        });
                    card = card.child(row);
                }
            }
            _ => {
                let title = match &menu.mode {
                    Mode::Rename(_) => "Rename Space",
                    Mode::Close(_) if menu.target.close_members.len() > 1 => "Close group?",
                    Mode::Close(_) => "Close only this Space?",
                    Mode::New(_) => "New worktree",
                    Mode::Open(_) => "Open worktree",
                    Mode::Delete { force: true, .. } => "Force delete checkout?",
                    _ => "Delete worktree checkout?",
                };
                card = card
                    .w(px(480.0))
                    .max_w(window.viewport_size().width - px(32.0))
                    .max_h(window.viewport_size().height - px(64.0))
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t(title)),
                    )
                    .child(div().text_size(px(13.0)).truncate().child(
                        herdr_client::text::safe_text(&menu.target.workspace.label, 160),
                    ));
                match &menu.mode {
                    Mode::Rename(input) | Mode::New(input) => {
                        if matches!(menu.mode, Mode::New(_)) {
                            card = card
                                .child(div().text_size(px(12.0)).text_color(rgb(p.muted)).child(
                                    format!(
                                            "{}: {}",
                                            t("Base branch"),
                                            menu.source
                                                .as_ref()
                                                .map_or("HEAD", |source| source.base.as_str())
                                        ),
                                ))
                                .child(
                                    div().text_size(px(13.0)).child(t("Branch name (optional)")),
                                );
                        }
                        card = card.child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded(px(5.0))
                                .border_1()
                                .border_color(rgb(p.muted))
                                .bg(rgb(p.terminal))
                                .child(input.clone()),
                        );
                        if matches!(menu.mode, Mode::New(_)) {
                            card = card.child(div().text_size(px(12.0)).text_color(rgb(p.muted)).child(t("Leave blank to generate a branch name. Herdr chooses the checkout directory.")));
                        }
                    }
                    Mode::Close(panes) => {
                        card = card.child(div().text_size(px(13.0)).child(format!("{}: {} · {}: {}", t("Spaces to close"), menu.target.close_members.len(), t("Terminals to close"), panes.len())))
                            .child(div().text_size(px(12.0)).text_color(rgb(p.muted)).child(t("Running processes in these Spaces will end. Checkout files and branches are kept.")));
                    }
                    Mode::Open(Some(list)) => {
                        if list.entries.is_empty() {
                            card = card.child(div().child(t("No worktrees available")));
                        }
                        let mut rows = div()
                            .id("worktree-list")
                            .max_h(px(300.0))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap_1();
                        for (index, entry) in list.entries.iter().enumerate() {
                            let caption = entry.branch.as_deref().unwrap_or(&entry.label);
                            rows = rows.child(
                                ui::keyboard_control(div().id(("worktree-entry", index)), p)
                                    .when(!busy, |row| {
                                        row.track_focus(&menu.items[index + 2])
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgba(p.control_hover())))
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.open_worktree_entry(index, cx)
                                            }))
                                    })
                                    .when(busy, |row| row.opacity(0.4))
                                    .p_2()
                                    .rounded(px(5.0))
                                    .child(
                                        div()
                                            .flex()
                                            .justify_between()
                                            .text_size(px(13.0))
                                            .child(
                                                div().truncate().child(
                                                    herdr_client::text::safe_text(caption, 160),
                                                ),
                                            )
                                            .when(entry.open_workspace_id.is_some(), |row| {
                                                row.child(
                                                    div().text_color(rgb(p.muted)).child(t("Open")),
                                                )
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(p.muted))
                                            .whitespace_normal()
                                            .child(herdr_client::text::safe_text(
                                                &entry.path,
                                                1024,
                                            )),
                                    ),
                            );
                        }
                        card = card.child(rows);
                    }
                    Mode::Delete {
                        path: Some(path),
                        force,
                    } => {
                        card = card
                            .child(
                                div()
                                    .p_2()
                                    .rounded(px(5.0))
                                    .bg(rgb(p.terminal))
                                    .text_size(px(12.0))
                                    .whitespace_normal()
                                    .child(herdr_client::text::safe_text(path, 4096)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(if *force { danger } else { p.muted }))
                                    .child(t(if *force {
                                        "Modified, untracked and submodule files in this checkout will be permanently lost. The branch is kept."
                                    } else {
                                        "Remove this checkout directory and close its Space. The branch is kept."
                                    })),
                            );
                    }
                    _ => {}
                }
                if busy {
                    card = card.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(p.muted))
                            .child(t("Working…")),
                    );
                }
                if let Some(error) = &menu.error {
                    card = card.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(danger))
                            .whitespace_normal()
                            .child(self.language.dynamic_text(error)),
                    );
                }
                let can_submit =
                    !busy && !matches!(menu.mode, Mode::Open(_) | Mode::Delete { path: None, .. });
                let mut buttons =
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(item(0, "Cancel", true).on_click(cx.listener(
                            |view, _, window, cx| view.dismiss_workspace_menu(window, cx),
                        )));
                if !matches!(menu.mode, Mode::Open(_)) {
                    let label = match &menu.mode {
                        Mode::Rename(_) => "Save",
                        Mode::New(_) => "Create worktree",
                        Mode::Delete { force: true, .. } => "Force remove",
                        Mode::Delete { .. } => "Remove",
                        _ if menu.target.close_members.len() > 1 => "Close group",
                        _ => "Close Space",
                    };
                    buttons = buttons.child(
                        item(1, label, can_submit)
                            .when(matches!(menu.mode, Mode::Delete { .. }), |button| {
                                button.text_color(rgb(danger))
                            })
                            .when(can_submit, |button| {
                                button.on_click(cx.listener(|view, _, window, cx| {
                                    view.confirm_workspace_dialog(window, cx)
                                }))
                            }),
                    );
                }
                card = card.child(buttons);
            }
        }
        Some(
            ui::block_terminal_actions(div().id("space-menu-overlay"))
                .absolute()
                .inset_0()
                .occlude()
                .track_focus(&menu.focus)
                .tab_group()
                .tab_stop(false)
                .when(!is_menu, |overlay| {
                    overlay
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(0x00000048))
                })
                .capture_key_down(cx.listener(Self::workspace_menu_key_down))
                .on_key_down(cx.listener(|view, _, window, cx| {
                    // Native text insertion follows key dispatch; stopping it here eats typing.
                    let editing = view.workspace_menu.as_ref().is_some_and(|menu| {
                        menu.input()
                            .is_some_and(|input| input.read(cx).is_focused(window))
                    });
                    if !editing {
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, window, cx| {
                        if is_menu {
                            view.dismiss_workspace_menu(window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |view, _, window, cx| {
                        if is_menu {
                            view.dismiss_workspace_menu(window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(card)
                .into_any_element(),
        )
    }
}
