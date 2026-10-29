use crate::typography::FontFamily;
use crate::{
    CloseSession, CloseTab, TerminalApp, ToggleSettings,
    appearance::{Appearance, Palette, Theme},
    locale::Language,
    ui::icon,
};
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, MouseButton, Pixels, Render,
    SharedString, Subscription, Window, WindowControlArea, WindowHandle, actions, canvas, div,
    prelude::*, px, rgb, rgba,
};
use std::collections::HashMap;

actions!(settings, [CloseSettings]);

/// Only preferences belong here: terminal output must not redraw Settings.
#[derive(Clone, PartialEq)]
pub(crate) struct SettingsSnapshot {
    appearance: Appearance,
    palette: Palette,
    font_size: f32,
    font_family: FontFamily,
    language: Language,
    sidebar: bool,
    zen: bool,
    feedback: Option<String>,
    follow: crate::follow::FollowSettings,
    follow_status: String,
    socket_path: String,
}

impl From<&TerminalApp> for SettingsSnapshot {
    fn from(root: &TerminalApp) -> Self {
        Self {
            appearance: root.appearance,
            palette: root.palette,
            font_size: root.font_size,
            font_family: root.typography.font_family,
            language: root.language,
            sidebar: root.sidebar,
            zen: root.zen,
            feedback: root.appearance_notice.clone(),
            follow: root.follow_settings.clone(),
            follow_status: root.follow_status.clone(),
            socket_path: root.follow_settings.socket_path(),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Category {
    Appearance,
    Window,
    Terminal,
    Shortcuts,
    Integrations,
    Version,
}

#[derive(Clone, Copy)]
enum Change {
    Language(Language),
    Theme(Theme),
    Slider(Slider, f32),
    Accent(u32),
    PasteAccent,
    ResetColors,
    Glass,
    Sidebar,
    Zen,
    Font(f32),
    FontFamily(FontFamily),
    ResetFont,
    Defaults,
    Follow,
    AutoOpenHerdr,
    PasteSocket,
    DefaultSocket,
    OpenHerdr,
    ReconnectHerdr,
    ExitPolicy,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Slider {
    Hue,
    Saturation,
    Lightness,
    Transparency,
    Blur,
}

impl Slider {
    fn maximum(self) -> f32 {
        match self {
            Self::Hue => 360.0,
            Self::Lightness => 30.0,
            Self::Blur => 64.0,
            _ => 100.0,
        }
    }

    fn value(self, appearance: Appearance) -> f32 {
        match self {
            Self::Hue => appearance.hue,
            Self::Saturation => appearance.saturation,
            Self::Lightness => appearance.lightness,
            Self::Transparency => 100.0 - appearance.opacity,
            Self::Blur => appearance.blur_radius as f32,
        }
    }

    fn enabled(self, dark: bool) -> bool {
        dark || matches!(self, Self::Hue | Self::Saturation)
    }
}

pub(crate) struct SettingsWindow {
    owner: WindowHandle<TerminalApp>,
    snapshot: SettingsSnapshot,
    category: Category,
    focus_handle: FocusHandle,
    slider_bounds: HashMap<Slider, Bounds<Pixels>>,
    dragging: Option<Slider>,
    applied_material: Option<(f32, u32)>,
    material_error: Option<String>,
    local_feedback: Option<String>,
    toast: Entity<crate::toast::Toast>,
    updater: Entity<crate::updates::Updater>,
    _updater_observation: Subscription,
    _owner_observation: Subscription,
}

impl SettingsWindow {
    pub(crate) fn new(
        owner: WindowHandle<TerminalApp>,
        entity: Entity<TerminalApp>,
        snapshot: SettingsSnapshot,
        updater: Entity<crate::updates::Updater>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Start from the supplied snapshot rather than eagerly reading the
        // owner, then refresh only when it notifies us of a preference change.
        let observation = cx.observe(&entity, |settings, entity, cx| {
            let snapshot = SettingsSnapshot::from(entity.read(cx));
            if settings.snapshot != snapshot {
                settings.snapshot = snapshot;
                cx.notify();
            }
        });
        window.on_window_should_close(cx, move |_, cx| {
            refocus_owner(owner, cx);
            true
        });
        let toast = crate::toast::new(window, snapshot.palette, cx);
        let updater_observation = cx.observe(&updater, |_, _, cx| cx.notify());
        let settings = Self {
            owner,
            snapshot,
            toast,
            updater,
            _updater_observation: updater_observation,
            category: Category::Appearance,
            focus_handle: cx.focus_handle(),
            slider_bounds: HashMap::new(),
            dragging: None,
            applied_material: None,
            material_error: None,
            local_feedback: None,
            _owner_observation: observation,
        };
        settings.focus(window, cx);
        settings
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        self.focus_handle.focus(window, cx);
    }

    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.remove_window();
        refocus_owner(self.owner, cx);
    }

    fn change(&mut self, change: Change, cx: &mut Context<Self>) {
        self.local_feedback = None;
        let change = if matches!(change, Change::PasteAccent) {
            let color = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .and_then(|text| crate::appearance::parse_accent(&text));
            let Some(color) = color else {
                self.local_feedback =
                    Some("Copy a color in #RRGGBB format, then choose Paste hex.".into());
                cx.notify();
                return;
            };
            Change::Accent(color)
        } else {
            change
        };
        let owner = self.owner;
        // Native appearance and Zen operations need the primary window. Deferring
        // also prevents borrowing either window recursively during dispatch.
        cx.defer(move |cx| {
            let _ = owner.update(cx, |root, terminal_window, cx| {
                match change {
                    Change::Language(language) => {
                        root.set_language(language, cx);
                        return;
                    }
                    Change::Follow => {
                        root.toggle_follow(terminal_window, cx);
                        return;
                    }
                    Change::AutoOpenHerdr => {
                        root.follow_settings.open_herdr_window =
                            !root.follow_settings.open_herdr_window;
                        root.save_follow(cx);
                        return;
                    }
                    Change::PasteSocket => {
                        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            root.set_follow_source(
                                Some(text.trim().to_owned()),
                                terminal_window,
                                cx,
                            );
                        } else {
                            root.follow_status = "Copy an absolute socket path first.".into();
                            cx.notify();
                        }
                        return;
                    }
                    Change::DefaultSocket => {
                        root.set_follow_source(None, terminal_window, cx);
                        return;
                    }
                    Change::OpenHerdr => {
                        root.resume_follow(terminal_window, cx);
                        root.open_herdr_window(terminal_window, cx);
                        return;
                    }
                    Change::ExitPolicy => {
                        root.follow_settings.exit_policy = match root.follow_settings.exit_policy {
                            crate::follow::ExitPolicy::Keep => {
                                crate::follow::ExitPolicy::CloseUnused
                            }
                            crate::follow::ExitPolicy::CloseUnused => {
                                crate::follow::ExitPolicy::Keep
                            }
                        };
                        root.save_follow(cx);
                        return;
                    }
                    Change::ReconnectHerdr => {
                        root.reconnect_herdr(cx);
                        return;
                    }
                    Change::Sidebar => {
                        root.sidebar = !root.sidebar;
                        cx.notify();
                        return;
                    }
                    Change::Zen => {
                        root.toggle_zen(terminal_window, cx);
                        return;
                    }
                    Change::Font(delta) => {
                        root.adjust_font(delta, cx);
                        return;
                    }
                    Change::FontFamily(family) => {
                        root.set_font_family(family, cx);
                        return;
                    }
                    Change::ResetFont => {
                        root.adjust_font(15.0 - root.font_size, cx);
                        return;
                    }
                    Change::Theme(theme) => root.appearance.theme = theme,
                    Change::Slider(slider, value) if slider.enabled(root.palette.dark) => {
                        match slider {
                            Slider::Hue => root.appearance.hue = value,
                            Slider::Saturation => root.appearance.saturation = value,
                            Slider::Lightness => root.appearance.lightness = value,
                            Slider::Transparency => root.appearance.opacity = 100.0 - value,
                            Slider::Blur => root.appearance.blur_radius = value.round() as u32,
                        }
                    }
                    Change::Accent(color) => root.appearance.accent = color,
                    Change::ResetColors => {
                        let defaults = Appearance::default();
                        root.appearance.hue = defaults.hue;
                        root.appearance.saturation = defaults.saturation;
                        root.appearance.lightness = defaults.lightness;
                        root.appearance.accent = defaults.accent;
                    }
                    Change::Glass if root.palette.dark => {
                        root.appearance.main_glass = !root.appearance.main_glass;
                    }
                    Change::Defaults => root.appearance = Appearance::default(),
                    _ => return,
                }
                root.appearance.clamp();
                root.apply_appearance(terminal_window, cx, true);
            });
        });
    }

    fn t(&self, text: &'static str) -> &'static str {
        self.snapshot.language.text(text)
    }

    fn description(&self, text: impl Into<SharedString>) -> gpui::Div {
        let text = text.into();
        div()
            .text_size(px(12.0))
            .line_height(px(18.0))
            .text_color(rgb(self.snapshot.palette.muted))
            .child(self.snapshot.language.dynamic_text(&text))
    }

    fn heading(&self, title: &'static str, description: &'static str) -> gpui::Div {
        div()
            .h(px(52.0))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(20.0))
                    .line_height(px(26.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.t(title)),
            )
            .child(self.description(description))
    }

    fn card(&self) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .rounded(px(10.0))
            .border_1()
            .border_color(rgb(self.snapshot.palette.border))
            .bg(rgba((self.snapshot.palette.shell << 8) | 24))
            .overflow_hidden()
    }

    fn row(&self, height: f32, last: bool) -> gpui::Div {
        div()
            .h(px(height))
            .flex_shrink_0()
            .px(px(16.0))
            .flex()
            .items_center()
            .gap(px(16.0))
            .when(!last, |row| {
                row.border_b_1()
                    .border_color(rgb(self.snapshot.palette.divider))
            })
    }

    fn row_label(&self, title: &'static str, description: &'static str) -> gpui::Div {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(div().text_size(px(13.0)).child(self.t(title)))
            .when(!description.is_empty(), |row| {
                row.child(self.description(description))
            })
    }

    fn step_button(
        &self,
        id: String,
        glyph: &'static str,
        enabled: bool,
        change: Change,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.snapshot.palette;
        let group = SharedString::from(id);
        div()
            .id(group.clone())
            .group(group.clone())
            .when(enabled, |button| crate::ui::keyboard_control(button, p))
            .size(px(30.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .text_color(rgb(p.muted))
            .when(enabled, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgba(p.control_hover())).text_color(rgb(p.text)))
                    .active(move |style| style.bg(rgba(p.control_pressed())))
            })
            .when(!enabled, |button| button.opacity(0.35))
            .on_click(cx.listener(move |settings, _, _, cx| {
                if enabled {
                    settings.change(change, cx);
                }
            }))
            .child(icon(glyph).text_color(rgb(p.muted)).when(enabled, |icon| {
                icon.group_hover(group, move |style| style.text_color(rgb(p.text)))
            }))
    }

