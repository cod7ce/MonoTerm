//! Adapted from Herdr UI's titlebar/{tabs,navigation}.rs and tab_layout/classic.rs.
//! Copyright 2026 Fabien Penso. Apache-2.0; see vendor/herdr/NOTICE.
//! MonoTerm adapts the GPUI version, snapshot data and acknowledged connection actions.
use super::{HerdrWindow, sidebar::SIDEBAR_WIDTH};
use crate::ui;
use gpui::{Context, Div, MouseButton, SharedString, Window, div, prelude::*, px, rgb, rgba};

pub(super) const HEADER_HEIGHT: f32 = 34.0;
pub(super) const FOOTER_HEIGHT: f32 = 22.0;

impl HerdrWindow {
    fn history_destination(&self, back: bool, cx: &Context<Self>) -> Option<(usize, String)> {
        let connection = self.connection.read(cx);
        if !connection.input_ready() {
            return None;
        }
        let snapshot = connection.snapshot.as_ref()?;
        self.history.destination(back, |id| {
            snapshot.panes.iter().any(|pane| pane.pane_id == id)
        })
    }

    fn travel(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((index, pane)) = self.history_destination(back, cx) else {
            return;
        };
        let result = self.connection.update(cx, |connection, cx| {
            let result = connection.focus_pane(&pane);
            cx.notify();
            result
        });
        if result.is_ok() {
            self.history.requested(index);
        }
        self.record(result, cx);
        self.focus_handle.focus(window, cx);
    }

    fn history_buttons(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        let mut pair = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .ml(px(6.0))
            .mr(px(11.0))
            .rounded(px(5.0))
            .overflow_hidden()
            .bg(rgba((p.text << 8) | 0x0f))
            .border_1()
            .border_color(rgba((p.text << 8) | 0x1a));
        for (back, id, glyph, label) in [
            (true, "herdr-back", "‹", "Back"),
            (false, "herdr-forward", "›", "Forward"),
        ] {
            if !back {
                pair = pair.child(div().w(px(1.0)).h(px(14.0)).bg(rgba((p.text << 8) | 0x24)));
            }
            let enabled = self.history_destination(back, cx).is_some();
            let label = self.language.text(label);
            pair = pair.child(
                div()
                    .id(id)
                    .w(px(30.0))
                    .h(px(22.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(18.0))
                    .text_color(rgb(if enabled { p.text } else { p.subtle }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .tooltip(move |_, cx| cx.new(|_| ui::Tooltip(label, p)).into())
                    .when(enabled, |button| {
                        ui::keyboard_control(button, p)
                            .cursor_pointer()
                            .hover(move |s| s.bg(rgba(p.control_hover())))
                            .active(move |s| s.bg(rgba(p.control_pressed())))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                cx.stop_propagation();
                                view.travel(back, window, cx);
                            }))
                    })
                    .child(glyph),
            );
        }
        pair
    }

