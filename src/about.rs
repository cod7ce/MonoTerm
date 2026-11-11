use crate::{
    ApplicationState, CloseWindow, TerminalApp,
    appearance::{Appearance, Palette},
    locale::Language,
    ui, updates,
};
use gpui::{
    App, Bounds, Context, Entity, FocusHandle, FontWeight, Render, Subscription, TitlebarOptions,
    Window, WindowControlArea, WindowOptions, div, prelude::*, px, rgb, rgba, size,
};

pub(crate) struct AboutWindow {
    appearance: Appearance,
    language: Language,
    focus_handle: FocusHandle,
    _owner_observation: Subscription,
    _appearance_observation: Subscription,
}

pub(crate) fn open(cx: &mut App) {
    let state = cx.global::<ApplicationState>();
    let owner = state.terminal.clone();
    if let Some(existing) = state.about_window
        && existing
            .update(cx, |about, window, cx| about.focus(window, cx))
            .is_ok()
    {
        return;
    }
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(440.0), px(510.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some(owner.read(cx).language.text("About MonoTerm · ONES").into()),
                appears_transparent: true,
                ..Default::default()
            }),
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        },
        |window, cx| cx.new(|cx| AboutWindow::new(owner, window, cx)),
    );
    match result {
        Ok(window) => cx.global_mut::<ApplicationState>().about_window = Some(window),
        Err(error) => eprintln!("Open About {}: {error}", crate::APP_NAME),
    }
}

impl AboutWindow {
    fn new(owner: Entity<TerminalApp>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let appearance = owner.read(cx).appearance;
        let language = owner.read(cx).language;
        let owner_observation = cx.observe(&owner, |about, owner, cx| {
            let owner = owner.read(cx);
            if about.appearance != owner.appearance || about.language != owner.language {
                about.appearance = owner.appearance;
                about.language = owner.language;
                cx.notify();
            }
        });
        let appearance_observation = cx.observe_window_appearance(window, |_, _, cx| cx.notify());
        let about = Self {
            appearance,
            language,
            focus_handle: cx.focus_handle(),
            _owner_observation: owner_observation,
            _appearance_observation: appearance_observation,
        };
        about.focus(window, cx);
        about
    }

    fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        self.focus_handle.focus(window, cx);
    }

    fn link(
        &self,
        id: &'static str,
        label: &'static str,
        url: &'static str,
        palette: Palette,
    ) -> gpui::Stateful<gpui::Div> {
        ui::keyboard_control(div().id(id), palette)
            .h(px(32.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(6.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgb(palette.border))
            .text_size(px(12.0))
            .cursor_pointer()
            .hover(move |style| style.bg(rgba(palette.control_hover())))
            .active(move |style| style.bg(rgba(palette.control_pressed())))
            .on_click(move |_, _, cx| cx.open_url(url))
            .child(self.language.text(label))
            .child(ui::icon("chevron").text_color(rgb(palette.muted)))
    }
}

impl Render for AboutWindow {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = self.appearance.palette(TerminalApp::system_dark(window));
        let title = self.language.text("About MonoTerm · ONES");
        window.set_window_title(title);
        let architecture = match std::env::consts::ARCH {
            "aarch64" => "Apple Silicon",
            "x86_64" => "Intel",
            architecture => architecture,
        };
        div()
            .id("about-window")
            .key_context("About")
            .track_focus(&self.focus_handle)
            .tab_group()
            .tab_stop(false)
            .on_key_down(ui::traverse_controls)
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(palette.shell))
            .text_color(rgb(palette.text))
            .font_family(".SystemUIFont")
            .child(
                div()
                    .h(px(40.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    })
                    .text_size(px(13.0))
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(24.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(20.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded(
                                    "app-icon.png".into(),
                                )))
                                .size(px(80.0)),
                            )
                            .child(
                                div()
                                    .text_size(px(24.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(crate::APP_NAME),
                            )
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(rgb(palette.muted))
                                    .text_center()
                                    .child(
                                        self.language
                                            .text("A native macOS terminal for focused work."),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .px(px(14.0))
                            .py(px(6.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(palette.border))
                            .bg(rgb(palette.panel))
                            .children(
                                [
                                    ("Version", updates::VERSION.to_owned()),
                                    ("Platform", format!("macOS · {architecture}")),
                                    ("Built with", "Rust · GPUI · Ghostty".to_owned()),
                                ]
                                .into_iter()
                                .map(|(label, value)| {
                                    div()
                                        .h(px(32.0))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .text_size(px(12.0))
                                        .child(
                                            div()
                                                .text_color(rgb(palette.muted))
                                                .child(self.language.text(label)),
                                        )
                                        .child(value)
                                }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(10.0))
                            .child(self.link(
                                "about-project",
                                "Project on GitHub",
                                env!("CARGO_PKG_REPOSITORY"),
                                palette,
                            ))
                            .child(self.link(
                                "about-releases",
                                "Releases & downloads",
                                updates::RELEASES_URL,
                                palette,
                            )),
                    ),
            )
    }
}