    #[allow(clippy::too_many_arguments)]
    fn stepper(
        &self,
        id: &'static str,
        value: String,
        value_width: f32,
        enabled: bool,
        less: bool,
        more: bool,
        decrease: Change,
        increase: Change,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(self.snapshot.palette.border))
            .child(self.step_button(format!("less-{id}"), "minus", enabled && less, decrease, cx))
            .child(
                div()
                    .w(px(value_width))
                    .text_center()
                    .text_size(px(12.0))
                    .when(!enabled, |label| label.opacity(0.45))
                    .child(value),
            )
            .child(self.step_button(format!("more-{id}"), "plus", enabled && more, increase, cx))
    }

    fn toggle(
        &self,
        id: &'static str,
        on: bool,
        enabled: bool,
        change: Change,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.snapshot.palette;
        div()
            .id(id)
            .when(enabled, |button| crate::ui::keyboard_control(button, p))
            .w(px(84.0))
            .h(px(30.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .rounded(px(6.0))
            .when(enabled, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgba(p.control_hover())))
                    .active(move |style| style.bg(rgba(p.control_pressed())))
            })
            .when(!enabled, |button| button.opacity(0.4))
            .on_click(cx.listener(move |settings, _, _, cx| {
                if enabled {
                    settings.change(change, cx);
                }
            }))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(p.muted))
                    .child(self.t(if on { "On" } else { "Off" })),
            )
            .child(
                div()
                    .w(px(38.0))
                    .h(px(22.0))
                    .px(px(3.0))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(rgb(if on { p.accent } else { p.border }))
                    .when(on, |track| track.justify_end())
                    .child(div().size(px(16.0)).rounded_full().bg(rgb(if on {
                        p.panel
                    } else {
                        p.muted
                    }))),
            )
    }

    fn text_button(
        &self,
        id: &'static str,
        label: &'static str,
        change: Change,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.snapshot.palette;
        crate::ui::keyboard_control(div().id(id), p)
            .h(px(30.0))
            .px(px(12.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(p.border))
            .text_size(px(12.0))
            .cursor_pointer()
            .hover(move |style| style.bg(rgba(p.control_hover())))
            .active(move |style| style.bg(rgba(p.control_pressed())))
            .on_click(cx.listener(move |settings, _, _, cx| settings.change(change, cx)))
            .child(self.t(label))
    }

    fn set_slider(&mut self, slider: Slider, x: Pixels, cx: &mut Context<Self>) {
        if !slider.enabled(self.snapshot.palette.dark) {
            return;
        }
        if let Some(bounds) = self.slider_bounds.get(&slider) {
            let fraction = f32::from(x - bounds.origin.x) / f32::from(bounds.size.width).max(1.0);
            let value = (fraction.clamp(0.0, 1.0) * slider.maximum()).round();
            if value != slider.value(self.snapshot.appearance) {
                self.change(Change::Slider(slider, value), cx);
            }
        }
    }

    fn slider(
        &self,
        id: &'static str,
        label: &'static str,
        slider: Slider,
        unit: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.snapshot.palette;
        let value = slider.value(self.snapshot.appearance);
        let enabled = slider.enabled(p.dark);
        let root = cx.entity().downgrade();
        let fraction = value / slider.maximum();
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .when(!enabled, |element| element.opacity(0.4))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(4.0))
                    .text_size(px(11.0))
                    .child(div().text_color(rgb(p.muted)).child(self.t(label)))
                    .child(format!("{value:.0}{unit}")),
            )
            .child(
                div()
                    .id(id)
                    .h(px(28.0))
                    .px(px(7.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .when(enabled, |element| {
                        crate::ui::keyboard_control(element, p)
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgba(p.control_hover())))
                            .active(move |style| style.bg(rgba(p.control_pressed())))
                            .on_key_down(cx.listener(
                                move |settings, event: &gpui::KeyDownEvent, _, cx| {
                                    let stroke = &event.keystroke;
                                    if stroke.modifiers.control
                                        || stroke.modifiers.alt
                                        || stroke.modifiers.platform
                                    {
                                        return;
                                    }
                                    let current = slider.value(settings.snapshot.appearance);
                                    let step = if stroke.modifiers.shift { 10.0 } else { 1.0 };
                                    let value = match stroke.key.as_str() {
                                        "left" | "down" => current - step,
                                        "right" | "up" => current + step,
                                        "home" => 0.0,
                                        "end" => slider.maximum(),
                                        _ => return,
                                    };
                                    settings.change(
                                        Change::Slider(slider, value.clamp(0.0, slider.maximum())),
                                        cx,
                                    );
                                    cx.stop_propagation();
                                },
                            ))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |settings, event: &gpui::MouseDownEvent, _, cx| {
                            if enabled {
                                settings.dragging = Some(slider);
                                settings.set_slider(slider, event.position.x, cx);
                            }
                        }),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(4.0))
                            .rounded_full()
                            .bg(rgba((p.accent << 8) | 40))
                            .child(
                                canvas(
                                    move |bounds, _, cx| {
                                        if let Err(error) = root.update(cx, |settings, _| {
                                            settings.slider_bounds.insert(slider, bounds);
                                        }) {
                                            eprintln!("Settings slider layout: {error}");
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .h_full()
                                    .w(gpui::relative(fraction))
                                    .rounded_full()
                                    .bg(rgb(p.accent)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(gpui::relative(fraction))
                                    .ml(px(-6.0))
                                    .top(px(-4.0))
                                    .size(px(12.0))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(p.accent))
                                    .bg(rgb(p.text)),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(10.0))
                    .text_color(rgb(p.subtle))
                    .child("0")
                    .child(format!("{:.0}", slider.maximum())),
            )
    }

    fn theme_card(
        &self,
        label: &'static str,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.snapshot.palette;
        let palette = |theme| {
            Appearance {
                theme,
                ..self.snapshot.appearance
            }
            .palette(p.dark)
        };
        let (left, right) = match theme {
            Theme::System => (palette(Theme::Dark), palette(Theme::Light)),
            _ => (palette(theme), palette(theme)),
        };
        let selected = self.snapshot.appearance.theme == theme;
        let half = |preview: Palette| {
            div()
                .w_1_2()
                .h_full()
                .p(px(8.0))
                .flex()
                .flex_col()
                .gap(px(7.0))
                .bg(rgb(preview.shell))
                .child(
                    div()
                        .w(px(20.0))
                        .h(px(3.0))
                        .rounded_full()
                        .bg(rgb(preview.accent)),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(3.0))
                        .rounded_full()
                        .bg(rgb(preview.muted)),
                )
                .child(
                    div()
                        .w_3_4()
                        .h(px(3.0))
                        .rounded_full()
                        .bg(rgb(preview.border)),
                )
                .child(
                    div()
                        .w_1_2()
                        .h(px(3.0))
                        .rounded_full()
                        .bg(rgb(preview.border)),
                )
        };
        crate::ui::keyboard_control(div().id(label), p)
            .map(crate::ui::choice_navigation)
            .w(px(164.0))
            .h(px(114.0))
            .flex_shrink_0()
            .p(px(7.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .rounded(px(9.0))
            .border_1()
            .border_color(rgb(if selected { p.accent } else { p.border }))
            .when(selected, |element| {
                element
                    .bg(rgba(p.control_selected(false)))
                    .hover(move |style| style.bg(rgba(p.control_selected(true))))
            })
            .when(!selected, |element| {
                element.hover(move |style| style.bg(rgba(p.control_hover())))
            })
            .cursor_pointer()
            .on_click(
                cx.listener(move |settings, _, _, cx| settings.change(Change::Theme(theme), cx)),
            )
            .child(
                div()
                    .h(px(68.0))
                    .flex_shrink_0()
                    .flex()
                    .rounded(px(4.0))
                    .overflow_hidden()
                    .child(half(left))
                    .child(half(right)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(12.0))
                    .text_color(rgb(p.text))
                    .child(self.t(label))
                    .child(div().flex_1())
                    .when(selected, |element| {
                        element.child(icon("check").text_color(rgb(p.accent)))
                    }),
            )
    }

    fn language_control(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.snapshot.palette;
        let mut choices = div().flex().gap(px(4.0));
        for language in Language::ALL {
            let selected = self.snapshot.language == language;
            let change = Change::Language(language);
            choices = choices.child(
                crate::ui::keyboard_control(div().id(language.label()), p)
                    .map(crate::ui::choice_navigation)
                    .h(px(30.0))
                    .px(px(8.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(rgb(if selected { p.accent } else { p.border }))
                    .text_size(px(12.0))
                    .text_color(rgb(if selected { p.text } else { p.muted }))
                    .cursor_pointer()
                    .when(selected, |button| {
                        button
                            .bg(rgba(p.control_selected(false)))
                            .hover(move |style| style.bg(rgba(p.control_selected(true))))
                    })
                    .when(!selected, |button| {
                        button
                            .hover(move |style| style.bg(rgba(p.control_hover())))
                            .active(move |style| style.bg(rgba(p.control_pressed())))
                    })
                    .on_click(cx.listener(move |settings, _, _, cx| settings.change(change, cx)))
                    .child(language.label()),
            );
        }
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(p.muted))
                    .child(self.t("Language")),
            )
            .child(choices)
    }

    fn appearance_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.snapshot.palette;
        let mut accents = div().flex().items_center().gap(px(10.0));
        for (name, color) in [
            ("Blue", 0x6aa9ef),
            ("Violet", 0x9b8afb),
            ("Pink", 0xdb8abe),
            ("Coral", 0xe08e8e),
            ("Amber", 0xdaaa63),
            ("Sage", 0x83b59c),
            ("Ocean", 0x7db8d9),
            ("Forest", 0xa3c1a3),
            ("Mint", 0x8fc9a0),
        ] {
            accents = accents.child(
                crate::ui::keyboard_control(div().id(name), p)
                    .map(crate::ui::choice_navigation)
                    .size(px(24.0))
                    .flex_shrink_0()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(if self.snapshot.appearance.accent == color {
                        p.text
                    } else {
                        p.panel
                    }))
                    .hover(move |style| style.border_color(rgb(p.text)))
                    .bg(rgb(color))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .on_click(cx.listener(move |settings, _, _, cx| {
                        settings.change(Change::Accent(color), cx)
                    }))
                    .when(self.snapshot.appearance.accent == color, |element| {
                        element.child(icon("check").text_color(rgb(0x151515)))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(12.0))
                    .child(self.heading(
                        "Appearance & preferences",
                        "Make your terminal feel at home.",
                    ))
                    .child(self.language_control(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(self.description("Display mode"))
                    .child(
                        div()
                            .flex()
                            .gap(px(10.0))
                            .child(self.theme_card("System", Theme::System, cx))
                            .child(self.theme_card("Dark", Theme::Dark, cx))
                            .child(self.theme_card("Light", Theme::Light, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(self.description("Accent color"))
                    .child(
                        accents
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(p.muted))
                                    .child(format!("#{:06X}", self.snapshot.appearance.accent)),
                            )
                            .child(self.text_button(
                                "custom-accent",
                                "Paste hex",
                                Change::PasteAccent,
                                cx,
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(20.0))
                    .child(self.slider("hue", "Surface hue", Slider::Hue, "°", cx))
                    .child(self.slider("saturation", "Saturation", Slider::Saturation, "%", cx))
                    .child(self.slider("lightness", "Dark lightness", Slider::Lightness, "%", cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(div().flex_1().child(self.description(
                        "Accent tints the sidebar and surround, and highlights selected tabs and controls.",
                    )))
                    .child(self.text_button(
                        "reset-colors",
                        "Reset colors",
                        Change::ResetColors,
                        cx,
                    )),
            )
    }

    fn window_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.snapshot.palette;
        let a = self.snapshot.appearance;
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(self.heading("Window", "Set the atmosphere and make room for your work."))
            .child(
                self.card()
                    .child(self.row(112.0, false)
                        .child(self.slider("transparency", "Transparency", Slider::Transparency, "%", cx))
                        .child(self.slider("blur", "Window blur", Slider::Blur, "", cx)))
                    .child(
                        self.row(56.0, false)
                            .child(self.row_label(
                                "Main pane glass",
                                if p.dark {
                                    "Apply transparency to the shell and its toolbar."
                                } else {
                                    "Light mode keeps surfaces opaque."
                                },
                            ))
                            .child(self.toggle(
                                "main-glass",
                                p.dark && a.main_glass,
                                p.dark,
                                Change::Glass,
                                cx,
                            )),
                    )
                    .child(
                        self.row(56.0, false)
                            .child(self.row_label(
                                "Tab sidebar",
                                "This session · shown when the window has room.",
                            ))
                            .child(self.toggle(
                                "sidebar",
                                self.snapshot.sidebar,
                                true,
                                Change::Sidebar,
                                cx,
                            )),
                    )
                    .child(
                        self.row(56.0, true)
                            .child(self.row_label(
                                "Zen mode",
                                "This session · hide chrome and focus on the terminal.",
                            ))
                            .child(self.toggle("zen", self.snapshot.zen, true, Change::Zen, cx)),
                    ),
            )
            .child(self.description(if p.dark {
                "Shell and toolbar use this transparency; the surround is 30% less transparent. 0% is solid. Text stays opaque."
            } else {
                "Transparency, blur, and glass are available in dark mode. Your values are kept."
            }))
    }

    fn terminal_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.snapshot.palette;
        let font_size = self.snapshot.font_size;
        let family = self.snapshot.font_family;
        let index = FontFamily::ALL
            .iter()
            .position(|candidate| *candidate == family)
            .unwrap_or(0);
        let previous = FontFamily::ALL[index.saturating_sub(1)];
        let next = FontFamily::ALL[(index + 1).min(FontFamily::ALL.len() - 1)];
        let terminal_font = family.font();
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(self.heading(
                "Terminal",
                "Comfortable type, with a preview at the actual size.",
            ))
            .child(
                self.card()
                    .child(
                        self.row(64.0, false)
                            .child(self.row_label(
                                "Font family",
                                "Saved for every pane, including new terminals.",
                            ))
                            .child(self.stepper(
                                "font-family",
                                family.label().to_owned(),
                                190.0,
                                true,
                                index > 0,
                                index + 1 < FontFamily::ALL.len(),
                                Change::FontFamily(previous),
                                Change::FontFamily(next),
                                cx,
                            )),
                    )
                    .child(
                        self.row(64.0, false)
                            .child(
                                self.row_label(
                                    "Font size",
                                    "Applies to every pane in this session.",
                                ),
                            )
                            .child(self.stepper(
                                "font",
                                format!("{font_size:.0} px"),
                                58.0,
                                true,
                                font_size > 10.0,
                                font_size < 28.0,
                                Change::Font(-1.0),
                                Change::Font(1.0),
                                cx,
                            )),
                    )
                    .child(
                        self.row(50.0, true)
                            .child(self.description("10–28 px · starts at 15 px on launch"))
                            .child(div().flex_1())
                            .child(
                                self.text_button(
                                    "reset-font",
                                    "Reset to 15 px",
                                    Change::ResetFont,
                                    cx,
                                )
                                .w(px(132.0)),
                            ),
                    ),
            )
            .child(
                self.card()
                    .h(px(118.0))
                    .p(px(16.0))
                    .gap(px(10.0))
                    .bg(rgb(p.terminal))
                    .child(self.description("MONOSPACE PREVIEW"))
                    .child(
                        div()
                            .font(terminal_font.clone())
                            .font_features(terminal_font.features)
                            .text_size(px(font_size))
                            .line_height(px(font_size * 1.35))
                            .child("Aa Bb 0123456789")
                            .child(div().text_color(rgb(p.accent)).child("let answer = 42;")),
                    ),
            )
            .child(self.description("Font family is saved. Font size applies to this session."))
    }

    fn shortcut_group(
        &self,
        title: &'static str,
        shortcuts: &[(&'static str, &'static str)],
    ) -> gpui::Div {
        let p = self.snapshot.palette;
        let mut group = self.card().px(px(16.0)).py(px(10.0)).child(
            div()
                .h(px(30.0))
                .flex()
                .items_center()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(self.t(title)),
        );
        for (label, key) in shortcuts {
            group = group.child(
                div()
                    .h(px(30.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .text_size(px(12.0))
                    .child(self.t(label))
                    .child(
                        div()
                            .text_color(rgb(p.muted))
                            .font_family(".SystemUIFont")
                            .child(*key),
                    ),
            );
        }
        group
    }

    fn shortcuts_page(&self) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .child(self.heading(
                "Shortcuts",
                "A reference for the terminal window. Bindings are fixed.",
            ))
            .child(self.shortcut_group(
                "Tabs & panes",
                &[
                    ("New local tab / return to local", "⌘T"),
                    ("Close pane", "⌘W"),
                    ("Close tab", "⌘⇧W"),
                    ("Next / previous tab", "⌃Tab / ⌃⇧Tab"),
                    ("Customize tab", "⌘⇧E"),
                    ("Split right / down", "⌘D / ⌘⇧D"),
                    ("Next / previous pane", "⌘⌥→ / ⌘⌥←"),
                    ("Restart active pane", "⌘⇧R"),
                ],
            ))
            .child(self.shortcut_group(
                "View & editing",
                &[
                    ("Tab sidebar", "⌘B"),
                    ("Zen mode", "⌘⇧Z"),
                    ("Increase / decrease font", "⌘+ / ⌘−"),
                    ("Reset font size", "⌘0"),
                    ("Copy / paste", "⌘C / ⌘V"),
                    ("Select all", "⌘A"),
                ],
            ))
            .child(self.shortcut_group(
                "Application",
                &[
                    ("Open settings", "⌘,"),
                    ("Close settings", "Esc / ⌘W / ⌘⇧W"),
                    ("Close Herdr window / detach client", "⌘W / ⌘⇧W"),
                    ("Quit MonoTerm", "⌘Q"),
                ],
            ))
    }

    fn integrations_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        div().flex().flex_col().gap(px(16.0))
            .child(self.heading("Herdr follow", "Spaces and agents share the Herdr window. This window follows with local terminals."))
            .child(self.card()
                .child(self.row(72.0, false)
                    .child(self.row_label("Enable Herdr follow", "Reuse a local shell and its splits for each selected agent."))
                    .child(self.toggle("enable-follow", self.snapshot.follow.enabled, true, Change::Follow, cx)))
                .child(self.row(72.0, false)
                    .child(self.row_label("Automatically open Herdr window", "On launch, enabling follow or clicking the Herdr icon. Existing windows stay open."))
                    .child(self.toggle("auto-open-herdr", self.snapshot.follow.open_herdr_window, true, Change::AutoOpenHerdr, cx)))
                .child(self.row(72.0, true)
                    .child(self.row_label("Connection", "Agent updates continue while local terminals are shown."))
                    .child(div().max_w(px(200.0)).truncate().text_size(px(12.0)).child(self.snapshot.language.dynamic_text(&self.snapshot.follow_status)))))
            .child(self.card()
                .child(div().p(px(16.0)).flex().flex_col().gap(px(12.0))
                    .child(div().text_size(px(13.0)).child(self.t("Herdr source socket")))
                    .child(self.description("Use the daemon's JSON socket. Its path determines the client surface endpoint."))
                    .child(div().text_size(px(12.0)).text_color(rgb(self.snapshot.palette.muted)).child(self.snapshot.socket_path.clone()))
                    .child(div().flex().gap(px(8.0))
                        .child(self.text_button("paste-herdr-socket", "Paste path", Change::PasteSocket, cx))
                        .child(self.text_button("default-herdr-socket", "Use default", Change::DefaultSocket, cx)))))
            .child(div().flex().gap(px(8.0))
                .child(self.text_button("open-herdr", "Open Herdr window", Change::OpenHerdr, cx))
                .child(self.text_button("reconnect-herdr", "Reconnect", Change::ReconnectHerdr, cx)))
            .child(self.card().child(self.row(72.0, false)
                .child(self.row_label("Close unused companions", "After agent exit, close only untouched, idle local shells."))
                .child(self.toggle("close-unused", self.snapshot.follow.exit_policy == crate::follow::ExitPolicy::CloseUnused, true, Change::ExitPolicy, cx))))
            .child(self.description("Closing the Herdr window detaches this client. Agents and local terminals keep running."))
    }

    fn version_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        use crate::updates::{RELEASES_URL, UpdateState, VERSION};
        let p = self.snapshot.palette;
        let status = &self.updater.read(cx).status;
        let title = match status.state {
            UpdateState::Ready => "Ready to check for updates",
            UpdateState::Checking => "Checking for updates…",
            UpdateState::Available => "An update is available",
            UpdateState::Current => "You’re up to date",
            UpdateState::InstallPending => "Preparing to restart…",
            UpdateState::Disabled => "Updates are disabled in test windows",
            UpdateState::Unavailable => "In-app updates are unavailable here",
            UpdateState::Error => "The update could not complete",
        };
        let detail = match status.state {
            UpdateState::Available => status
                .version
                .as_ref()
                .map(|version| format!("{} {version}", self.t("Available version")))
                .unwrap_or_default(),
            UpdateState::Disabled => self
                .t("Check and install updates in the normally launched application.")
                .to_owned(),
            UpdateState::Unavailable => self
                .t("Get a configured application package from Releases & downloads.")
                .to_owned(),
            UpdateState::Error => status.message.clone(),
            _ => String::new(),
        };
        let can_check = status.can_check;
        div()
            .flex()
            .flex_col()
            .gap(px(20.0))
            .child(self.heading("Version", "Get the latest release and view app version information."))
            .child(
                self.card().child(
                    div()
                        .p(px(20.0))
                        .flex()
                        .items_center()
                        .gap(px(16.0))
                        .child(
                            gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded(
                                "app-icon.png".into(),
                            )))
                            .size(px(64.0))
                            .flex_shrink_0(),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .text_size(px(18.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("MonoTerm"),
                                )
                                .child(self.description(format!("{} {VERSION}", self.t("Version"))))
                                .child(self.description("Native macOS terminal · GPUI + Ghostty")),
                        ),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.t(title)),
                    )
                    .when(!detail.is_empty(), |row| row.child(self.description(detail))),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(10.0))
                    .child(
                        div()
                            .id("check-updates")
                            .when(can_check, |button| crate::ui::keyboard_control(button, p))
                            .h(px(32.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(p.border))
                            .text_size(px(12.0))
                            .when(can_check, |button| {
                                button
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgba(p.control_hover())))
                                    .active(move |style| style.bg(rgba(p.control_pressed())))
                            })
                            .when(!can_check, |button| button.opacity(0.45))
                            .on_click(cx.listener(move |settings, _, _, cx| {
                                if can_check {
                                    settings.updater.update(cx, |updater, cx| updater.check(cx));
                                }
                            }))
                            .child(self.t("Check for updates")),
                    )
                    .child(
                        crate::ui::keyboard_control(div().id("update-releases"), p)
                            .h(px(32.0))
                            .px(px(12.0))
                            .flex()
                            .items_center()
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(rgb(p.border))
                            .text_size(px(12.0))
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgba(p.control_hover())))
                            .active(move |style| style.bg(rgba(p.control_pressed())))
                            .on_click(|_, _, cx| cx.open_url(RELEASES_URL))
                            .child(self.t("Releases & downloads")),
                    ),
            )
            .child(self.description("Updates are checked manually and verified with Sparkle. Automatic checks and background downloads are off."))
            .child(self.description("Installing an update restarts MonoTerm and closes local terminal sessions. Finish running jobs first. Herdr sessions keep running."))
    }

    fn category_tabs(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.snapshot.palette;
        let mut tabs = div()
            .h(px(64.0))
            .flex_shrink_0()
            .px(px(24.0))
            .pt(px(8.0))
            .pb(px(16.0))
            .flex()
            .gap(px(6.0))
            .border_b_1()
            .border_color(rgb(p.divider));
        for (id, label, glyph, category) in [
            (
                "category-appearance",
                "Appearance",
                "settings",
                Category::Appearance,
            ),
            ("category-window", "Window", "monitor", Category::Window),
            (
                "category-integrations",
                "Integrations",
                "terminal",
                Category::Integrations,
            ),
            (
                "category-terminal",
                "Terminal",
                "terminal",
                Category::Terminal,
            ),
            (
                "category-shortcuts",
                "Shortcuts",
                "keyboard",
                Category::Shortcuts,
            ),
            ("category-version", "Version", "restart", Category::Version),
        ] {
            let selected = self.category == category;
            tabs = tabs.child(
                crate::ui::keyboard_control(div().id(id), p)
                    .map(crate::ui::choice_navigation)
                    .flex_1()
                    .h(px(40.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(5.0))
                    .rounded(px(7.0))
                    .text_size(px(12.0))
                    .text_color(rgb(if selected { p.text } else { p.muted }))
                    .when(selected, |tab| {
                        tab.bg(rgba(p.control_selected(false)))
                            .hover(move |style| style.bg(rgba(p.control_selected(true))))
                    })
                    .when(!selected, |tab| {
                        tab.hover(move |style| style.bg(rgba(p.control_hover())))
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |settings, _, _, cx| {
                        if settings.category != category {
                            settings.category = category;
                            cx.notify();
                        }
                    }))
                    .when(category == Category::Integrations, |tab| {
                        tab.child(crate::ui::herdr_icon())
                    })
                    .when(category != Category::Integrations, |tab| {
                        tab.child(icon(glyph).text_color(rgb(if selected {
                            p.accent
                        } else {
                            p.muted
                        })))
                    })
                    .child(self.t(label)),
            );
        }
        tabs
    }

    fn footer(&self, cx: &mut Context<Self>) -> gpui::Div {
        let message = match self
            .local_feedback
            .as_deref()
            .or(self.material_error.as_deref())
            .or(self.snapshot.feedback.as_deref())
        {
            Some(message)
                if message.starts_with("Could not save")
                    || message.starts_with("Could not encode") =>
            {
                "Could not save preferences. Changes are temporary."
            }
            Some(message) if self.local_feedback.is_some() || self.material_error.is_some() => {
                message
            }
            Some(message)
                if message.starts_with("Could not read")
                    || message.starts_with("Could not decode") =>
            {
                "Preferences could not be loaded. Defaults are in use."
            }
            Some(_) => "Settings could not be applied or saved. Check the terminal status.",
            None => "Preferences saved automatically.",
        };
        div()
            .h(px(56.0))
            .flex_shrink_0()
            .px(px(24.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .border_t_1()
            .border_color(rgb(self.snapshot.palette.divider))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.0))
                    .line_height(px(16.0))
                    .text_color(rgb(self.snapshot.palette.muted))
                    .child(self.snapshot.language.dynamic_text(message)),
            )
            .child(
                self.text_button(
                    "reset-appearance",
                    "Appearance defaults",
                    Change::Defaults,
                    cx,
                )
                .w(px(154.0)),
            )
    }
}

