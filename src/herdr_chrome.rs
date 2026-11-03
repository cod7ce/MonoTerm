//! Tab navigation adapted from Herdr UI at 38ea4248d2c64879f02053b419bdd5386eebce10.
//! Copyright 2026 Fabien Penso. Apache-2.0; see vendor/herdr/NOTICE.
use super::{HerdrWindow, indicators};
use crate::{
    appearance::Palette,
    herdr_connection::TabTarget,
    ui,
    window_chrome::{self, SHELL_INSET, TOOLBAR_CENTER, TOOLBAR_HEIGHT},
};
use gpui::{
    Context, Div, MouseButton, ScrollHandle, SharedString, Window, div, prelude::*, px, rgb, rgba,
};
use herdr_protocol::{AgentStatus, ClientShellSnapshot, ClientShellTab};

#[derive(PartialEq, Eq)]
struct TabSelection {
    boot_id: String,
    workspace_id: String,
    tab_id: String,
    pane_id: Option<String>,
    index: usize,
}

impl TabSelection {
    fn from_snapshot(snapshot: &ClientShellSnapshot) -> Option<Self> {
        let workspace_id = snapshot.focused_workspace_id.as_ref()?;
        let tab_id = snapshot.focused_tab_id.as_ref()?;
        let index = snapshot
            .tabs
            .iter()
            .filter(|tab| &tab.workspace_id == workspace_id)
            .position(|tab| &tab.tab_id == tab_id)?;
        Some(Self {
            boot_id: snapshot.boot_id.clone(),
            workspace_id: workspace_id.clone(),
            tab_id: tab_id.clone(),
            pane_id: snapshot.focused_pane_id.clone(),
            index,
        })
    }
}

#[derive(Default)]
pub(super) struct TabReveal {
    selection: Option<TabSelection>,
    attempts: u8,
}

impl TabReveal {
    fn request(&mut self, selection: Option<TabSelection>, visible: bool) -> Option<usize> {
        const MAX_ATTEMPTS: u8 = 4;
        if self.selection != selection {
            self.selection = selection;
            self.attempts = 0;
        } else if self.attempts > 0 && visible {
            self.attempts = MAX_ATTEMPTS;
        }
        let selection = self.selection.as_ref()?;
        if self.attempts >= MAX_ATTEMPTS {
            return None;
        }
        self.attempts += 1;
        Some(selection.index)
    }
}

fn tab_is_visible(handle: &ScrollHandle, index: usize) -> bool {
    let Some(tab) = handle.bounds_for_item(index) else {
        return false;
    };
    let viewport = handle.bounds();
    let offset = handle.offset().x;
    let tolerance = px(0.5);
    viewport.size.width > px(0.0)
        && tab.left() + offset >= viewport.left() - tolerance
        && (tab.right() + offset <= viewport.right() + tolerance
            || tab.size.width > viewport.size.width)
}

fn tab_icon(snapshot: &ClientShellSnapshot, tab: &ClientShellTab) -> &'static str {
    let mut agents = snapshot
        .agents
        .iter()
        .filter(|agent| agent.tab_id == tab.tab_id);
    let agent = agents
        .clone()
        .find(|agent| {
            snapshot
                .panes
                .iter()
                .any(|pane| pane.pane_id == agent.pane_id && pane.focused)
        })
        .or_else(|| agents.next());
    match agent.and_then(|agent| agent.agent.as_deref()) {
        Some(kind) if kind.eq_ignore_ascii_case("claude") => "agent-claude",
        Some(kind) if kind.eq_ignore_ascii_case("codex") => "agent-codex",
        _ => "terminal-window",
    }
}

struct TabTooltip(String, Palette);
impl gpui::Render for TabTooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(5.0))
            .bg(rgb(self.1.panel))
            .border_1()
            .border_color(rgb(self.1.border))
            .text_color(rgb(self.1.text))
            .text_size(px(11.0))
            .child(self.0.clone())
    }
}

