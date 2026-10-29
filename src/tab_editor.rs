use crate::{
    appearance::Palette,
    layout::{TabAppearance, TabIcon},
    locale::Language,
    text_input::TextInput,
    ui,
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyBinding,
    MouseButton, Render, Subscription, Window, actions, div, prelude::*, px, rgb, rgba,
};

actions!(tab_editor, [Save, Cancel]);

pub enum EditorEvent {
    Save(TabAppearance),
    Cancel,
}

pub struct TabEditor {
    pub tab_id: usize,
    title: Entity<TextInput>,
    selected_icon: Option<TabIcon>,
    palette: Palette,
    language: Language,
    save_focus: FocusHandle,
    _focus_subscriptions: Vec<Subscription>,
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Save, Some("TabEditor > MonoTermTextInput")),
        KeyBinding::new("escape", Cancel, Some("TabEditor")),
        KeyBinding::new("cmd-w", Cancel, Some("TabEditor")),
    ]);
}

impl EventEmitter<EditorEvent> for TabEditor {}

impl TabEditor {
    pub fn new(
        tab_id: usize,
        appearance: &TabAppearance,
        default_title: String,
        palette: Palette,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = cx.new(|cx| {
            TextInput::new(
                appearance.title.clone().unwrap_or_default(),
                default_title,
                cx,
            )
        });
        let focus = title.focus_handle(cx);
        let subscriptions = vec![
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        Self {
            tab_id,
            title,
            selected_icon: appearance.icon,
            palette,
            language,
            save_focus: cx.focus_handle().tab_index(0).tab_stop(true),
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

    fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        if !self.title.read(cx).has_marked_text() {
            cx.emit(EditorEvent::Save(TabAppearance::new(
                self.title.read(cx).value(),
                self.selected_icon,
            )));
        }
        cx.stop_propagation();
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        // Escape belongs to the IME until composition has finished.
        if !self.title.read(cx).has_marked_text() {
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
        let p = self.palette;
        ui::keyboard_control(div().id(id), p)
            .h(px(30.0))
            .px_3()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .text_size(px(12.0))
            .text_color(rgb(if primary { p.accent } else { p.text }))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(if primary { p.accent } else { p.border }))
            .hover(move |style| style.bg(rgba(p.control_hover())))
            .active(move |style| style.bg(rgba(p.control_pressed())))
            .child(self.language.text(label))
    }
}

impl Render for TabEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let compact = window.viewport_size().height < px(480.0);
        let title_focused = self.title.read(cx).is_focused(window);
        let t = |label| self.language.text(label);
        let mut icons = div().flex().flex_wrap().gap(px(6.0));
        for (index, choice) in std::iter::once(None)
            .chain(TabIcon::ALL.into_iter().map(Some))
            .enumerate()
        {
            let selected = self.selected_icon == choice;
            let label = t(choice.map_or("Default", TabIcon::label));
            icons = icons.child(
                ui::keyboard_control(div().id(("tab-icon-choice", index)), p)
                    .map(ui::choice_navigation)
                    .w(px(116.0))
                    .h(px(if compact { 30.0 } else { 34.0 }))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .border_1()
                    .border_color(rgb(if selected { p.accent } else { p.border }))
                    .when(selected, |choice| {
                        choice
                            .bg(rgba(p.control_selected(false)))
                            .hover(move |style| style.bg(rgba(p.control_selected(true))))
                    })
                    .when(!selected, |choice| {
                        choice
                            .bg(rgb(p.panel))
                            .hover(move |style| style.bg(rgba(p.control_hover())))
                    })
                    .active(move |style| style.bg(rgba(p.control_pressed())))
                    .child(
                        ui::icon(choice.map_or("minus", TabIcon::name))
                            .text_color(rgb(if selected { p.accent } else { p.muted })),
                    )
                    .child(div().text_size(px(11.0)).child(label))
                    .on_click(cx.listener(move |editor, _, _, cx| {
                        editor.selected_icon = choice;
                        cx.notify();
                    })),
            );
        }
        ui::block_terminal_actions(div().id("tab-editor-overlay"))
            .absolute()
            .inset_0()
            .occlude()
            .key_context("TabEditor")
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
                    .id("tab-editor-card")
                    .w(px(408.0))
                    .max_h(window.viewport_size().height - px(32.0))
                    .overflow_y_scroll()
                    .p(px(if compact { 16.0 } else { 20.0 }))
                    .flex()
                    .flex_col()
                    .gap(px(if compact { 6.0 } else { 10.0 }))
                    .rounded(px(12.0))
                    .bg(rgb(p.panel))
                    .border_1()
                    .border_color(rgb(p.border))
                    .text_color(rgb(p.text))
                    .child(
                        div()
                            .text_size(px(17.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(t("Customize tab")),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(p.muted))
                            .child(t("Title")),
                    )
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(if title_focused { p.accent } else { p.border }))
                            .bg(rgb(p.terminal))
                            .child(self.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(p.muted))
                            .child(t("Leave blank to use the automatic title.")),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(p.muted))
                            .child(t("Icon")),
                    )
                    .child(icons)
                    .when(!compact, |card| {
                        card.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgb(p.muted))
                                .child(t("Applies to this tab for the current session.")),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(
                                self.button("tab-editor-reset", "Restore default", false)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.selected_icon = None;
                                        editor.title.update(cx, |title, cx| {
                                            title.set_value(String::new(), cx)
                                        });
                                        editor.focus(window, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .child(self.button("tab-editor-cancel", "Cancel", false).on_click(
                                cx.listener(|editor, _, window, cx| {
                                    editor.cancel(&Cancel, window, cx)
                                }),
                            ))
                            .child(
                                self.button("tab-editor-save", "Save", true)
                                    .track_focus(&self.save_focus)
                                    .on_click(cx.listener(|editor, _, window, cx| {
                                        editor.save(&Save, window, cx)
                                    })),
                            ),
                    ),
            )
    }
}