fn refocus_owner(owner: WindowHandle<TerminalApp>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = owner.update(cx, |root, window, cx| {
            window.activate_window();
            if let Some(session) = root.active_session() {
                session.focus(window, cx);
            }
        });
    });
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self
            .snapshot
            .appearance
            .palette(TerminalApp::system_dark(window));
        self.snapshot.palette = p;
        window.set_window_title(self.t("MonoTerm Settings"));
        let material = (p.alpha, self.snapshot.appearance.blur_radius);
        if self.applied_material != Some(material) {
            self.material_error =
                crate::window_style::apply_material(window, material.0, material.1).err();
            self.applied_material = Some(material);
        }
        let page = match self.category {
            Category::Appearance => self.appearance_page(cx),
            Category::Window => self.window_page(cx),
            Category::Terminal => self.terminal_page(cx),
            Category::Shortcuts => self.shortcuts_page(),
            Category::Integrations => self.integrations_page(cx),
            Category::Version => self.version_page(cx),
        };
        div()
            .id("settings-window")
            .relative()
            .key_context("Settings")
            .track_focus(&self.focus_handle)
            .tab_group()
            .tab_stop(false)
            .on_key_down(crate::ui::traverse_controls)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgba((p.shell << 8) | (p.alpha * 255.0) as u32))
            .text_color(rgb(p.text))
            .font_family(".SystemUIFont")
            .on_mouse_move(
                cx.listener(|settings, event: &gpui::MouseMoveEvent, _, cx| {
                    if let Some(slider) = settings.dragging {
                        if event.pressed_button == Some(MouseButton::Left) {
                            settings.set_slider(slider, event.position.x, cx);
                        } else {
                            settings.dragging = None;
                        }
                    }
                }),
            )
            .capture_any_mouse_up(cx.listener(|settings, _, _, _| settings.dragging = None))
            .on_action(
                cx.listener(|settings, _: &CloseSettings, window, cx| settings.close(window, cx)),
            )
            .on_action(
                cx.listener(|settings, _: &CloseSession, window, cx| settings.close(window, cx)),
            )
            .on_action(cx.listener(|settings, _: &CloseTab, window, cx| settings.close(window, cx)))
            .on_action(
                cx.listener(|settings, _: &ToggleSettings, window, cx| settings.focus(window, cx)),
            )
            .child(
                div()
                    .h(px(44.0))
                    .flex_shrink_0()
                    .pl(px(78.0))
                    .pr(px(78.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(gpui::MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.titlebar_double_click();
                        } else {
                            window.start_window_move();
                        }
                    })
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.t("Settings")),
            )
            .child(self.category_tabs(cx))
            .child(
                div()
                    .id(match self.category {
                        Category::Appearance => "appearance-scroll",
                        Category::Window => "window-scroll",
                        Category::Terminal => "terminal-scroll",
                        Category::Shortcuts => "shortcuts-scroll",
                        Category::Integrations => "integrations-scroll",
                        Category::Version => "version-scroll",
                    })
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().px(px(24.0)).py(px(20.0)).child(page)),
            )
            .when(self.category != Category::Version, |page| {
                page.child(self.footer(cx))
            })
            .child(crate::toast::layer(&self.toast, p, cx))
    }
}
