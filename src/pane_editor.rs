use crate::{
    appearance::Palette, layout::custom_title, locale::Language, text_input::TextInput, ui,
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, Render, Subscription, Window, actions, div, prelude::*, px, rgb, rgba,
};

actions!(pane_editor, [Save, Cancel]);

pub enum EditorEvent {
    Save(Option<String>),
    Cancel,
}

pub struct PaneEditor {
    title: Entity<TextInput>,
    palette: Palette,
    language: Language,
    save_focus: FocusHandle,
    pending: bool,
    error: Option<String>,
    _focus_subscriptions: Vec<Subscription>,
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Save, Some("PaneEditor > MonoTermTextInput")),
        KeyBinding::new("escape", Cancel, Some("PaneEditor")),
    ]);
}

impl EventEmitter<EditorEvent> for PaneEditor {}

impl PaneEditor {
    pub fn new(
        title: Option<String>,
        default_title: String,
        palette: Palette,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = cx.new(|cx| TextInput::new(title.unwrap_or_default(), default_title, cx));
        let focus = title.focus_handle(cx);
        let subscriptions = vec![
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        Self {
            title,
            palette,
            language,
            save_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            pending: false,
            error: None,
            _focus_subscriptions: subscriptions,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.title.update(cx, |title, cx| title.focus(window, cx));
    }

    pub fn set_style(&mut self, palette: Palette, language: Language, cx: &mut Context<Self>) {
        if self.palette != palette || self.language != language {
            self.palette = palette;
            self.language = language;
            cx.notify();
        }
    }

    pub fn set_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        self.pending = pending;
        if pending {
            self.error = None;
        }
        cx.notify();
    }

    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.pending = false;
        self.error = Some(error);
        cx.notify();
    }

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if !self.pending && !self.title.read(cx).has_marked_text() {
            let title = custom_title(self.title.read(cx).value());
            self.set_pending(true, cx);
            cx.emit(EditorEvent::Save(title));
        }
        cx.stop_propagation();
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.title.read(cx).has_marked_text() {
            cx.emit(EditorEvent::Cancel);
        }
        cx.stop_propagation();
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        primary: bool,
    ) -> gpui::Stateful<gpui::Div> {
        let palette = self.palette;
        ui::keyboard_control(div().id(id), palette)
            .h(px(30.0))
            .px_3()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(12.0))
            .text_color(rgb(if primary {
                palette.accent
            } else {
                palette.text
            }))
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(if primary {
                palette.accent
            } else {
                palette.border
            }))
            .hover(move |style| style.bg(rgba(palette.control_hover())))
            .active(move |style| style.bg(rgba(palette.control_pressed())))
            .child(self.language.text(label))
    }
}

impl Render for PaneEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let language = self.language;
        let title_focused = self.title.read(cx).is_focused(window);
        ui::block_terminal_actions(div().id("pane-editor-overlay"))
            .absolute()
            .inset_0()
            .occlude()
            .key_context("PaneEditor")
            .tab_group()
            .capture_key_down(
                cx.listener(|editor, event: &gpui::KeyDownEvent, window, cx| {
                    let stroke = &event.keystroke;
                    if stroke.key != "tab"
                        || stroke.modifiers.control
                        || stroke.modifiers.alt
                        || stroke.modifiers.platform
                        || editor.title.read(cx).has_marked_text()
                    {
                        return;
                    }
                    if stroke.modifiers.shift && editor.title.read(cx).is_focused(window) {
                        editor.save_focus.focus(window, cx);
                    } else if !stroke.modifiers.shift && editor.save_focus.is_focused(window) {
                        editor.focus(window, cx);
                    } else {
                        ui::traverse_controls(event, window, cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000048))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("pane-editor-card")
                    .w(px(408.0))
                    .max_w(window.viewport_size().width - px(32.0))
                    .max_h(window.viewport_size().height - px(32.0))
                    .overflow_y_scroll()
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .rounded(px(12.0))
                    .bg(rgb(palette.panel))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .text_color(rgb(palette.text))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(language.text("Rename pane")),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(palette.muted))
                            .child(language.text("Pane name")),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(if title_focused {
                                palette.accent
                            } else {
                                palette.border
                            }))
                            .bg(rgb(palette.terminal))
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(palette.muted))
                            .child(language.text("Leave blank to use the automatic name.")),
                    )
                    .when_some(self.error.as_ref(), |card, error| {
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
                            .gap_2()
                            .items_center()
                            .child(
                                self.button("pane-editor-reset", "Restore default", false)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        if !editor.pending {
                                            editor.title.update(cx, |title, cx| {
                                                title.set_value(String::new(), cx)
                                            });
                                            editor.error = None;
                                            editor.focus(window, cx);
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(div().flex_1())
                            .child(self.button("pane-editor-cancel", "Cancel", false).on_click(
                                cx.listener(|editor, _, window, cx| {
                                    editor.cancel(&Cancel, window, cx)
                                }),
                            ))
                            .child(
                                self.button(
                                    "pane-editor-save",
                                    if self.pending { "Saving…" } else { "Save" },
                                    true,
                                )
                                .track_focus(&self.save_focus)
                                .when(self.pending, |button| button.opacity(0.5))
                                .on_click(cx.listener(
                                    |editor, _, window, cx| editor.save(&Save, window, cx),
                                )),
                            ),
                    ),
            )
    }
}
