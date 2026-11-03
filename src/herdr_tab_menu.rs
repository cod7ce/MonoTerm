use super::HerdrWindow;
use crate::{herdr_connection::TabTarget, text_input::TextInput, ui};
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, FontWeight, KeyDownEvent, MouseButton,
    Pixels, Point, Window, div, prelude::*, px, rgb, rgba,
};

pub(super) struct TabMenu {
    target: TabTarget,
    position: Point<Pixels>,
    focus: FocusHandle,
    items: [FocusHandle; 2],
    input: Option<Entity<TextInput>>,
    error: Option<String>,
}

impl HerdrWindow {
    pub(super) fn open_tab_menu(
        &mut self,
        target: TabTarget,
        position: Point<Pixels>,
        rename: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_menu = None;
        self.sidebar_menu = None;
        self.sidebar_drag = None;
        self.selecting = false;
        self.cancel_links();
        let menu = TabMenu {
            target,
            position,
            focus: cx.focus_handle(),
            items: std::array::from_fn(|_| cx.focus_handle().tab_index(0).tab_stop(true)),
            input: None,
            error: None,
        };
        menu.items[0].focus(window, cx);
        self.tab_menu = Some(menu);
        if rename {
            self.edit_tab_name(window, cx);
        }
        cx.notify();
    }

