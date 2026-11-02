//! Session-local choices for the Herdr sidebar's visible sections.
use super::HerdrWindow;
use crate::{ui, window_chrome};
use gpui::{AnyElement, Context, FocusHandle, MouseButton, Window, div, prelude::*, px, rgb, rgba};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SidebarView {
    Spaces,
    Agents,
    #[default]
    Both,
}

impl SidebarView {
    pub(super) fn spaces(self) -> bool {
        self != Self::Agents
    }
    pub(super) fn agents(self) -> bool {
        self != Self::Spaces
    }
    pub(super) fn both(self) -> bool {
        self == Self::Both
    }
}

const VIEWS: [(SidebarView, &str); 3] = [
    (SidebarView::Spaces, "Spaces"),
    (SidebarView::Agents, "Agents"),
    (SidebarView::Both, "Spaces and Agents"),
];
const CARD_WIDTH: f32 = 202.0;

pub(super) struct SidebarMenu {
    focus: FocusHandle,
    items: [FocusHandle; 3],
}

impl HerdrWindow {
    pub(super) fn open_sidebar_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_menu = None;
        self.sidebar_drag = None;
        self.selecting = false;
        let menu = SidebarMenu {
            focus: cx.focus_handle(),
            items: std::array::from_fn(|_| cx.focus_handle().tab_index(0).tab_stop(true)),
        };
        let index = VIEWS
            .iter()
            .position(|(view, _)| *view == self.sidebar_view)
            .unwrap_or(2);
        menu.items[index].focus(window, cx);
        self.sidebar_menu = Some(menu);
        cx.notify();
    }

    fn dismiss_sidebar_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_sidebar_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.sidebar_menu.as_ref()?;
        let p = self.palette.herdr_sidebar();
        let visible = window_chrome::show_sidebar(
            self.sidebar_visible,
            f32::from(window.viewport_size().width),
        );
        let width = window_chrome::sidebar_width(
            self.sidebar_width,
            f32::from(window.viewport_size().width),
            window.scale_factor(),
        );
        let left = if visible {
            (width - CARD_WIDTH).max(8.0)
        } else {
            8.0
        };
        let mut card = div()
            .absolute()
            .left(px(left))
            .top(px(39.0))
            .w(px(CARD_WIDTH))
            .p_1()
            .rounded(px(8.0))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .text_color(rgb(p.muted))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
        for (index, (view, label)) in VIEWS.into_iter().enumerate() {
            card = card.child(
                ui::keyboard_control(div().id(("herdr-sidebar-view-option", index)), p)
                    .track_focus(&menu.items[index])
                    .h(px(30.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(5.0))
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgba(p.control_hover())))
                    .active(move |style| style.bg(rgba(p.control_pressed())))
                    .child(
                        div()
                            .size(px(14.0))
                            .when(view == self.sidebar_view, |mark| {
                                mark.child(ui::icon("check").size(px(14.0)).text_color(rgb(p.text)))
                            }),
                    )
                    .child(label)
                    .on_click(cx.listener(move |window_view, _, window, cx| {
                        window_view.sidebar_view = view;
                        window_view.sidebar_visible = true;
                        window_view.sidebar_drag = None;
                        window_view.dismiss_sidebar_menu(window, cx);
                    })),
            );
        }
        Some(
            ui::block_terminal_actions(div().id("herdr-sidebar-view-overlay"))
                .absolute()
                .inset_0()
                .occlude()
                .track_focus(&menu.focus)
                .tab_group()
                .tab_stop(false)
                .capture_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, window, cx| {
                    let key = &event.keystroke;
                    if key.modifiers.control || key.modifiers.alt || key.modifiers.platform {
                        return;
                    }
                    if key.key == "escape" {
                        view.dismiss_sidebar_menu(window, cx);
                        cx.stop_propagation();
                    } else if matches!(key.key.as_str(), "up" | "down" | "tab") {
                        if let Some(menu) = &view.sidebar_menu {
                            let index = menu
                                .items
                                .iter()
                                .position(|item| item.is_focused(window))
                                .unwrap_or(0);
                            let backwards = key.key == "up" || key.modifiers.shift;
                            menu.items[(index + if backwards { 2 } else { 1 }) % 3]
                                .focus(window, cx);
                        }
                        cx.stop_propagation();
                    }
                }))
                .on_key_down(|_, _, cx| cx.stop_propagation())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, window, cx| {
                        view.dismiss_sidebar_menu(window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|view, _, window, cx| {
                        view.dismiss_sidebar_menu(window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(card)
                .into_any_element(),
        )
    }
}
