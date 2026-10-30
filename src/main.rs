mod appearance;
mod companion;
mod follow;
mod herdr;
mod herdr_connection;
mod herdr_render;
mod herdr_view;
mod layout;
mod locale;
mod process_monitor;
mod pty;
mod session;
mod settings;
mod shell_surface;
mod tab_editor;
mod text_input;
mod toast;
mod typography;
mod ui;
mod updates;
mod window_style;

use appearance::{Appearance, Palette};
use gpui::{
    App, Application, Bounds, Context, KeyBinding, Menu, MenuItem, MouseButton, OsAction, Pixels,
    QuitMode, Render, SharedString, Subscription, Task, TitlebarOptions, Window, WindowAppearance,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, canvas, div, point,
    prelude::*, px, relative, rgb, rgba, size,
};
use gpui_ghostty_terminal::view::{Copy, Paste, SelectAll};
use layout::{Axis, Layout, Tab};
use session::Session;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};
use ui::{dot, icon, icon_button};

actions!(
    terminal,
    [
        Quit,
        Restart,
        IncreaseFont,
        DecreaseFont,
        ResetFont,
        NewSession,
        CloseSession,
        CloseTab,
        NextSession,
        PreviousSession,
        ToggleSidebar,
        ToggleSettings,
        CustomizeTab,
        DismissTabMenu,
        SplitRight,
        SplitDown,
        NextPane,
        PreviousPane,
        ToggleZen,
        ResumeFollow,
        ReturnLocal,
        OpenHerdrWindow
    ]
);

/// Gap between the window edge and the shell on every side except Zen.
const SHELL_INSET: f32 = 5.0;
/// Native window buttons, sidebar toggle and shell header controls share this center.
const TOOLBAR_CENTER: f32 = 22.0;
/// Compact titlebar: keeps the y=22 control center with less empty space below.
const TOOLBAR_HEIGHT: f32 = 34.0;
/// Inner curve visually follows the native window contour after the shell inset.
const SHELL_RADIUS: f32 = 12.0;

struct TerminalApp {
    sessions: Vec<Session>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: usize,
    default_directory: PathBuf,
    /// Active pane's starting directory, also used for the default native title.
    directory: PathBuf,
    shell: String,
    font_size: f32,
    typography: typography::Typography,
    language: locale::Language,
    sidebar: bool,
    sidebar_width: f32,
    resizing_sidebar: bool,
    settings_window: Option<gpui::WindowHandle<settings::SettingsWindow>>,
    updater: gpui::Entity<updates::Updater>,
    tab_menu: Option<(usize, gpui::Point<Pixels>)>,
    tab_menu_focus: gpui::FocusHandle,
    tab_menu_items: [gpui::FocusHandle; 3],
    tab_editor: Option<gpui::Entity<tab_editor::TabEditor>>,
    tab_editor_subscription: Option<Subscription>,
    zen: bool,
    appearance: Appearance,
    palette: Palette,
    shell_corners: shell_surface::CornerCache,
    toast: gpui::Entity<toast::Toast>,
    split_bounds: HashMap<usize, Bounds<Pixels>>,
    dragging: Option<(usize, Axis)>,
    notice: Option<String>,
    appearance_notice: Option<String>,
    follow_settings: follow::FollowSettings,
    follow_controller: follow::Controller,
    follow_observer: Option<herdr::Client>,
    follow_snapshot: Option<herdr::Snapshot>,
    follow_generation: u64,
    retiring_companions: std::collections::HashSet<usize>,
    herdr_connection: Option<gpui::Entity<herdr_connection::HerdrConnection>>,
    herdr_window: Option<gpui::WindowHandle<herdr_view::HerdrWindow>>,
    follow_status: String,
    root_focus: gpui::FocusHandle,
    _output_task: Task<()>,
    _quit: Subscription,
    _appearance_subscription: Subscription,
}