    pub(super) fn rename_focused_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions_available(cx) {
            return;
        }
        let target = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .tabs
                    .iter()
                    .find(|tab| Some(&tab.tab_id) == snapshot.focused_tab_id.as_ref())
                    .map(|tab| TabTarget::new(snapshot, tab))
            });
        if let Some(target) = target {
            self.open_tab_menu(target, window.mouse_position(), true, window, cx);
        }
    }

    fn dismiss_tab_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn edit_tab_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &mut self.tab_menu else {
            return;
        };
        let input = cx.new(|cx| TextInput::new(menu.target.tab.label.clone(), String::new(), cx));
        input.update(cx, |input, cx| input.focus(window, cx));
        menu.input = Some(input);
        menu.error = None;
        cx.notify();
    }

    fn save_tab_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.tab_menu else {
            return;
        };
        let Some(input) = &menu.input else {
            return;
        };
        if input.read(cx).has_marked_text() {
            return;
        }
        let target = menu.target.clone();
        let label = input.read(cx).value().to_owned();
        let result = self.connection.update(cx, |connection, cx| {
            let result = connection.rename_tab(&target, &label);
            cx.notify();
            result
        });
        match result {
            Ok(()) => self.dismiss_tab_menu(window, cx),
            Err(error) => {
                if let Some(menu) = &mut self.tab_menu {
                    menu.error = Some(error);
                }
                cx.notify();
            }
        }
    }

    fn tab_menu_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &self.tab_menu else {
            return;
        };
        let key = &event.keystroke;
        if key.is_ime_in_progress()
            || menu
                .input
                .as_ref()
                .is_some_and(|input| input.read(cx).has_marked_text())
            || key.modifiers.control
            || key.modifiers.alt
            || key.modifiers.platform
        {
            return;
        }
        if event.is_held && matches!(key.key.as_str(), "enter" | "return" | "space") {
            cx.stop_propagation();
            window.prevent_default();
            return;
        }
        if key.key == "escape" {
            self.dismiss_tab_menu(window, cx);
            cx.stop_propagation();
            window.prevent_default();
        } else if key.key == "tab"
            || (menu.input.is_none() && matches!(key.key.as_str(), "up" | "down"))
        {
            let mut handles = Vec::new();
            if let Some(input) = &menu.input {
                handles.push(input.focus_handle(cx));
                handles.extend(menu.items.iter().cloned());
            } else {
                handles.push(menu.items[0].clone());
            }
            let backwards = key.modifiers.shift || key.key == "up";
            let current = handles.iter().position(|handle| handle.is_focused(window));
            let index = match current {
                Some(index) if backwards => (index + handles.len() - 1) % handles.len(),
                Some(index) => (index + 1) % handles.len(),
                None if backwards => handles.len() - 1,
                None => 0,
            };
            handles[index].focus(window, cx);
            cx.stop_propagation();
            window.prevent_default();
        } else if matches!(key.key.as_str(), "enter" | "return")
            && menu
                .input
                .as_ref()
                .is_some_and(|input| input.read(cx).is_focused(window))
        {
            self.save_tab_name(window, cx);
            cx.stop_propagation();
            window.prevent_default();
        }
    }

    pub(super) fn render_tab_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let menu = self.tab_menu.as_ref()?;
        let palette = self.palette.herdr_sidebar();
        let language = self.language;
        let editing = menu.input.is_some();
        let item = |index, label| {
            ui::keyboard_control(div().id(("herdr-tab-menu-item", index)), palette)
                .track_focus(&menu.items[index])
                .h(px(30.0))
                .px_3()
                .rounded(px(5.0))
                .flex()
                .items_center()
                .text_size(px(13.0))
                .cursor_pointer()
                .hover(move |style| style.bg(rgba(palette.control_hover())))
                .active(move |style| style.bg(rgba(palette.control_pressed())))
                .child(language.text(label))
        };
        let mut card = div()
            .id("herdr-tab-menu-card")
            .p_2()
            .rounded(px(10.0))
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .text_color(rgb(palette.text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation());
        if let Some(input) = &menu.input {
            card = card
                .w(px(420.0))
                .max_w(window.viewport_size().width - px(32.0))
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(language.text("Rename tab")),
                )
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded(px(5.0))
                        .border_1()
                        .border_color(rgb(palette.muted))
                        .bg(rgb(palette.terminal))
                        .child(input.clone()),
                )
                .when_some(menu.error.as_ref(), |card, error| {
                    card.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(if palette.dark { 0xef8888 } else { 0xb33131 }))
                            .whitespace_normal()
                            .child(language.dynamic_text(error)),
                    )
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(item(0, "Cancel").on_click(
                            cx.listener(|view, _, window, cx| view.dismiss_tab_menu(window, cx)),
                        ))
                        .child(item(1, "Save").on_click(
                            cx.listener(|view, _, window, cx| view.save_tab_name(window, cx)),
                        )),
                );
        } else {
            card =
                card.absolute()
                    .w(px(220.0))
                    .left(
                        menu.position
                            .x
                            .min(window.viewport_size().width - px(228.0))
                            .max(px(8.0)),
                    )
                    .top(
                        menu.position
                            .y
                            .min(window.viewport_size().height - px(54.0))
                            .max(px(40.0)),
                    )
                    .child(item(0, "Rename tab…").on_click(
                        cx.listener(|view, _, window, cx| view.edit_tab_name(window, cx)),
                    ));
        }
        Some(
            ui::block_terminal_actions(div().id("herdr-tab-menu-overlay"))
                .absolute()
                .inset_0()
                .occlude()
                .track_focus(&menu.focus)
                .tab_group()
                .tab_stop(false)
                .when(editing, |overlay| {
                    overlay
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(0x00000048))
                })
                .capture_key_down(cx.listener(Self::tab_menu_key_down))
                .on_key_down(cx.listener(|view, _, window, cx| {
                    // Ordinary keys must reach the native text insertion callback.
                    let editing = view
                        .tab_menu
                        .as_ref()
                        .and_then(|menu| menu.input.as_ref())
                        .is_some_and(|input| input.read(cx).is_focused(window));
                    if !editing {
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, window, cx| {
                        if !editing {
                            view.dismiss_tab_menu(window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |view, _, window, cx| {
                        if !editing {
                            view.dismiss_tab_menu(window, cx);
                        }
                        cx.stop_propagation();
                    }),
                )
                .child(card)
                .into_any_element(),
        )
    }
}
