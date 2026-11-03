// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Fabien Penso
// Workspace/agent navigation adapted from Herdr UI at 38ea4248d2c64879f02053b419bdd5386eebce10.
use super::{HerdrWindow, SidebarDrag};
use crate::{
    ui,
    window_chrome::{self, SECTION_HANDLE, SIDEBAR_HANDLE},
};
use gpui::{Context, Div, MouseButton, SharedString, Window, div, prelude::*, px, rgb, rgba};

#[path = "herdr_sidebar_rows.rs"]
pub(super) mod rows;
#[path = "herdr_workspaces.rs"]
mod workspaces;
use rows::Row;

enum FocusTarget {
    Workspace(String),
    Pane(String),
}

impl HerdrWindow {
    pub(super) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.sidebar_drag = None;
        self.save_layout(cx);
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
        if result.is_ok() {
            // Reselecting the current agent should reveal its tab after manual scrolling too.
            self.tab_reveal = Default::default();
        }
        self.record(result, cx);
        self.focus_handle.focus(window, cx);
    }

    pub(super) fn sidebar(&self, width: f32, height: f32, cx: &mut Context<Self>) -> Div {
        let p = self.palette.herdr_sidebar();
        let language = self.language;
        let connection = self.connection.read(cx);
        let connected = connection.connected();
        let snapshot = connection.snapshot.clone();
        let mut spaces = div().w_full().flex().flex_col().gap(px(1.0));
        let mut agents = div().w_full().flex().flex_col().gap(px(1.0));
        let mut spaces_count = 0;
        let mut agent_count = 0;
        if let Some(snapshot) = &snapshot {
            for entry in workspaces::entries(&snapshot.workspaces, &self.collapsed_repos) {
                spaces_count += 1;
                let workspace = entry.workspace;
                let selected = snapshot.focused_workspace_id.as_deref()
                    == Some(workspace.workspace_id.as_str());
                let target = FocusTarget::Workspace(workspace.workspace_id.clone());
                let menu_target =
                    crate::herdr_connection::WorkspaceTarget::new(snapshot, workspace);
                let group = entry.group.map(str::to_owned);
                spaces = spaces.child(
                    Row::workspace(&entry)
                        .render(selected, connected, self)
                        .w(px(width - SIDEBAR_HANDLE))
                        .when(connected, |row| {
                            row.on_click(cx.listener(move |view, _, window, cx| {
                                cx.stop_propagation();
                                view.navigate(&target, window, cx);
                            }))
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(
                                    move |view, event: &gpui::MouseDownEvent, window, cx| {
                                        view.open_workspace_menu(
                                            menu_target.clone(),
                                            group.clone(),
                                            event.position,
                                            window,
                                            cx,
                                        );
                                        cx.stop_propagation();
                                    },
                                ),
                            )
                        })
                        .when_some(entry.group, |row, key| {
                            let collapsed = self.collapsed_repos.contains(key);
                            let key = key.to_owned();
                            row.child(
                                ui::keyboard_control(
                                    div()
                                        .id(SharedString::from(format!(
                                            "herdr-fold-{}",
                                            workspace.workspace_id
                                        )))
                                        .size(px(20.0))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(4.0)),
                                    p,
                                )
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgba(p.control_hover())))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    cx.stop_propagation();
                                    if !view.collapsed_repos.remove(&key) {
                                        view.collapsed_repos.insert(key.clone());
                                    }
                                    view.save_layout(cx);
                                    view.focus_handle.focus(window, cx);
                                    cx.notify();
                                }))
                                .child(
                                    ui::icon(if collapsed { "chevron" } else { "chevron-down" })
                                        .size(px(12.0))
                                        .text_color(rgb(p.muted)),
                                ),
                            )
                        }),
                );
            }
            for agent in rows::agents(snapshot) {
                agent_count += 1;
                let selected = snapshot.focused_pane_id.as_deref() == Some(agent.pane_id.as_str());
                let target = FocusTarget::Pane(agent.pane_id.clone());
                agents = agents.child(
                    Row::agent(agent, snapshot)
                        .render(selected, connected, self)
                        .when(connected, |row| {
                            row.on_click(cx.listener(move |view, _, window, cx| {
                                cx.stop_propagation();
                                view.navigate(&target, window, cx);
                            }))
                        }),
                );
            }
        }
        let empty = |label| {
            div()
                .h(px((self.font_size * 1.35).ceil() + 10.0))
                .px_2()
                .text_size(px(self.font_size))
                .text_color(rgb(p.muted))
                .child(language.text(label))
        };
        if spaces_count == 0 {
            spaces = spaces.child(empty(if connected {
                "No spaces available"
            } else {
                "Waiting for Herdr"
            }));
        }
        if agent_count == 0 {
            agents = agents.child(empty(if !connected {
                "Waiting for Herdr"
            } else if snapshot
                .as_ref()
                .is_some_and(|s| s.agent_view_label.is_some())
            {
                "No matching agents"
            } else {
                "No agents available"
            }));
        }
        let space_height = if self.sidebar_view.both() {
            window_chrome::section_height(self.sidebar_ratio, height - SECTION_HANDLE)
        } else {
            height
        };
        div()
            .w(px(width - SIDEBAR_HANDLE))
            .h_full()
            .min_h_0()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font(self.font.clone())
            .font_features(self.font.features.clone())
            .text_size(px(self.font_size))
            .line_height(px((self.font_size * 1.35).ceil()))
            .when(self.sidebar_view.spaces(), |sidebar| {
                sidebar.child(
                    div()
                        .h(px(space_height))
                        .flex_none()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(
                            window_chrome::sidebar_heading()
                                .pl(px(14.0))
                                .h(px(32.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px((self.font_size - 2.0).max(10.0)))
                                        .text_color(rgb(p.section_heading()))
                                        .child("SPACES"),
                                )
                                .child(
                                    ui::icon_button_enabled(
                                        "herdr-new-workspace",
                                        "plus",
                                        language.text("New workspace"),
                                        p,
                                        connected,
                                    )
                                    .when(
                                        connected,
                                        |button| {
                                            button.on_click(cx.listener(|view, _, window, cx| {
                                                let result =
                                                    view.connection.update(cx, |connection, cx| {
                                                        let result = connection.create_workspace();
                                                        cx.notify();
                                                        result
                                                    });
                                                view.record(result, cx);
                                                view.focus_handle.focus(window, cx);
                                            }))
                                        },
                                    ),
                                ),
                        )
                        .child(
                            div()
                                .id("herdr-spaces-scroll")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .track_scroll(&self.sidebar_scroll[0])
                                .child(spaces),
                        ),
                )
            })
            .when(self.sidebar_view.both(), |sidebar| {
                sidebar.child(
                    div()
                        .id("herdr-section-divider")
                        .group("herdr-section-divider")
                        .h(px(SECTION_HANDLE))
                        .w_full()
                        .flex_none()
                        .px_2()
                        .flex()
                        .items_center()
                        .cursor_row_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, cx| {
                                view.sidebar_drag = Some(SidebarDrag::Sections);
                                view.selecting = false;
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        )
                        .child(
                            div()
                                .h(px(1.0))
                                .w_full()
                                .bg(rgba((p.text << 8) | 12))
                                .group_hover("herdr-section-divider", move |style| {
                                    style.bg(rgba((p.text << 8) | 20))
                                }),
                        ),
                )
            })
            .when(self.sidebar_view.agents(), |sidebar| {
                sidebar.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(
                            window_chrome::sidebar_heading()
                                .pl(px(14.0))
                                .h(px(32.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(px((self.font_size - 2.0).max(10.0)))
                                        .text_color(rgb(p.section_heading()))
                                        .child("AGENTS"),
                                ),
                        )
                        .child(
                            div()
                                .id("herdr-agents-scroll")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .track_scroll(&self.sidebar_scroll[1])
                                .pb_2()
                                .child(agents),
                        ),
                )
            })
    }

    pub(super) fn sidebar_resize_handle(&self, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        div()
            .id("herdr-sidebar-resize")
            .w(px(SIDEBAR_HANDLE))
            .h_full()
            .flex_none()
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.sidebar_drag = Some(SidebarDrag::Width);
                    view.selecting = false;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
    }
}