impl TerminalApp {
    fn new(directory: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let output_task = cx.spawn_in(window, async move |root, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if root
                    .update_in(cx, |root, window, cx| {
                        root.pump_follow(window, cx);
                        let mut changed = false;
                        let mut exited = Vec::new();
                        for session in &mut root.sessions {
                            let was_running = session.running;
                            changed |= session.poll(cx);
                            if was_running && !session.running {
                                exited.push(session.id);
                            }
                        }
                        // Only a confirmed child exit closes a pane. Startup and I/O
                        // errors leave it available for inspection or restart.
                        for id in exited {
                            root.remove_pane(id, false, window, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let quit = cx.on_app_quit(|root, _| {
            root.sessions.clear();
            std::future::ready(())
        });
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/bin/zsh".into());
        let appearance_subscription = cx.observe_window_appearance(window, |root, window, cx| {
            root.apply_appearance(window, cx, false);
        });
        let (appearance, notice) = Appearance::load();
        let (language, language_notice) = locale::Language::load();
        let (typography, typography_notice) = typography::Typography::load();
        set_menus(language, cx);
        let (follow_settings, follow_notice) = follow::FollowSettings::load();
        let palette = appearance.palette(Self::system_dark(window));
        let root = cx.weak_entity();
        let app = cx.to_async();
        updates::set_update_termination_handler(move || {
            app.update(|cx| {
                root.update(cx, |root, _| {
                    root.sessions.clear();
                    true
                })
                .unwrap_or(false)
            })
            .unwrap_or(false)
        });
        let updater = cx
            .new(|cx| updates::Updater::new(std::env::var_os("MONOTERM_CONFIG_DIR").is_none(), cx));
        let mut root = Self {
            sessions: Vec::new(),
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            default_directory: directory.clone(),
            directory,
            shell,
            font_size: 15.0,
            typography,
            language,
            sidebar: true,
            sidebar_width: 228.0,
            resizing_sidebar: false,
            settings_window: None,
            updater,
            tab_menu: None,
            tab_menu_focus: cx.focus_handle(),
            tab_menu_items: std::array::from_fn(|_| cx.focus_handle().tab_index(0).tab_stop(true)),
            tab_editor: None,
            tab_editor_subscription: None,
            zen: false,
            appearance,
            palette,
            shell_corners: shell_surface::CornerCache::default(),
            toast: toast::new(window, palette, cx),
            split_bounds: HashMap::new(),
            dragging: None,
            notice: None,
            appearance_notice: notice.or(language_notice).or(typography_notice),
            follow_controller: follow::Controller::default(),
            follow_observer: None,
            follow_snapshot: None,
            follow_generation: 0,
            retiring_companions: Default::default(),
            follow_settings,
            herdr_connection: None,
            herdr_window: None,
            follow_status: follow_notice.unwrap_or_else(|| "Off".into()),
            root_focus: cx.focus_handle(),
            _output_task: output_task,
            _quit: quit,
            _appearance_subscription: appearance_subscription,
        };
        if !root.follow_settings.enabled {
            root.new_session(window, cx);
        } else {
            root.root_focus.focus(window, cx);
        }
        root.start_follow(cx);
        root.apply_appearance(window, cx, false);
        if root.follow_settings.enabled && root.follow_settings.open_herdr_window {
            root.open_herdr_window(window, cx);
        }
        root
    }

    fn system_dark(window: &Window) -> bool {
        matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        )
    }

    fn active_session(&self) -> Option<&Session> {
        let id = self.tabs.get(self.active)?.active_pane;
        self.sessions.iter().find(|s| s.id == id)
    }

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self
            .active_session()
            .map(|session| &session.directory)
            .unwrap_or(&self.default_directory)
            .clone();
        self.new_session_in(directory, window, cx);
    }

    fn new_session_in(&mut self, directory: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() >= 12 || self.sessions.len() >= 48 {
            if let Some(session) = self.active_session() {
                session.focus(window, cx);
            }
            self.notice = Some("Limit reached: 12 tabs / 48 panes.".into());
            cx.notify();
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(Session::new(
            id,
            &self.shell,
            &directory,
            self.typography.font_family.font(),
            cx,
        ));
        let previous = self.tabs.get(self.active).map(|tab| tab.id);
        self.tabs.push(Tab::new(id, id));
        self.follow_controller.manual_tab_change(previous, Some(id));
        self.select_content(self.tabs.len() - 1, false, window, cx);
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select_content(index, true, window, cx);
    }

    fn select_content(
        &mut self,
        index: usize,
        manual: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() {
            return;
        }
        if manual {
            self.follow_controller.manual_tab_change(
                self.tabs.get(self.active).map(|tab| tab.id),
                Some(self.tabs[index].id),
            );
        }
        self.active = index;
        self.dragging = None;
        self.split_bounds.clear();
        if let Some(session) = self.active_session() {
            let directory = session.directory.clone();
            if self.tab_editor.is_none() && self.tab_menu.is_none() {
                session.focus(window, cx);
            }
            self.directory = directory;
        }
        self.notice = None;
        cx.notify();
    }

    fn focus_pane(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active)
            && tab.layout.contains(id)
        {
            tab.active_pane = id;
            if let Some(session) = self.sessions.iter().find(|s| s.id == id) {
                session.focus(window, cx);
                self.directory = session.directory.clone();
            }
            cx.notify();
        }
    }

    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        self.mark_local_edit();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if tab.pane_count() >= 8 || self.sessions.len() >= 48 {
            self.notice = Some("Limit reached: 8 panes per tab / 48 panes total.".into());
            cx.notify();
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        if tab.split(id, id, axis) {
            self.sessions.push(Session::new(
                id,
                &self.shell,
                &self.directory,
                self.typography.font_family.font(),
                cx,
            ));
            self.focus_pane(id, window, cx);
        }
    }

    fn close_pane(&mut self, id: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_pane(id, true, window, cx);
    }

    fn remove_pane(
        &mut self,
        id: usize,
        manual: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tabs.iter().position(|tab| tab.layout.contains(id)) else {
            return;
        };
        let selected = self.tabs.get(self.active).map(|tab| tab.id);
        let was_active = index == self.active;
        let tab = self.tabs.remove(index);
        if tab.pane_count() == 1 {
            // A locally exited shell must not be recreated on the next snapshot.
            if manual || self.follow_controller.is_companion(tab.id) {
                self.follow_controller.close_tab(tab.id);
            }
        } else if manual {
            self.follow_controller.mark_used(tab.id);
        }
        if let Some(session) = self.sessions.iter().position(|session| session.id == id) {
            self.sessions.remove(session).retire();
        }
        let remaining = if tab.active_pane == id {
            tab.close_active()
        } else {
            tab.close(id)
        };
        if let Some(tab) = remaining {
            self.tabs.insert(index, tab);
        }
        if self.tabs.is_empty() {
            if manual {
                self.follow_controller.manual_tab_change(selected, None);
            }
            self.finish_local_sessions(window, cx);
        } else {
            let next = selected
                .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
                .unwrap_or(index.min(self.tabs.len() - 1));
            if manual {
                self.follow_controller
                    .manual_tab_change(selected, Some(self.tabs[next].id));
            }
            if was_active {
                self.select_content(next, false, window, cx);
            } else {
                // A background exit can shift tab indices, but must not move focus
                // or cancel a divider drag in the selected tab.
                self.active = next;
                cx.notify();
            }
        }
    }

    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let selected = self.tabs.get(self.active).map(|tab| tab.id);
        let tab = self.tabs.remove(index);
        self.follow_controller.close_tab(tab.id);
        let ids = tab.pane_ids();
        for id in ids {
            if let Some(index) = self.sessions.iter().position(|session| session.id == id) {
                self.sessions.remove(index).retire();
            }
        }
        if self.tabs.is_empty() {
            self.follow_controller.manual_tab_change(selected, None);
            self.finish_local_sessions(window, cx);
        } else {
            let next = selected
                .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
                .unwrap_or(index.min(self.tabs.len() - 1));
            self.follow_controller
                .manual_tab_change(selected, Some(self.tabs[next].id));
            self.select_content(next, false, window, cx);
        }
    }

    fn cycle(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let next = if forward {
            (self.active + 1) % self.tabs.len()
        } else {
            (self.active + self.tabs.len() - 1) % self.tabs.len()
        };
        self.select(next, window, cx);
    }

    fn cycle_pane(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            let id = tab.cycle_focus(forward);
            self.focus_pane(id, window, cx);
        }
    }

    fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mark_local_edit();
        let Some(id) = self.tabs.get(self.active).map(|t| t.active_pane) else {
            return;
        };
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
            session.start(&self.shell, &session.directory.clone(), cx);
            session.focus(window, cx);
            cx.notify();
        }
    }

    fn toggle_zen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match window_style::set_zen(window, !self.zen) {
            Ok(()) => {
                self.zen = !self.zen;
                self.dragging = None;
                if !self.zen {
                    // Unhiding AppKit buttons can reset their frames. Updating the
                    // title makes GPUI reapply the configured traffic-light position.
                    window.set_window_title(&window.window_title());
                }
            }
            Err(error) => self.notice = Some(error),
        }
        if let Some(session) = self.active_session() {
            session.focus(window, cx);
        }
        cx.notify();
    }

    fn adjust_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.font_size = (self.font_size + delta).clamp(10.0, 28.0);
        self.sync_herdr_preferences(cx);
        cx.notify();
    }

    fn resize_terminal(
        &mut self,
        id: usize,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut style = window.text_style();
        let font = self.typography.font_family.font();
        style.font_family = font.family;
        style.font_fallbacks = font.fallbacks;
        style.font_features = font.features;
        let Ok(lines) = window.text_system().shape_text(
            SharedString::from("M"),
            px(self.font_size),
            &[style.to_run(1)],
            None,
            Some(1),
        ) else {
            return;
        };
        let Some(line) = lines.first() else {
            return;
        };
        let cols = (f32::from(bounds.size.width) / f32::from(line.width()).max(1.0))
            .floor()
            .clamp(1.0, 500.0) as u16;
        let rows = (f32::from(bounds.size.height) / (self.font_size * 1.35))
            .floor()
            .clamp(1.0, 300.0) as u16;
        let scale = window.scale_factor();
        let cell_width = (f32::from(line.width()).max(1.0) * scale).round().max(1.0);
        let cell_height = (self.font_size * 1.35 * scale).round().max(1.0);
        let pixel_size = (
            (f32::from(cols) * cell_width).min(f32::from(u16::MAX)) as u16,
            (f32::from(rows) * cell_height).min(f32::from(u16::MAX)) as u16,
        );
        if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id)
            && session.resize(cols, rows, pixel_size, cx)
        {
            cx.notify();
        }
    }

    fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>, save: bool) {
        if save
            || self
                .appearance_notice
                .as_deref()
                .is_some_and(|notice| notice.starts_with("Window effect:"))
        {
            self.appearance_notice = None;
        }
        self.palette = self.appearance.palette(Self::system_dark(window));
        if let Err(error) =
            window_style::apply_material(window, self.palette.alpha, self.appearance.blur_radius)
        {
            self.appearance_notice = Some(format!("Window effect: {error}"));
        }
        if save && let Err(error) = self.appearance.save() {
            self.appearance_notice = Some(error);
        }
        self.sync_herdr_preferences(cx);
        cx.notify();
    }

    fn t(&self, text: &'static str) -> &'static str {
        self.language.text(text)
    }

    fn set_font_family(&mut self, family: typography::FontFamily, cx: &mut Context<Self>) {
        self.typography.font_family = family;
        self.appearance_notice = self.typography.save().err();
        for session in &mut self.sessions {
            session.set_font(family.font(), cx);
        }
        self.sync_herdr_preferences(cx);
        cx.notify();
    }

    fn set_language(&mut self, language: locale::Language, cx: &mut Context<Self>) {
        self.language = language;
        self.appearance_notice = language.save().err();
        set_menus(language, cx);
        self.sync_herdr_preferences(cx);
        cx.notify();
    }

    fn label(&self, text: impl Into<SharedString>) -> gpui::Div {
        let text = text.into();
        div()
            .text_size(px(11.0))
            .text_color(rgb(self.palette.muted))
            .child(self.language.dynamic_text(&text))
    }

    /// Follow state as a single icon: linked while following, pinned after a
    /// manual tab change, and crossed out when follow is disabled.
    fn follow_status_icon(&self, size: f32) -> gpui::Stateful<gpui::Div> {
        let (name, color) = if !self.follow_settings.enabled {
            ("follow-off", self.palette.subtle)
        } else if self.follow_controller.is_pinned() {
            (
                "pin",
                if self.palette.dark {
                    ui::PINNED
                } else {
                    ui::PINNED_LIGHT
                },
            )
        } else {
            (
                "follow-on",
                if self.palette.dark {
                    ui::FOLLOWING
                } else {
                    ui::FOLLOWING_LIGHT
                },
            )
        };
        let tip = if self.follow_settings.enabled && self.follow_controller.is_pinned() {
            self.t("Resume")
        } else {
            self.t(self.follow_label())
        };
        let palette = self.palette;
        div()
            .id("follow-status")
            .size(px(size + 4.0))
            .flex()
            .items_center()
            .justify_center()
            .tooltip(move |_, cx| cx.new(|_| ui::Tooltip(tip, palette)).into())
            .child(icon(name).size(px(size)).text_color(rgb(color)))
    }

    fn background(&mut self, shell_left: f32, window: &mut Window) -> impl IntoElement {
        let p = self.palette;
        let zen = self.zen;
        let color = |rgb: u32, alpha: f32| rgba((rgb << 8) | (alpha * 255.0).round() as u32);
        let outer = color(p.outer_color(), p.outer_alpha());
        let shell = color(p.terminal, p.terminal_alpha());
        let corners = if zen {
            Vec::new()
        } else {
            self.shell_corners.images(outer, shell, window)
        };
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if zen {
                    window.paint_quad(gpui::fill(bounds, shell));
                    return;
                }
                let surface = Bounds::new(
                    bounds.origin + point(px(shell_left), px(SHELL_INSET)),
                    size(
                        bounds.size.width - px(shell_left + SHELL_INSET),
                        bounds.size.height - px(SHELL_INSET * 2.0),
                    ),
                );
                shell_surface::paint(bounds, surface, outer, shell, &corners, window);
            },
        )
        .absolute()
        .size_full()
    }

    fn titlebar(
        &self,
        show_sidebar: bool,
        sidebar_width: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.palette;
        let follow_tip = self.t(self.follow_label());
        let control_offset = px(TOOLBAR_CENTER - (SHELL_INSET + TOOLBAR_HEIGHT / 2.0));
        div()
            .h(px(TOOLBAR_HEIGHT))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pr_2()
            .child(
                div()
                    .relative()
                    .top(control_offset)
                    .w(px(if show_sidebar {
                        sidebar_width - 4.0
                    } else {
                        110.0
                    }))
                    .h_full()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .pl(px(78.0))
                    .pr_1()
                    .child(div().flex_1())
                    .child(
                        ui::icon_toggle(
                            "toggle-sidebar",
                            "sidebar",
                            self.t("Toggle sidebar · ⌘B"),
                            p,
                            self.sidebar,
                        )
                        .on_click(cx.listener(|root, _, window, cx| {
                            root.sidebar = !root.sidebar;
                            if let Some(session) = root.active_session() {
                                session.focus(window, cx);
                            }
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .relative()
                    .top(control_offset)
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl_3()
                    .child(
                        ui::keyboard_control(div().id("follow-herdr"), p)
                            .size(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.0))
                            .cursor_pointer()
                            .when(self.follow_settings.enabled, |button| {
                                button
                                    .bg(rgba(p.control_selected(false)))
                                    .hover(move |style| style.bg(rgba(p.control_selected(true))))
                            })
                            .when(!self.follow_settings.enabled, |button| {
                                button
                                    .hover(move |style| style.bg(rgba(p.control_hover())))
                                    .active(move |style| style.bg(rgba(p.control_pressed())))
                            })
                            .tooltip(move |_, cx| cx.new(|_| ui::Tooltip(follow_tip, p)).into())
                            .on_click(cx.listener(|root, _, window, cx| {
                                root.follow_herdr(window, cx);
                            }))
                            .child(
                                div()
                                    .size(px(17.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.0))
                                    .border_1()
                                    .border_color(if self.follow_settings.enabled {
                                        rgb(p.accent)
                                    } else {
                                        rgb(p.border)
                                    })
                                    .child(ui::herdr_icon()),
                            ),
                    )
                    .child(self.follow_status_icon(15.0))
                    .when(
                        self.follow_settings.enabled && self.follow_controller.is_pinned(),
                        |row| {
                            row.child(
                                ui::keyboard_control(div().id("resume-follow"), p)
                                    .active(move |style| style.bg(rgba(p.control_pressed())))
                                    .px_1()
                                    .rounded(px(4.0))
                                    .cursor_pointer()
                                    .text_size(px(11.0))
                                    .text_color(rgb(p.accent))
                                    .hover(move |style| style.bg(rgba(p.control_hover())))
                                    .on_click(cx.listener(|root, _, window, cx| {
                                        root.resume_follow(window, cx)
                                    }))
                                    .child(self.t("Resume")),
                            )
                        },
                    ),
            )
            .child(div().flex_1().min_w_0().h_full().on_mouse_down(
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
                    .top(control_offset)
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        icon_button("restart", "restart", self.t("Restart pane · ⌘⇧R"), p)
                            .on_click(cx.listener(|root, _, window, cx| root.restart(window, cx))),
                    )
                    .child(
                        icon_button("split-right", "split-right", self.t("Split right · ⌘D"), p)
                            .on_click(cx.listener(|root, _, window, cx| {
                                root.split(Axis::Horizontal, window, cx)
                            })),
                    )
                    .child(
                        icon_button("split-down", "split-down", self.t("Split down · ⌘⇧D"), p)
                            .on_click(cx.listener(|root, _, window, cx| {
                                root.split(Axis::Vertical, window, cx)
                            })),
                    )
                    .child(
                        icon_button("zen", "zen", self.t("Zen mode · ⌘⇧Z"), p).on_click(
                            cx.listener(|root, _, window, cx| root.toggle_zen(window, cx)),
                        ),
                    )
                    .child(
                        ui::icon_toggle(
                            "settings",
                            "settings",
                            self.t("Settings · ⌘,"),
                            p,
                            self.settings_window
                                .is_some_and(|settings| settings.entity(cx).is_ok()),
                        )
                        .on_click(
                            cx.listener(|root, _, window, cx| root.open_settings(window, cx)),
                        ),
                    ),
            )
    }

    fn dismiss_tab_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_menu = None;
        self.tab_editor = None;
        self.tab_editor_subscription = None;
        if let Some(session) = self.active_session() {
            session.focus(window, cx);
        } else {
            self.root_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn edit_tab(&mut self, tab_id: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
            return;
        };
        let appearance = tab.appearance.clone();
        let default_title = self.default_tab_label(tab);
        let editor = cx.new(|cx| {
            tab_editor::TabEditor::new(
                tab_id,
                &appearance,
                default_title,
                self.palette,
                self.language,
                window,
                cx,
            )
        });
        self.tab_editor_subscription =
            Some(
                cx.subscribe_in(&editor, window, move |root, _, event, window, cx| {
                    if let tab_editor::EditorEvent::Save(appearance) = event
                        && let Some(tab) = root.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    {
                        tab.appearance = appearance.clone();
                        root.follow_controller.mark_used(tab_id);
                    }
                    root.dismiss_tab_overlay(window, cx);
                }),
            );
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        self.tab_editor = Some(editor);
        self.tab_menu = None;
        cx.notify();
    }

    fn render_tab_menu(
        &self,
        tab_id: usize,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.palette;
        let x = position
            .x
            .min(window.viewport_size().width - px(208.0))
            .max(px(8.0));
        let y = position
            .y
            .min(window.viewport_size().height - px(120.0))
            .max(px(40.0));
        let item = |index: usize, id: &'static str, label: &'static str| {
            ui::keyboard_control(div().id(id), p)
                .track_focus(&self.tab_menu_items[index])
                .h(px(32.0))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .hover(move |style| style.bg(rgba(p.control_hover())))
                .active(move |style| style.bg(rgba(p.control_pressed())))
                .child(self.t(label))
        };
        ui::block_terminal_actions(div().id("tab-menu-overlay"))
            .absolute()
            .inset_0()
            .occlude()
            .key_context("TabMenu")
            .track_focus(&self.tab_menu_focus)
            .tab_group()
            .tab_stop(false)
            .capture_key_down(cx.listener(|root, event: &gpui::KeyDownEvent, window, cx| {
                let stroke = &event.keystroke;
                if stroke.modifiers.control || stroke.modifiers.alt || stroke.modifiers.platform {
                    return;
                }
                let backwards = match stroke.key.as_str() {
                    "up" => true,
                    "down" => false,
                    "tab" => stroke.modifiers.shift,
                    _ => return,
                };
                let current = root
                    .tab_menu_items
                    .iter()
                    .position(|focus| focus.is_focused(window));
                let index = match current {
                    Some(index) if backwards => (index + 2) % 3,
                    Some(index) => (index + 1) % 3,
                    None if backwards => 2,
                    None => 0,
                };
                root.tab_menu_items[index].focus(window, cx);
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _, window, cx| {
                    root.dismiss_tab_overlay(window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|root, _, window, cx| {
                    root.dismiss_tab_overlay(window, cx);
                    cx.stop_propagation();
                }),
            )
            .child(
                div()
                    .id("tab-menu")
                    .absolute()
                    .left(x)
                    .top(y)
                    .w(px(200.0))
                    .p_1()
                    .rounded(px(8.0))
                    .bg(rgb(p.panel))
                    .border_1()
                    .border_color(rgb(p.border))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(item(0, "customize-tab", "Customize tab…").on_click(
                        cx.listener(move |root, _, window, cx| root.edit_tab(tab_id, window, cx)),
                    ))
                    .child(
                        item(1, "reset-tab", "Restore default").on_click(cx.listener(
                            move |root, _, window, cx| {
                                if let Some(tab) = root.tabs.iter_mut().find(|tab| tab.id == tab_id)
                                {
                                    tab.appearance = Default::default();
                                }
                                root.dismiss_tab_overlay(window, cx);
                            },
                        )),
                    )
                    .child(item(2, "close-tab", "Close tab").on_click(cx.listener(
                        move |root, _, window, cx| {
                            root.dismiss_tab_overlay(window, cx);
                            if let Some(index) = root.tabs.iter().position(|tab| tab.id == tab_id) {
                                root.close(index, window, cx);
                            }
                        },
                    ))),
            )
            .into_any_element()
    }

    fn sidebar(&self, width: f32, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.palette;
        let mut list = div().flex().flex_col().gap(px(2.0));
        for (index, tab) in self.tabs.iter().enumerate() {
            let selected = index == self.active;
            let tab_id = tab.id;
            let label = self.tab_label(tab);
            let panes = tab.pane_count();
            list = list.child(
                ui::keyboard_control(div().id(("session", tab_id)), p)
                    .map(ui::choice_navigation)
                    .h(px(28.0))
                    .pl(px(10.0))
                    .pr_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .bg(rgba(p.tab_background(selected, false)))
                    .hover(move |s| s.bg(rgba(p.tab_background(selected, true))))
                    .on_click(
                        cx.listener(move |root, _, window, cx| root.select(index, window, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |root, event: &gpui::MouseDownEvent, window, cx| {
                            root.dragging = None;
                            root.resizing_sidebar = false;
                            root.tab_menu = Some((tab_id, event.position));
                            root.tab_menu_focus.focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
                    .when_some(self.follow_controller.tab_icon(tab), |row, name| {
                        row.child(icon(name).size(px(14.0)).text_color(rgb(if selected {
                            p.accent
                        } else {
                            p.muted
                        })))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.0))
                            .font_weight(if selected {
                                gpui::FontWeight::MEDIUM
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .text_color(rgb(if selected { p.text } else { p.muted }))
                            .child(label),
                    )
                    .when(panes > 1, |e| {
                        e.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(icon("split-right").size(px(11.0)))
                                .child(self.label(panes.to_string())),
                        )
                    }),
            );
        }
        div()
            .w(px(width - 4.0))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .pl_3()
                    .pr_1()
                    .h(px(40.0))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().flex_1().child(self.label("TABS")))
                    .child(
                        icon_button("new-tab", "plus", self.t("New tab · ⌘T"), p).on_click(
                            cx.listener(|root, _, window, cx| root.new_session(window, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .id("tab-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_2()
                    .child(list),
            )
    }

    fn sidebar_resize_handle(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("sidebar-resize-handle")
            .w(px(4.0))
            .h_full()
            .flex_shrink_0()
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|root, _, _, cx| {
                    root.dragging = None;
                    root.resizing_sidebar = true;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
    }

    fn pane(&self, id: usize, multiple: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            return div().into_any_element();
        };
        let active = self.tabs[self.active].active_pane == id;
        let root = cx.entity().downgrade();
        let p = self.palette;
        if let Some(view) = &session.view {
            let channels = |color: u32| [(color >> 16) as u8, (color >> 8) as u8, color as u8];
            view.update(cx, |view, cx| {
                view.set_appearance(
                    channels(p.text),
                    channels(p.terminal),
                    if p.glass { 0.0 } else { 1.0 },
                    cx,
                )
            });
        }
        let content = div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .text_size(px(self.font_size))
            .line_height(px(self.font_size * 1.35))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |root, _, window, cx| root.focus_pane(id, window, cx)),
            )
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let _ = root
                            .update(cx, |root, cx| root.resize_terminal(id, bounds, window, cx));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .children(session.view.clone())
            .when(session.view.is_none(), |e| {
                e.child(
                    div()
                        .p_4()
                        .child(self.language.dynamic_text(&session.status)),
                )
            });
        div()
            .id(("pane", id))
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .when(multiple && !self.zen, |e| {
                e.child(
                    div()
                        .h(px(26.0))
                        .flex_shrink_0()
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(if active {
                            rgba((p.accent << 8) | 100)
                        } else {
                            rgb(p.divider)
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |root, _, window, cx| root.focus_pane(id, window, cx)),
                        )
                        .child(dot(session.running))
                        .child(self.label(format!("Pane {id}")))
                        .child(div().flex_1())
                        .child(self.label(format!("{} × {}", session.grid.0, session.grid.1)))
                        .child(
                            ui::keyboard_control(div().id(("close-pane", id)), p)
                                .size(px(20.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgba(p.control_hover())))
                                .active(move |style| style.bg(rgba(p.control_pressed())))
                                .on_click(cx.listener(move |root, _, window, cx| {
                                    cx.stop_propagation();
                                    root.close_pane(id, window, cx);
                                }))
                                .child(icon("close").size(px(11.0)).text_color(rgb(p.muted))),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .px(px(if self.zen { 4.0 } else { 10.0 }))
                    .pb(px(if self.zen { 4.0 } else { 10.0 }))
                    .pt(px(4.0))
                    .flex()
                    .flex_col()
                    .child(content),
            )
            .into_any_element()
    }

    fn pane_tree(
        &self,
        layout: &Layout,
        multiple: bool,
        divider_clip: Option<Bounds<Pixels>>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match layout {
            Layout::Leaf(id) => self.pane(*id, multiple, cx),
            Layout::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                let (id, axis, ratio) = (*id, *axis, *ratio);
                let root = cx.entity().downgrade();
                let accent = self.palette.accent;
                let divider = self.palette.divider;
                div()
                    .relative()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .when(axis == Axis::Vertical, |e| e.flex_col())
                    .child(
                        canvas(
                            move |bounds, _, cx| {
                                let _ = root.update(cx, |root, _| {
                                    root.split_bounds.insert(id, bounds);
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .min_h_0()
                            .flex_shrink_0()
                            .when(axis == Axis::Horizontal, |e| e.w(relative(ratio)).h_full())
                            .when(axis == Axis::Vertical, |e| e.h(relative(ratio)).w_full())
                            .child(self.pane_tree(first, multiple, divider_clip, cx)),
                    )
                    .child(
                        div()
                            .id(("divider", id))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(axis == Axis::Horizontal, |e| {
                                e.w(px(4.0)).h_full().cursor_col_resize()
                            })
                            .when(axis == Axis::Vertical, |e| {
                                e.h(px(4.0)).w_full().cursor_row_resize()
                            })
                            .child(
                                canvas(
                                    |bounds, window, _| {
                                        window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
                                    },
                                    move |bounds, hitbox, window, _| {
                                        let hovered = hitbox.is_hovered(window);
                                        let radius = px((SHELL_RADIUS * window.scale_factor())
                                            .ceil()
                                            / window.scale_factor());
                                        // Only trim the ends that would cross a bottom corner.
                                        let clip = divider_clip.map(|shell| {
                                            let vertical = axis == Axis::Horizontal;
                                            let near_side = bounds.left() < shell.left() + radius
                                                || bounds.right() > shell.right() - radius;
                                            let crosses_bottom =
                                                bounds.bottom() > shell.bottom() - radius;
                                            if vertical && near_side && crosses_bottom {
                                                Bounds::new(
                                                    shell.origin,
                                                    size(
                                                        shell.size.width,
                                                        shell.size.height - radius,
                                                    ),
                                                )
                                            } else if !vertical && crosses_bottom {
                                                Bounds::new(
                                                    shell.origin + point(radius, px(0.0)),
                                                    size(
                                                        shell.size.width - radius * 2.0,
                                                        shell.size.height,
                                                    ),
                                                )
                                            } else {
                                                shell
                                            }
                                        });
                                        // Clip only paint: the full divider remains draggable.
                                        window.with_content_mask(
                                            clip.map(|bounds| gpui::ContentMask { bounds }),
                                            |window| {
                                                if hovered {
                                                    window.paint_quad(gpui::fill(
                                                        bounds,
                                                        rgba((accent << 8) | 50),
                                                    ));
                                                }
                                                let line = match axis {
                                                    Axis::Horizontal => Bounds::new(
                                                        point(
                                                            bounds.center().x - px(0.5),
                                                            bounds.top(),
                                                        ),
                                                        size(px(1.0), bounds.size.height),
                                                    ),
                                                    Axis::Vertical => Bounds::new(
                                                        point(
                                                            bounds.left(),
                                                            bounds.center().y - px(0.5),
                                                        ),
                                                        size(bounds.size.width, px(1.0)),
                                                    ),
                                                };
                                                window.paint_quad(gpui::fill(line, rgb(divider)));
                                            },
                                        );
                                    },
                                )
                                .size_full(),
                            )
                            .on_hover(|_, window, _| window.refresh())
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |root, _, _, cx| {
                                    root.mark_local_edit();
                                    root.dragging = Some((id, axis));
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(div().flex_1().min_w_0().min_h_0().child(self.pane_tree(
                        second,
                        multiple,
                        divider_clip,
                        cx,
                    )))
                    .into_any_element()
            }
        }
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = window
            .window_handle()
            .downcast::<Self>()
            .expect("terminal window root");
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                window.display(cx).map(|display| display.id()),
                size(px(680.0), px(620.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("MonoTerm Settings".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.0), px(14.0))),
            }),
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: Some(size(px(600.0), px(540.0))),
            ..Default::default()
        };
        // Opening a GPUI window draws it synchronously; release the owner first.
        cx.defer(move |cx| {
            let Ok(existing) = owner.read_with(cx, |root, _| root.settings_window) else {
                return;
            };
            if let Some(settings) = existing
                && settings
                    .update(cx, |settings, window, cx| settings.focus(window, cx))
                    .is_ok()
            {
                return;
            }
            let Ok(entity) = owner.entity(cx) else { return };
            let snapshot = settings::SettingsSnapshot::from(entity.read(cx));
            let updater = entity.read(cx).updater.clone();
            let result = cx.open_window(options, |window, cx| {
                cx.new(|cx| {
                    settings::SettingsWindow::new(owner, entity, snapshot, updater, window, cx)
                })
            });
            let _ = owner.update(cx, |root, _, cx| {
                match result {
                    Ok(settings) => root.settings_window = Some(settings),
                    Err(error) => root.notice = Some(format!("Open settings: {error}")),
                }
                cx.notify();
            });
        });
    }
}

impl Render for TerminalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.palette != self.appearance.palette(Self::system_dark(window)) {
            self.apply_appearance(window, cx, false);
        }
        let title = self
            .tabs
            .get(self.active)
            .map(|tab| {
                format!(
                    "{} — MonoTerm",
                    tab.appearance.title.clone().unwrap_or_else(|| {
                        if self.follow_controller.is_companion(tab.id) {
                            self.default_tab_label(tab)
                        } else {
                            path_name(&self.directory)
                        }
                    })
                )
            })
            .unwrap_or_else(|| "MonoTerm".into());
        if window.window_title() != title {
            window.set_window_title(&title);
        }
        let missing_overlay_target = self
            .tab_editor
            .as_ref()
            .map(|editor| editor.read(cx).tab_id)
            .or_else(|| self.tab_menu.map(|(id, _)| id))
            .is_some_and(|id| !self.tabs.iter().any(|tab| tab.id == id));
        if missing_overlay_target {
            self.dismiss_tab_overlay(window, cx);
        }
        if let Some(editor) = &self.tab_editor {
            editor.update(cx, |editor, cx| {
                editor.set_style(self.palette, self.language, cx)
            });
        }
        let notice = self
            .notice
            .clone()
            .or_else(|| self.appearance_notice.clone())
            .or_else(|| {
                self.active_session()
                    .and_then(|session| session.notice.clone())
            });
        let viewport_width = f32::from(window.viewport_size().width);
        let show_sidebar = !self.zen && self.sidebar && viewport_width >= 760.0;
        // Clamp only the rendered layout. The stored width is the user's choice,
        // so shrinking the window temporarily does not discard it.
        let sidebar_width = if show_sidebar {
            let width = self
                .sidebar_width
                .clamp(180.0, (viewport_width - 420.0).min(480.0));
            // Image tiles and layout must share the same device-pixel boundary.
            (width * window.scale_factor()).round() / window.scale_factor()
        } else {
            self.sidebar_width
        };
        if !show_sidebar {
            self.resizing_sidebar = false;
        }
        let p = self.palette;
        let shell_left = if show_sidebar {
            sidebar_width
        } else {
            SHELL_INSET
        };
        let divider_clip = (!self.zen).then(|| {
            Bounds::new(
                point(px(shell_left), px(SHELL_INSET)),
                size(
                    window.viewport_size().width - px(shell_left + SHELL_INSET),
                    window.viewport_size().height - px(SHELL_INSET * 2.0),
                ),
            )
        });
        let content = self
            .tabs
            .get(self.active)
            .map(|t| self.pane_tree(&t.layout, t.pane_count() > 1, divider_clip, cx));
        div()
            .track_focus(&self.root_focus)
            .tab_group()
            .tab_stop(false)
            .on_action(cx.listener(|root, _: &ui::FocusControls, window, cx| {
                if root.tab_editor.is_none() && root.tab_menu.is_none() {
                    root.root_focus.focus(window, cx);
                    window.focus_next(cx);
                }
                cx.stop_propagation();
            }))
            .on_key_down(cx.listener(|root, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape"
                    && !event.keystroke.modifiers.modified()
                    && root.tab_editor.is_none()
                    && root.tab_menu.is_none()
                {
                    if let Some(session) = root.active_session() {
                        session.focus(window, cx);
                    }
                    cx.stop_propagation();
                }
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(self.background(shell_left, window))
            .text_color(rgb(p.text))
            .font_family(".SystemUIFont")
            .on_action(cx.listener(|root, _: &NewSession, window, cx| root.new_session(window, cx)))
            .on_action(cx.listener(|root, _: &CloseSession, window, cx| {
                if let Some(tab) = root.tabs.get(root.active) {
                    root.close_pane(tab.active_pane, window, cx);
                }
            }))
            .on_action(
                cx.listener(|root, _: &CloseTab, window, cx| root.close(root.active, window, cx)),
            )
            .on_action(cx.listener(|root, _: &Restart, window, cx| root.restart(window, cx)))
            .on_action(
                cx.listener(|root, _: &NextSession, window, cx| root.cycle(true, window, cx)),
            )
            .on_action(
                cx.listener(|root, _: &PreviousSession, window, cx| root.cycle(false, window, cx)),
            )
            .on_action(
                cx.listener(|root, _: &NextPane, window, cx| root.cycle_pane(true, window, cx)),
            )
            .on_action(
                cx.listener(|root, _: &PreviousPane, window, cx| {
                    root.cycle_pane(false, window, cx)
                }),
            )
            .on_action(cx.listener(|root, _: &SplitRight, window, cx| {
                root.split(Axis::Horizontal, window, cx)
            }))
            .on_action(
                cx.listener(|root, _: &SplitDown, window, cx| {
                    root.split(Axis::Vertical, window, cx)
                }),
            )
            .on_action(
                cx.listener(|root, _: &ResumeFollow, window, cx| root.resume_follow(window, cx)),
            )
            .on_action(
                cx.listener(|root, _: &ReturnLocal, window, cx| root.return_local(window, cx)),
            )
            .on_action(cx.listener(|root, _: &OpenHerdrWindow, window, cx| {
                root.open_herdr_window(window, cx)
            }))
            .on_action(cx.listener(|root, _: &ToggleZen, window, cx| root.toggle_zen(window, cx)))
            .on_action(cx.listener(|root, _: &IncreaseFont, _, cx| root.adjust_font(1.0, cx)))
            .on_action(cx.listener(|root, _: &DecreaseFont, _, cx| root.adjust_font(-1.0, cx)))
            .on_action(cx.listener(|root, _: &ResetFont, _, cx| {
                root.adjust_font(15.0 - root.font_size, cx);
            }))
            .on_action(cx.listener(|root, _: &ToggleSidebar, _, cx| {
                root.sidebar = !root.sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|root, _: &ToggleSettings, window, cx| {
                root.open_settings(window, cx);
            }))
            .on_action(cx.listener(|root, _: &CustomizeTab, window, cx| {
                if let Some(tab) = root.tabs.get(root.active) {
                    root.edit_tab(tab.id, window, cx);
                }
            }))
            .on_action(cx.listener(|root, _: &DismissTabMenu, window, cx| {
                root.dismiss_tab_overlay(window, cx);
            }))
            .capture_any_mouse_down(cx.listener(
                |root, event: &gpui::MouseDownEvent, window, cx| {
                    if root.zen
                        && event.button == MouseButton::Left
                        && event.modifiers.platform
                        && event.modifiers.alt
                    {
                        window.start_window_move();
                        cx.stop_propagation();
                    }
                },
            ))
            .on_mouse_move(
                cx.listener(|root, event: &gpui::MouseMoveEvent, window, cx| {
                    if root.resizing_sidebar {
                        if event.pressed_button != Some(MouseButton::Left) {
                            root.resizing_sidebar = false;
                            return;
                        }
                        let maximum =
                            (f32::from(window.viewport_size().width) - 420.0).clamp(180.0, 480.0);
                        root.sidebar_width =
                            (f32::from(event.position.x) + 2.0).clamp(180.0, maximum);
                        cx.notify();
                        return;
                    }
                    if let Some((id, axis)) = root.dragging {
                        if event.pressed_button != Some(MouseButton::Left) {
                            root.dragging = None;
                            return;
                        }
                        if let Some(bounds) = root.split_bounds.get(&id) {
                            let ratio = match axis {
                                Axis::Horizontal => {
                                    f32::from(event.position.x - bounds.origin.x)
                                        / f32::from(bounds.size.width)
                                }
                                Axis::Vertical => {
                                    f32::from(event.position.y - bounds.origin.y)
                                        / f32::from(bounds.size.height)
                                }
                            };
                            if let Some(tab) = root.tabs.get_mut(root.active)
                                && tab.layout.set_ratio(id, ratio)
                            {
                                cx.notify();
                            }
                        }
                    }
                }),
            )
            .capture_any_mouse_up(cx.listener(|root, _, _, cx| {
                let sidebar_changed = std::mem::take(&mut root.resizing_sidebar);
                if root.dragging.take().is_some() || sidebar_changed {
                    cx.notify();
                }
            }))
            .when(!self.zen, |e| {
                e.child(div().h(px(SHELL_INSET)).flex_shrink_0())
                    .child(self.titlebar(show_sidebar, sidebar_width, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .when(show_sidebar, |e| {
                        e.child(self.sidebar(sidebar_width, cx))
                            .child(self.sidebar_resize_handle(cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .when(!self.zen && !show_sidebar, |e| e.ml(px(SHELL_INSET)))
                            .when(!self.zen, |e| {
                                e.mr(px(SHELL_INSET))
                                    .mb(px(SHELL_INSET))
                                    .rounded_b(px(SHELL_RADIUS))
                                    .overflow_hidden()
                                    .px(px(1.0))
                                    .pb(px(1.0))
                            })
                            .child(div().flex_1().min_h_0().min_w_0().children(content).when(
                                self.tabs.is_empty(),
                                |element| {
                                    element.child(
                                        div()
                                            .size_full()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .justify_center()
                                            .gap_3()
                                            .child(self.t(if self.follow_settings.enabled {
                                                "Waiting for a Herdr agent"
                                            } else {
                                                "No local terminals"
                                            }))
                                            .when(self.follow_settings.enabled, |element| {
                                                element
                                                    .child(self.label(self.follow_status.clone()))
                                            })
                                            .child(
                                                self.label("Press ⌘T to open a local terminal."),
                                            ),
                                    )
                                },
                            )),
                    ),
            )
            .when_some(notice, |element, notice| {
                element.child(
                    div()
                        .absolute()
                        .bottom(px(14.0))
                        .right(px(16.0))
                        .max_w(px((viewport_width - 32.0).min(480.0)))
                        .p_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(6.0))
                        .bg(rgb(p.panel))
                        .child(div().flex_1().min_w_0().child(self.label(notice)))
                        .child(
                            icon_button("dismiss-notice", "close", self.t("Dismiss"), p).on_click(
                                cx.listener(|root, _, window, cx| {
                                    root.notice = None;
                                    root.appearance_notice = None;
                                    if let Some(id) =
                                        root.tabs.get(root.active).map(|tab| tab.active_pane)
                                        && let Some(session) = root
                                            .sessions
                                            .iter_mut()
                                            .find(|session| session.id == id)
                                    {
                                        session.notice = None;
                                        session.focus(window, cx);
                                    }
                                    cx.notify();
                                }),
                            ),
                        ),
                )
            })
            .when(self.zen, |element| {
                element.child(
                    div()
                        .id("zen-follow-status")
                        .absolute()
                        .top(px(10.0))
                        .right(px(10.0))
                        .h(px(30.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .bg(rgba((p.panel << 8) | 0x26))
                        .when(
                            self.follow_settings.enabled && self.follow_controller.is_pinned(),
                            |element| ui::keyboard_control(element, p).cursor_pointer(),
                        )
                        .on_click(cx.listener(|root, _, window, cx| {
                            if root.follow_settings.enabled && root.follow_controller.is_pinned() {
                                root.resume_follow(window, cx);
                            }
                        }))
                        .child(self.follow_status_icon(15.0)),
                )
            })
            .when_some(self.tab_menu, |element, (id, position)| {
                element.child(self.render_tab_menu(id, position, window, cx))
            })
            .children(self.tab_editor.clone())
            .child(toast::layer(&self.toast, p, cx).when(!self.zen, |layer| {
                layer.left(px(shell_left)).right(px(SHELL_INSET))
            }))
            .into_any_element()
    }
}

fn path_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
fn shell_name(shell: &str) -> String {
    path_name(Path::new(shell))
}
fn set_menus(language: locale::Language, cx: &mut App) {
    let t = |text| language.text(text);
    cx.set_menus(vec![
        Menu {
            name: "MonoTerm".into(),
            items: vec![
                MenuItem::action(t("New Tab"), NewSession),
                MenuItem::separator(),
                MenuItem::action(t("Settings"), ToggleSettings),
                MenuItem::action(t("Quit MonoTerm"), Quit),
            ],
        },
        Menu {
            name: t("Edit").into(),
            items: vec![
                MenuItem::os_action(t("Copy"), Copy, OsAction::Copy),
                MenuItem::os_action(t("Paste"), Paste, OsAction::Paste),
                MenuItem::os_action(t("Select All"), SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action(t("Customize tab…"), CustomizeTab),
            ],
        },
        Menu {
            name: t("View").into(),
            items: vec![
                MenuItem::action(t("Split Right"), SplitRight),
                MenuItem::action(t("Split Down"), SplitDown),
                MenuItem::action(t("Next Pane"), NextPane),
                MenuItem::action(t("Previous Pane"), PreviousPane),
                MenuItem::separator(),
                MenuItem::action(t("Toggle Zen Mode"), ToggleZen),
                MenuItem::action(t("Toggle Sidebar"), ToggleSidebar),
                MenuItem::separator(),
                MenuItem::action(t("Herdr agents"), ResumeFollow),
                MenuItem::action(t("Return to local terminals"), ReturnLocal),
                MenuItem::action(t("Open Herdr window"), OpenHerdrWindow),
                MenuItem::separator(),
                MenuItem::action(t("Close Pane"), CloseSession),
                MenuItem::action(t("Close Tab"), CloseTab),
            ],
        },
    ]);
}

fn main() {
    let directory = match std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
        .canonicalize()
    {
        Ok(directory) if directory.is_dir() => directory,
        _ => {
            eprintln!("Pass an existing working directory as the first argument.");
            std::process::exit(2);
        }
    };
    Application::new()
        .with_assets(ui::Icons)
        .run(move |cx: &mut App| {
            if let Err(error) = window_style::set_application_icon() {
                eprintln!("Set application icon: {error}");
            }
            cx.set_quit_mode(QuitMode::LastWindowClosed);
            cx.on_action(|_: &Quit, cx| cx.quit());
            text_input::init(cx);
            tab_editor::init(cx);
            ui::init(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-shift-e", CustomizeTab, None),
                KeyBinding::new("escape", DismissTabMenu, Some("TabMenu")),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-w", CloseSession, None),
                KeyBinding::new("cmd-shift-w", CloseTab, None),
                KeyBinding::new("cmd-t", NewSession, None),
                KeyBinding::new("ctrl-tab", NextSession, None),
                KeyBinding::new("ctrl-shift-tab", PreviousSession, None),
                KeyBinding::new("cmd-b", ToggleSidebar, None),
                KeyBinding::new("cmd-,", ToggleSettings, None),
                KeyBinding::new("escape", settings::CloseSettings, Some("Settings")),
                KeyBinding::new("cmd-d", SplitRight, None),
                KeyBinding::new("cmd-shift-d", SplitDown, None),
                KeyBinding::new("cmd-alt-right", NextPane, None),
                KeyBinding::new("cmd-alt-left", PreviousPane, None),
                KeyBinding::new("cmd-shift-z", ToggleZen, None),
                KeyBinding::new("cmd-c", Copy, Some("Terminal")),
                KeyBinding::new("cmd-v", Paste, Some("Terminal")),
                KeyBinding::new("cmd-a", SelectAll, Some("Terminal")),
                KeyBinding::new("cmd-=", IncreaseFont, None),
                KeyBinding::new("cmd-+", IncreaseFont, None),
                KeyBinding::new("cmd--", DecreaseFont, None),
                KeyBinding::new("cmd-0", ResetFont, None),
                KeyBinding::new("cmd-shift-r", Restart, None),
            ]);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(780.0)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("MonoTerm".into()),
                    appears_transparent: true,
                    // The inset grows downward on this host; 15 puts the native
                    // buttons on the shell controls' y=22 center.
                    traffic_light_position: Some(point(px(12.0), px(15.0))),
                }),
                window_background: WindowBackgroundAppearance::Blurred,
                window_min_size: Some(size(px(640.0), px(400.0))),
                ..Default::default()
            };
            match cx.open_window(options, |window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    false
                });
                cx.new(|cx| TerminalApp::new(directory, window, cx))
            }) {
                Ok(_) => cx.activate(true),
                Err(error) => {
                    eprintln!("Open terminal window: {error}");
                    cx.quit();
                }
            }
        });
}