    pub(super) fn chrome(&self, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        let connection = self.connection.read(cx);
        let enabled = connection.connected();
        let snapshot = connection.snapshot.clone();
        let can_create_tab = enabled
            && snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.workspaces.iter().any(|workspace| {
                    Some(&workspace.workspace_id) == snapshot.focused_workspace_id.as_ref()
                })
            });
        let mut tabs = div()
            .id("herdr-tab-strip")
            .flex()
            .h_full()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll);
        if let Some(snapshot) = snapshot {
            for tab in snapshot
                .tabs
                .iter()
                .filter(|tab| Some(&tab.workspace_id) == snapshot.focused_workspace_id.as_ref())
            {
                let selected = snapshot.focused_tab_id.as_deref() == Some(tab.tab_id.as_str());
                let id = tab.tab_id.clone();
                let close_id = id.clone();
                let close_group = SharedString::from(format!("herdr-strip-close-{id}"));
                let ink = if !enabled {
                    p.subtle
                } else if selected {
                    p.text
                } else {
                    p.muted
                };
                let dot = super::sidebar::status_color(tab.agent_status, p);
                tabs = tabs.child(
                    div()
                        .id(SharedString::from(format!("herdr-strip-{id}")))
                        .h_full()
                        .min_w(px(100.0))
                        .max_w(px(240.0))
                        .flex_shrink_0()
                        .pl(px(12.0))
                        .pr(px(3.0))
                        .py(px(2.0))
                        .border_r_1()
                        .border_color(rgb(p.divider))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .text_color(rgb(ink))
                        .when(selected, |tab| tab.bg(rgba(p.control_selected(false))))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when(enabled, |tab| {
                            ui::choice_navigation(ui::keyboard_control(tab, p))
                                .cursor_pointer()
                                .hover(move |style| {
                                    style.bg(rgba(if selected {
                                        p.control_selected(true)
                                    } else {
                                        p.control_hover()
                                    }))
                                })
                                .active(move |style| style.bg(rgba(p.control_pressed())))
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    cx.stop_propagation();
                                    let result = view.connection.update(cx, |connection, cx| {
                                        let result = connection.focus_tab(&id);
                                        cx.notify();
                                        result
                                    });
                                    view.record(result, cx);
                                    view.focus_handle.focus(window, cx);
                                }))
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .min_w_0()
                                .child(ui::dot(false).bg(rgb(dot)))
                                .child(div().truncate().child(tab.label.clone())),
                        )
                        .child(
                            div()
                                .id(close_group.clone())
                                .group(close_group.clone())
                                .size(px(18.0))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .when(enabled, |button| {
                                    ui::keyboard_control(button, p)
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(rgba(p.control_hover())))
                                        .active(move |s| s.bg(rgba(p.control_pressed())))
                                        .on_click(cx.listener(move |view, _, window, cx| {
                                            cx.stop_propagation();
                                            let result =
                                                view.connection.update(cx, |connection, cx| {
                                                    let result = connection.close_tab(&close_id);
                                                    cx.notify();
                                                    result
                                                });
                                            view.record(result, cx);
                                            view.focus_handle.focus(window, cx);
                                        }))
                                })
                                .child(if enabled {
                                    ui::control_icon("close", p, close_group)
                                        .size(px(12.0))
                                        .text_color(rgb(ink))
                                } else {
                                    ui::icon("close").size(px(12.0)).text_color(rgb(ink))
                                }),
                        ),
                );
            }
        }
        div()
            .h(px(HEADER_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .font_family(self.font.family.clone())
            .text_size(px(12.0))
            .border_b_1()
            .border_color(rgb(p.divider))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(
                div()
                    .w(px(if self.sidebar_visible {
                        SIDEBAR_WIDTH
                    } else {
                        189.0
                    }))
                    .h_full()
                    .flex_shrink_0()
                    .pl(px(80.0))
                    .flex()
                    .items_center()
                    .child(
                        ui::icon_toggle(
                            "herdr-toggle-sidebar",
                            "sidebar",
                            self.language.text("Toggle sidebar · ⌘B"),
                            p,
                            self.sidebar_visible,
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|view, _, _, cx| view.toggle_sidebar(cx))),
                    )
                    .child(self.history_buttons(cx)),
            )
            .child(tabs)
            .child(
                div()
                    .id("herdr-new-tab")
                    .group("herdr-new-tab")
                    .w(px(34.0))
                    .h_full()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_r_1()
                    .border_color(rgb(p.divider))
                    .text_color(rgb(if can_create_tab { p.muted } else { p.subtle }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(can_create_tab, |button| {
                        ui::keyboard_control(button, p)
                            .cursor_pointer()
                            .hover(move |s| s.bg(rgba(p.control_hover())).text_color(rgb(p.text)))
                            .active(move |s| s.bg(rgba(p.control_pressed())))
                            .on_click(cx.listener(|view, _, window, cx| {
                                cx.stop_propagation();
                                let result = view.connection.update(cx, |connection, cx| {
                                    let result = connection.create_tab();
                                    cx.notify();
                                    result
                                });
                                view.record(result, cx);
                                view.focus_handle.focus(window, cx);
                            }))
                    })
                    .child(if can_create_tab {
                        ui::control_icon("plus", p, "herdr-new-tab").size(px(14.0))
                    } else {
                        ui::icon("plus").size(px(14.0)).text_color(rgb(p.subtle))
                    }),
            )
            .child(div().flex_1().min_w(px(40.0)).h_full())
    }

    pub(super) fn status_bar(&self, status: String, cx: &mut Context<Self>) -> Div {
        let p = self.palette;
        div()
            .h(px(FOOTER_HEIGHT))
            .flex_shrink_0()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .border_t_1()
            .border_color(rgb(p.divider))
            .font_family(self.font.family.clone())
            .text_size(px(11.0))
            .text_color(rgb(p.muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(self.language.dynamic_text(&status)),
            )
            .child(
                ui::keyboard_control(div().id("herdr-reconnect"), p)
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgba(p.control_hover())).text_color(rgb(p.text)))
                    .active(move |s| s.bg(rgba(p.control_pressed())))
                    .child(self.language.text("Reconnect"))
                    .on_click(cx.listener(|view, _, _, cx| {
                        cx.stop_propagation();
                        view.connection.update(cx, |connection, cx| {
                            connection.reconnect();
                            cx.notify();
                        });
                        view.error = None;
                        cx.notify();
                    })),
            )
    }
}