impl HerdrWindow {
    pub(super) fn chrome(
        &mut self,
        show_sidebar: bool,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = self.palette.herdr_sidebar();
        let connection = self.connection.read(cx);
        let enabled = connection.connected();
        let snapshot = connection.snapshot.clone();
        let selection = snapshot.as_deref().and_then(TabSelection::from_snapshot);
        let visible = selection
            .as_ref()
            .is_some_and(|selection| tab_is_visible(&self.tab_scroll, selection.index));
        if let Some(index) = self.tab_reveal.request(selection, visible) {
            self.tab_scroll.scroll_to_item(index);
            // GPUI uses the previous viewport bounds while revealing an item.
            // Another frame settles a newly mounted or resized tab strip.
            cx.on_next_frame(window, |_, _, cx| cx.notify());
        }
        let snapshot = snapshot.as_ref();
        let can_create_tab = enabled
            && snapshot.is_some_and(|snapshot| {
                snapshot.workspaces.iter().any(|workspace| {
                    Some(&workspace.workspace_id) == snapshot.focused_workspace_id.as_ref()
                })
            });
        let offset = px(TOOLBAR_CENTER - (SHELL_INSET + TOOLBAR_HEIGHT / 2.0));
        let mut tabs = div()
            .id("herdr-tab-strip")
            .relative()
            .top(offset)
            .flex()
            .flex_initial()
            .min_w_0()
            .h(px(28.0))
            .gap(px(4.0))
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
                let icon_id = SharedString::from(format!("herdr-strip-icon-{id}"));
                let zoom_tooltip = self.language.text("Pane maximized · ⌘⇧↵ to restore");
                let menu_target = TabTarget::new(snapshot, tab);
                let rename_target = menu_target.clone();
                let label = herdr_client::text::safe_text(&tab.label, 160);
                let tooltip = label.clone();
                let group = SharedString::from(format!("herdr-strip-group-{id}"));
                let foreground = if !enabled {
                    p.subtle
                } else if selected {
                    p.text
                } else {
                    p.muted
                };
                tabs = tabs.child(
                    div()
                        .id(SharedString::from(format!("herdr-strip-{id}")))
                        .group(group.clone())
                        .h_full()
                        .max_w(px(208.0))
                        .flex_shrink_0()
                        .pl(px(9.0))
                        .pr(px(5.0))
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .rounded(px(7.0))
                        .bg(rgba(p.tab_background(selected, false)))
                        .tooltip(move |_, cx| cx.new(|_| TabTooltip(tooltip.clone(), p)).into())
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when(enabled, |tab| {
                            ui::choice_navigation(ui::keyboard_control(tab, p))
                                .cursor_pointer()
                                .hover(move |style| {
                                    style.bg(rgba(p.tab_background(selected, true)))
                                })
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |view, event: &gpui::MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        view.open_tab_menu(
                                            menu_target.clone(),
                                            event.position,
                                            false,
                                            window,
                                            cx,
                                        );
                                    }),
                                )
                                .on_click(cx.listener(move |view, event, window, cx| {
                                    cx.stop_propagation();
                                    if matches!(event, gpui::ClickEvent::Mouse(click) if click.up.click_count == 2) {
                                        view.open_tab_menu(
                                            rename_target.clone(),
                                            event.position(),
                                            true,
                                            window,
                                            cx,
                                        );
                                        return;
                                    }
                                    let result = view.connection.update(cx, |connection, cx| {
                                        let result = connection.focus_tab(&id);
                                        cx.notify();
                                        result
                                    });
                                    view.record(result, cx);
                                    view.focus_handle.focus(window, cx);
                                }))
                        })
                        .child({
                            let frame = ui::tab_icon_frame(
                                icon_id,
                                tab_icon(snapshot, tab),
                                15.0,
                                tab.zoomed,
                                rgb(foreground),
                                rgb(if enabled { p.text } else { p.subtle }),
                            );
                            if tab.zoomed {
                                frame.tooltip(move |_, cx| {
                                    cx.new(|_| TabTooltip(zoom_tooltip.into(), p)).into()
                                })
                            } else {
                                frame
                            }
                        })
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(14.0))
                                .font_weight(if selected {
                                    gpui::FontWeight::MEDIUM
                                } else {
                                    gpui::FontWeight::NORMAL
                                })
                                .text_color(rgb(foreground))
                                .child(label),
                        )
                        .child(
                            div()
                                .relative()
                                .size(px(18.0))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(tab.agent_status != AgentStatus::Unknown, |slot| {
                                    slot.child(
                                        div()
                                            .group_hover(group.clone(), |style| style.opacity(0.0))
                                            .child(indicators::mark(
                                                tab.agent_status,
                                                self.indicator_elapsed,
                                                if enabled {
                                                    indicators::color(tab.agent_status, p)
                                                } else {
                                                    p.subtle
                                                },
                                            )),
                                    )
                                })
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "herdr-strip-close-{close_id}"
                                        )))
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(4.0))
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .when(enabled, |button| {
                                            ui::keyboard_control(button, p)
                                                .cursor_pointer()
                                                .hover(move |style| {
                                                    style.bg(rgba(p.control_hover()))
                                                })
                                                .on_click(cx.listener(
                                                    move |view, _, window, cx| {
                                                        cx.stop_propagation();
                                                        let result = view.connection.update(
                                                            cx,
                                                            |connection, cx| {
                                                                let result =
                                                                    connection.close_tab(&close_id);
                                                                cx.notify();
                                                                result
                                                            },
                                                        );
                                                        view.record(result, cx);
                                                        view.focus_handle.focus(window, cx);
                                                    },
                                                ))
                                        })
                                        .opacity(0.0)
                                        .group_hover(group, |style| style.opacity(1.0))
                                        .focus_visible(move |style| {
                                            style
                                                .opacity(1.0)
                                                .bg(rgb(p.panel))
                                                .border_1()
                                                .border_color(rgb(p.accent))
                                        })
                                        .child(
                                            ui::icon("close")
                                                .size(px(12.0))
                                                .text_color(rgb(foreground)),
                                        ),
                                ),
                        ),
                );
            }
        }
        window_chrome::toolbar()
            .child(
                window_chrome::sidebar_toggle_slot(show_sidebar, width)
                    .when(!show_sidebar, |slot| slot.w(px(128.0)))
                    .child(
                        ui::icon_toggle(
                            "herdr-toggle-sidebar",
                            "sidebar",
                            self.language.text("Toggle sidebar · ⌘B"),
                            p,
                            self.sidebar_visible,
                        )
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.toggle_sidebar(cx);
                            view.focus_handle.focus(window, cx);
                        })),
                    )
                    .child(
                        ui::icon_button(
                            "herdr-sidebar-view",
                            "chevron-down",
                            self.language.text("Sidebar view"),
                            p,
                        )
                        .w(px(18.0))
                        .on_click(
                            cx.listener(|view, _, window, cx| view.open_sidebar_menu(window, cx)),
                        ),
                    ),
            )
            .child(tabs)
            .child(
                div().relative().top(offset).flex_none().child(
                    ui::icon_button_enabled(
                        "herdr-new-tab",
                        "plus",
                        self.language.text("New tab"),
                        p,
                        can_create_tab,
                    )
                    .when(can_create_tab, |button| {
                        button.on_click(cx.listener(|view, _, window, cx| {
                            let result = view.connection.update(cx, |connection, cx| {
                                let result = connection.create_tab();
                                cx.notify();
                                result
                            });
                            view.record(result, cx);
                            view.focus_handle.focus(window, cx);
                        }))
                    }),
                ),
            )
            .child(div().flex_1().min_w(px(32.0)).h_full().on_mouse_down(
                MouseButton::Left,
                |event, window, _| {
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    } else {
                        window.start_window_move();
                    }
                },
            ))
            .child(
                div()
                    .relative()
                    .top(offset)
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        ui::icon_button(
                            "herdr-settings",
                            "settings",
                            self.language.text("Settings · ⌘,"),
                            p,
                        )
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.owner
                                .update(cx, |root, cx| root.open_settings(window, cx));
                        })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_reveal_tracks_selection_without_snapping_on_metadata_updates() {
        let mut snapshot: ClientShellSnapshot = serde_json::from_str(include_str!(
            "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
        ))
        .expect("valid protocol fixture");
        let mut next = snapshot.tabs[0].clone();
        next.tab_id = "w1:t2".into();
        snapshot.tabs.push(next);
        let mut reveal = TabReveal::default();
        let selected = || TabSelection::from_snapshot(&snapshot);
        assert_eq!(reveal.request(selected(), false), Some(0));
        assert_eq!(reveal.request(selected(), false), Some(0));
        assert_eq!(reveal.request(selected(), true), None);

        // A user may scroll away after the selection has been revealed.
        snapshot.revision += 1;
        snapshot.tabs[0].label = "Updated title".into();
        snapshot.tabs[0].agent_status = AgentStatus::Unknown;
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), false),
            None
        );
        snapshot.focused_tab_id = Some("w1:t2".into());
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), false),
            Some(1)
        );
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), true),
            None
        );
        snapshot.focused_pane_id = Some("w1:p2".into());
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), false),
            Some(1)
        );
    }

    #[test]
    fn tab_reveal_uses_workspace_indices_and_bounds_layout_retries() {
        let mut snapshot: ClientShellSnapshot = serde_json::from_str(include_str!(
            "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
        ))
        .expect("valid protocol fixture");
        let mut other = snapshot.tabs[0].clone();
        other.workspace_id = "w2".into();
        other.tab_id = "w2:t1".into();
        snapshot.tabs.insert(0, other);
        let mut reveal = TabReveal::default();
        for _ in 0..4 {
            assert_eq!(
                reveal.request(TabSelection::from_snapshot(&snapshot), false),
                Some(0)
            );
        }
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), false),
            None
        );
        snapshot.boot_id = "new-boot".into();
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), false),
            Some(0)
        );
        snapshot.focused_tab_id = Some("missing".into());
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), false),
            None
        );
        snapshot.focused_workspace_id = Some("w2".into());
        snapshot.focused_tab_id = Some("w2:t1".into());
        assert_eq!(
            reveal.request(TabSelection::from_snapshot(&snapshot), false),
            Some(0)
        );
    }

    #[test]
    fn tab_icons_follow_the_tabs_own_agents() {
        let mut snapshot: ClientShellSnapshot = serde_json::from_str(include_str!(
            "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
        ))
        .expect("valid protocol fixture");
        let tab = snapshot.tabs[0].clone();
        snapshot.agents[0].agent = Some("claude".into());
        assert_eq!(tab_icon(&snapshot, &tab), "agent-claude");
        let mut second = snapshot.agents[0].clone();
        second.pane_id = "w1:p2".into();
        second.agent = Some("codex".into());
        snapshot.agents.push(second);
        snapshot.panes[0].pane_id = "w1:p2".into();
        snapshot.panes[0].focused = true;
        assert_eq!(tab_icon(&snapshot, &tab), "agent-codex");
        snapshot
            .agents
            .iter_mut()
            .for_each(|agent| agent.tab_id = "other-tab".into());
        assert_eq!(tab_icon(&snapshot, &tab), "terminal-window");
    }
}
