// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Fabien Penso
//
// Adapted from Herdr UI, revision 38ea4248d2c64879f02053b419bdd5386eebce10:
// crates/herdr-gpui/src/sidebar/render.rs and render/agent_rows.rs.
// MonoTerm keeps its shared connection/focus fences; rows are the small native
// Herdr layout subset in herdr_sidebar_rows.rs, not a second app dependency.

use super::HerdrWindow;
use crate::ui;
use gpui::{Context, Div, Window, div, prelude::*, px, relative, rgb, rgba};

#[path = "herdr_sidebar_rows.rs"]
mod rows;

pub(super) use rows::status_color;
use rows::{ROW_PADDING, Row, RowContext};

pub(super) const SIDEBAR_WIDTH: f32 = 232.0;

enum FocusTarget {
    Workspace(String),
    Pane(String),
}

impl HerdrWindow {
    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    fn navigate(&mut self, target: &FocusTarget, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.connection.update(cx, |connection, cx| {
            let result = match target {
                FocusTarget::Workspace(id) => connection.focus_workspace(id),
                FocusTarget::Pane(id) => connection.focus_pane(id),
            };
            cx.notify();
            result
        });
        self.record(result, cx);
        self.focus_handle.focus(window, cx);
    }

    pub(super) fn sidebar(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let palette = self.palette;
        let language = self.language;
        let connection = self.connection.read(cx);
        let connected = connection.connected();
        let snapshot = connection.snapshot.clone();
        let row_cx = RowContext {
            font: &self.font,
            // Upstream defaults its sidebar to 12px independently of terminal
            // zoom. Keep MonoTerm's chosen face with a readable compact UI size.
            font_size: self.font_size.clamp(12.0, 13.0),
            palette,
            language,
            text_system: window.text_system().clone(),
        };
        let mut spaces = div().w_full().flex().flex_col();
        let mut agents = div().w_full().flex().flex_col();
        let mut spaces_count = 0;
        let mut agent_count = 0;
        let mut agent_content_height = 0.0;

        if let Some(snapshot) = &snapshot {
            for workspace in &snapshot.workspaces {
                spaces_count += 1;
                let selected = snapshot.focused_workspace_id.as_deref()
                    == Some(workspace.workspace_id.as_str());
                let target = FocusTarget::Workspace(workspace.workspace_id.clone());
                spaces = spaces.child(
                    Row::workspace(workspace)
                        .render(selected, connected, &row_cx)
                        .when(connected, |row| {
                            row.on_click(cx.listener(move |view, _, window, cx| {
                                cx.stop_propagation();
                                view.navigate(&target, window, cx);
                            }))
                        }),
                );
            }
            for agent in rows::agents(snapshot) {
                agent_count += 1;
                let selected = snapshot.focused_pane_id.as_deref() == Some(agent.pane_id.as_str());
                let target = FocusTarget::Pane(agent.pane_id.clone());
                let row = Row::agent(agent, snapshot);
                agent_content_height += row.height(&row_cx);
                agents = agents.child(row.render(selected, connected, &row_cx).when(
                    connected,
                    |row| {
                        row.on_click(cx.listener(move |view, _, window, cx| {
                            cx.stop_propagation();
                            view.navigate(&target, window, cx);
                        }))
                    },
                ));
            }
        }

        if spaces_count == 0 {
            spaces = spaces.child(empty_label(
                language.text(if connected {
                    "No spaces available"
                } else {
                    "Waiting for Herdr"
                }),
                &row_cx,
            ));
        }
        if agent_count == 0 {
            agent_content_height = row_cx.line_height();
            agents = agents.child(empty_label(
                language.text(if !connected {
                    "Waiting for Herdr"
                } else if snapshot
                    .as_ref()
                    .is_some_and(|s| s.agent_view_label.is_some())
                {
                    "No matching agents"
                } else {
                    "No agents available"
                }),
                &row_cx,
            ));
        }

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .min_h_0()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_r_1()
            .border_color(rgb(palette.divider))
            .font(self.font.clone())
            .text_size(px(row_cx.font_size))
            .line_height(px(row_cx.line_height()))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(heading("spaces", &row_cx))
                    .child(
                        div()
                            .id("herdr-spaces-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.sidebar_scroll[0])
                            .child(spaces),
                    )
                    .child(self.sidebar_footer(connected, &row_cx, cx)),
            )
            .child(
                div()
                    // The agent list takes only its content height, capped at
                    // half the sidebar; the spaces list gets the remaining room.
                    .h(px(agent_content_height + row_cx.header_height() + 12.0))
                    .max_h(relative(0.5))
                    .min_h_0()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .border_t_1()
                    .border_color(rgb(palette.divider))
                    .pt(px(5.0))
                    .child(heading("agents", &row_cx))
                    .child(
                        div()
                            .id("herdr-agents-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.sidebar_scroll[1])
                            .pb(px(6.0))
                            .child(agents),
                    ),
            )
    }

    fn sidebar_footer(
        &self,
        connected: bool,
        row_cx: &RowContext<'_>,
        cx: &mut Context<Self>,
    ) -> Div {
        let palette = self.palette;
        let language = self.language;
        let new_tooltip = language.text("New workspace");
        let menu_tooltip = language.text("MonoTerm Settings");
        div()
            .h(px(row_cx.footer_height()))
            .flex_none()
            .px(px(ROW_PADDING))
            .flex()
            .items_center()
            .justify_between()
            .text_color(rgb(palette.muted))
            .child(
                div()
                    .id("herdr-new-workspace")
                    .child(language.text("new"))
                    .when(connected, |button| {
                        button.tooltip(move |_, cx| {
                            cx.new(|_| ui::Tooltip(new_tooltip, palette)).into()
                        })
                    })
                    .when(!connected, |button| button.text_color(rgb(palette.subtle)))
                    .when(connected, |button| {
                        ui::keyboard_control(button, palette)
                            .cursor_pointer()
                            .hover(move |style| {
                                style
                                    .bg(rgba(palette.control_hover()))
                                    .text_color(rgb(palette.text))
                            })
                            .active(move |style| style.bg(rgba(palette.control_pressed())))
                            .on_click(cx.listener(|view, _, window, cx| {
                                cx.stop_propagation();
                                let result = view.connection.update(cx, |connection, cx| {
                                    let result = connection.create_workspace();
                                    cx.notify();
                                    result
                                });
                                view.record(result, cx);
                                view.focus_handle.focus(window, cx);
                            }))
                    }),
            )
            .child(
                ui::keyboard_control(div().id("herdr-sidebar-menu"), palette)
                    .child(language.text("menu"))
                    .cursor_pointer()
                    .hover(move |style| {
                        style
                            .bg(rgba(palette.control_hover()))
                            .text_color(rgb(palette.text))
                    })
                    .active(move |style| style.bg(rgba(palette.control_pressed())))
                    .tooltip(move |_, cx| cx.new(|_| ui::Tooltip(menu_tooltip, palette)).into())
                    .on_click(cx.listener(|view, _, _, cx| {
                        cx.stop_propagation();
                        let owner = view.owner;
                        cx.defer(move |cx| {
                            if let Err(error) =
                                owner.update(cx, |root, window, cx| root.open_settings(window, cx))
                            {
                                eprintln!("Herdr settings: {error}");
                            }
                        });
                    })),
            )
    }
}

fn heading(label: &'static str, cx: &RowContext<'_>) -> Div {
    div()
        .h(px(cx.header_height()))
        .flex_none()
        .px(px(ROW_PADDING))
        .flex()
        .items_center()
        .text_color(rgb(cx.palette.muted))
        .child(cx.language.text(label))
}

fn empty_label(label: &'static str, cx: &RowContext<'_>) -> Div {
    div()
        .h(px(cx.line_height()))
        .px(px(ROW_PADDING))
        .truncate()
        .text_color(rgb(cx.palette.muted))
        .child(label)
}
