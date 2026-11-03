//! Native interactive window for Herdr's daemon-owned terminal surface.
use crate::{
    TerminalApp,
    appearance::Palette,
    herdr_connection::{CompositionToken, HerdrConnection, InputTarget, InputToken},
    herdr_render::{self, CellPainter, GlyphCache},
    locale::Language,
    shell_surface, toast, ui,
    window_chrome::{self, SHELL_INSET, SHELL_RADIUS, TOOLBAR_CENTER, TOOLBAR_HEIGHT},
};
use gpui::{
    App, Bounds, Context, DispatchPhase, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Font, InputHandler, KeyDownEvent, Modifiers, MouseButton, Pixels, Point, Render,
    Subscription, TextAlign, TextRun, UTF16Selection, Window, canvas, div, fill, point, prelude::*,
    px, rgb, size,
};
use gpui_ghostty_terminal::view::ScrollAccumulator;
use herdr_protocol::{
    ClientKeyCode, ClientKeyKind, ClientMouseButton, ClientMouseGeometry, ClientMouseKind,
    ClientMousePosition, ClientPaneInputEvent, ClientSurfaceSize, SurfaceRect,
};
use std::ops::Range;

#[path = "herdr_actions.rs"]
mod actions;
#[path = "herdr_chrome.rs"]
mod chrome;
#[path = "herdr_indicators.rs"]
mod indicators;
#[path = "herdr_links.rs"]
mod links;
#[path = "herdr_sidebar.rs"]
mod sidebar;
#[path = "herdr_sidebar_menu.rs"]
mod sidebar_menu;
#[path = "herdr_sidebar_style.rs"]
mod sidebar_style;
#[path = "herdr_tab_menu.rs"]
mod tab_menu;
#[path = "herdr_workspace_menu.rs"]
mod workspace_menu;

/// The IME paints its marked text at the terminal cursor; a cursor quad there
/// would cover the first preedit glyph.
fn shows_terminal_cursor(focused: bool, composing: bool) -> bool {
    focused && !composing
}

#[derive(Clone, Copy)]
enum SidebarDrag {
    Width,
    Sections,
}

pub(crate) struct HerdrWindow {
    connection: Entity<HerdrConnection>,
    owner: Entity<TerminalApp>,
    focus_handle: FocusHandle,
    font: Font,
    font_size: f32,
    glyph_cache: GlyphCache,
    palette: Palette,
    toast: Entity<toast::Toast>,
    blur_radius: u32,
    language: Language,
    sidebar_visible: bool,
    sidebar_view: sidebar_menu::SidebarView,
    sidebar_menu: Option<sidebar_menu::SidebarMenu>,
    sidebar_style: sidebar_style::SidebarStyle,
    sidebar_width: f32,
    sidebar_ratio: f32,
    sidebar_drag: Option<SidebarDrag>,
    workspace_menu: Option<workspace_menu::WorkspaceMenu>,
    tab_menu: Option<tab_menu::TabMenu>,
    shell_corners: shell_surface::CornerCache,
    sidebar_scroll: [gpui::ScrollHandle; 2],
    collapsed_repos: std::collections::HashSet<String>,
    tab_scroll: gpui::ScrollHandle,
    tab_reveal: chrome::TabReveal,
    indicator_started: std::time::Instant,
    indicator_elapsed: u128,
    indicator_tick: Option<gpui::Task<()>>,
    bounds: Bounds<Pixels>,
    cell: gpui::Size<Pixels>,
    composition: String,
    composition_selection: Range<usize>,
    composition_token: Option<CompositionToken>,
    composition_cancelled: bool,
    cursor_bounds: Bounds<Pixels>,
    selection: Option<((u16, u16), (u16, u16))>,
    selecting: bool,
    selection_dragged: bool,
    selection_rect: SurfaceRect,
    selection_token: Option<InputToken>,
    links: links::LinkState,
    scroll: ScrollAccumulator,
    scroll_target: Option<InputTarget>,
    error: Option<String>,
    trace: bool,
    input_started: Option<(std::time::Instant, u64)>,
    _subscriptions: Vec<Subscription>,
}

impl HerdrWindow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        connection: Entity<HerdrConnection>,
        owner: Entity<TerminalApp>,
        font: Font,
        font_size: f32,
        palette: Palette,
        blur_radius: u32,
        language: Language,
        preferences: crate::window_preferences::HerdrPreferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observation = cx.observe(&connection, |view, _, cx| {
            view.validate_links(cx);
            if view
                .composition_token
                .as_ref()
                .is_some_and(|token| !view.connection.read(cx).composition_is_current(token))
            {
                view.composition.clear();
                view.composition_selection = 0..0;
                view.composition_cancelled = true;
            }
            if view.selection_token.is_some()
                && view.selection_token != view.connection.read(cx).input_token()
            {
                view.selection = None;
                view.selection_token = None;
                view.selecting = false;
                view.selection_dragged = false;
            }
            cx.notify();
        });
        let activation = cx.observe_window_activation(window, |view, window, cx| {
            if !window.is_window_active() {
                view.cancel_links();
                if view.sidebar_drag.take().is_some() {
                    view.save_layout(cx);
                }
            }
            let result = view.connection.update(cx, |connection, cx| {
                let result = connection.set_focus(window.is_window_active());
                cx.notify();
                result
            });
            view.record(result, cx);
        });
        let appearance = cx.observe_window_appearance(window, |view, window, cx| {
            view.owner
                .update(cx, |root, cx| root.apply_appearance(window, cx, false));
        });
        let close_connection = connection.clone();
        let shared = owner.clone();
        window.on_window_should_close(cx, move |_, cx| {
            shared.update(cx, |root, cx| {
                root.herdr_window = None;
                cx.notify();
            });
            close_connection.update(cx, |connection, cx| {
                if let Err(error) = connection.set_surface_active(false) {
                    eprintln!("Herdr detach: {error}");
                }
                if let Err(error) = connection.set_focus(false) {
                    eprintln!("Herdr focus: {error}");
                }
                cx.notify();
            });
            true
        });
        let mut view = Self {
            connection,
            owner,
            focus_handle: cx.focus_handle(),
            font,
            font_size,
            glyph_cache: GlyphCache::default(),
            palette,
            toast: toast::new(window, palette, cx),
            blur_radius,
            language,
            sidebar_visible: preferences.visible,
            sidebar_view: preferences.view,
            sidebar_menu: None,
            sidebar_style: sidebar_style::SidebarStyle::load(),
            sidebar_width: preferences.width,
            sidebar_ratio: preferences.ratio,
            sidebar_drag: None,
            workspace_menu: None,
            tab_menu: None,
            shell_corners: shell_surface::CornerCache::default(),
            sidebar_scroll: std::array::from_fn(|_| gpui::ScrollHandle::new()),
            collapsed_repos: preferences.collapsed_repos,
            tab_scroll: gpui::ScrollHandle::new(),
            tab_reveal: chrome::TabReveal::default(),
            indicator_started: std::time::Instant::now(),
            indicator_elapsed: 0,
            indicator_tick: None,
            bounds: Bounds::default(),
            cell: size(px(9.0), px(20.0)),
            composition: String::new(),
            composition_selection: 0..0,
            composition_token: None,
            composition_cancelled: false,
            cursor_bounds: Bounds::default(),
            selection: None,
            selecting: false,
            selection_dragged: false,
            selection_rect: SurfaceRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            selection_token: None,
            links: links::LinkState::default(),
            scroll: ScrollAccumulator::default(),
            scroll_target: None,
            error: None,
            trace: std::env::var_os("MONOTERM_HERDR_TRACE").is_some(),
            input_started: None,
            _subscriptions: vec![observation, activation, appearance],
        };
        view._subscriptions
            .push(cx.on_blur(&view.focus_handle, window, |view, _, cx| {
                view.cancel_links();
                cx.notify();
            }));
        view.apply_material(window, cx);
        view.focus(window, cx);
        view
    }

    pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        self.focus_handle.focus(window, cx);
        let result = self.connection.update(cx, |connection, cx| {
            let result = connection
                .set_surface_active(true)
                .and_then(|()| connection.set_focus(true));
            cx.notify();
            result
        });
        self.record(result, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_preferences(
        &mut self,
        font: Font,
        font_size: f32,
        palette: Palette,
        blur_radius: u32,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.font != font || self.font_size != font_size {
            self.cancel_links();
            self.glyph_cache.clear();
        }
        self.font = font;
        self.font_size = font_size;
        self.palette = palette;
        self.blur_radius = blur_radius;
        self.language = language;
        self.apply_material(window, cx);
        cx.notify();
    }

    fn adjust_font(&mut self, delta: f32, cx: &mut Context<Self>) {
        if self.link_menu_open() || !self.composition.is_empty() {
            return;
        }
        self.cancel_links();
        let owner = self.owner.clone();
        cx.defer(move |cx| {
            owner.update(cx, |root, cx| root.adjust_font(delta, cx));
        });
    }

    fn apply_material(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result =
            crate::window_style::apply_material(window, self.palette.alpha, self.blur_radius);
        if let Err(error) = result {
            self.error = Some(error);
        }
        let palette = self.palette;
        let color = |value: u32| {
            let [_, r, g, b] = value.to_be_bytes();
            herdr_protocol::ClientHostColor { r, g, b }
        };
        let theme = herdr_client::HostTheme {
            foreground: color(palette.text),
            background: color(palette.terminal),
            palette: std::array::from_fn(|index| {
                color(herdr_render::color(0x01000000 | index as u32, 0))
            }),
            appearance: if palette.dark {
                herdr_protocol::ClientHostAppearance::Dark
            } else {
                herdr_protocol::ClientHostAppearance::Light
            },
        };
        self.connection.update(cx, |connection, cx| {
            if let Err(error) = connection.set_host_theme(theme) {
                eprintln!("Herdr theme: {error}");
            }
            cx.notify();
        });
    }

    fn record(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        let error = result.err();
        if self.error != error {
            self.error = error;
            cx.notify();
        }
    }

    fn send_to(
        &mut self,
        target: Option<&InputTarget>,
        events: Vec<ClientPaneInputEvent>,
        cx: &mut Context<Self>,
    ) {
        if !events.iter().all(|event| {
            matches!(
                event,
                ClientPaneInputEvent::Mouse {
                    kind: ClientMouseKind::Moved,
                    ..
                }
            )
        }) {
            self.cancel_links();
        }
        if self.selection.take().is_some() {
            cx.notify();
        }
        self.selecting = false;
        self.selection_dragged = false;
        self.selection_token = None;
        let result = self.connection.update(cx, |connection, cx| {
            let revision = connection.revision();
            let result = match target {
                Some(target) => connection.send_input_to_visible(target, events),
                None => connection.send_input(events),
            };
            if connection.revision() != revision {
                cx.notify();
            }
            result
        });
        if self.trace && result.is_ok() && self.input_started.is_none() {
            self.input_started = Some((
                std::time::Instant::now(),
                self.connection
                    .read(cx)
                    .display_surface
                    .as_ref()
                    .map_or(0, |surface| surface.surface_revision),
            ));
        }
        self.record(result, cx);
    }

    fn send(&mut self, event: ClientPaneInputEvent, cx: &mut Context<Self>) {
        self.send_to(None, vec![event], cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_menu.is_some() || self.sidebar_menu.is_some() || self.tab_menu.is_some() {
            return;
        }
        let key = &event.keystroke;
        if self.link_menu_open() {
            self.cancel_links();
            cx.notify();
            cx.stop_propagation();
            return;
        }
        self.cancel_links();
        if !self.focus_handle.is_focused(window) {
            if key.key == "escape" && !key.modifiers.modified() {
                self.focus_handle.focus(window, cx);
            } else {
                ui::traverse_controls(event, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        // Platform shortcuts dispatch through the same GPUI actions as menus.
        if key.modifiers.platform {
            return;
        }
        if !self.composition.is_empty() || key.is_ime_in_progress() {
            return;
        }
        if let Some(input) = key_event(event) {
            self.send(input, cx);
            cx.stop_propagation();
        }
    }

    fn coordinates(&self, position: Point<Pixels>) -> Option<(u16, u16)> {
        if !self.bounds.contains(&position) {
            return None;
        }
        Some((
            (f32::from(position.x - self.bounds.left()) / f32::from(self.cell.width)).floor()
                as u16,
            (f32::from(position.y - self.bounds.top()) / f32::from(self.cell.height)).floor()
                as u16,
        ))
    }

    fn mouse(
        &mut self,
        position: Point<Pixels>,
        kind: ClientMouseKind,
        modifiers: Modifiers,
        lines: u16,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.link_mouse(position, kind, modifiers, window, cx) {
            return;
        }
        if self.selecting {
            if !self.focus_handle.is_focused(window)
                || self.selection_token.is_none()
                || self.selection_token != self.connection.read(cx).input_token()
            {
                self.selection = None;
                self.selection_token = None;
                self.selecting = false;
                self.selection_dragged = false;
                cx.notify();
                return;
            }
            if matches!(
                kind,
                ClientMouseKind::Drag(ClientMouseButton::Left) | ClientMouseKind::Up(_)
            ) {
                let left_release = matches!(kind, ClientMouseKind::Up(ClientMouseButton::Left));
                if matches!(kind, ClientMouseKind::Drag(ClientMouseButton::Left)) || left_release {
                    // Keep the original pane even when the drag leaves the terminal bounds.
                    let rect = self.selection_rect;
                    let col = (f32::from(position.x - self.bounds.left())
                        / f32::from(self.cell.width))
                    .floor() as u16;
                    let row = (f32::from(position.y - self.bounds.top())
                        / f32::from(self.cell.height))
                    .floor() as u16;
                    if let Some((start, end)) = &mut self.selection {
                        *end = (
                            col.clamp(rect.x, rect.x.saturating_add(rect.width).saturating_sub(1)),
                            row.clamp(rect.y, rect.y.saturating_add(rect.height).saturating_sub(1)),
                        );
                        self.selection_dragged |= *start != *end
                            || matches!(kind, ClientMouseKind::Drag(ClientMouseButton::Left));
                    }
                }
                if matches!(kind, ClientMouseKind::Up(_)) {
                    self.selecting = false;
                    let dragged = std::mem::take(&mut self.selection_dragged);
                    if left_release && dragged {
                        self.copy(window, cx);
                    }
                }
                cx.notify();
                return;
            }
        }
        let Some((col, row)) = self.coordinates(position) else {
            return;
        };
        let connection = self.connection.read(cx);
        let Some(surface) = connection.surface.clone() else {
            return;
        };
        let focused_target = connection.input_target();
        let scrolling = matches!(
            kind,
            ClientMouseKind::ScrollUp
                | ClientMouseKind::ScrollDown
                | ClientMouseKind::ScrollLeft
                | ClientMouseKind::ScrollRight
        );
        let Some(MouseTarget {
            target,
            rect,
            pixel_width,
            pixel_height,
            reporting,
            stepwise,
        }) = mouse_target(&surface, col, row)
        else {
            return;
        };
        if !scrolling && focused_target.as_ref() != Some(&target) {
            if matches!(kind, ClientMouseKind::Down(_))
                && let InputTarget::Pane(pane_id) = &target
            {
                let result = self.connection.update(cx, |connection, cx| {
                    let result = connection.focus_pane(pane_id);
                    cx.notify();
                    result
                });
                self.record(result, cx);
            }
            return;
        }
        if (!reporting || modifiers.shift) && !scrolling {
            match kind {
                ClientMouseKind::Down(ClientMouseButton::Left) => {
                    self.selection = Some(((col, row), (col, row)));
                    self.selection_rect = rect;
                    self.selection_token = connection.input_token();
                    self.selecting = self.selection_token.is_some();
                    self.selection_dragged = false;
                }
                _ => return,
            }
            cx.notify();
            return;
        }
        let column = col - rect.x;
        let row = row - rect.y;
        let event = ClientPaneInputEvent::Mouse {
            kind,
            position: ClientMousePosition::Pixels {
                column,
                row,
                x: f32::from(position.x - self.bounds.left() - self.cell.width * f32::from(rect.x))
                    .max(0.0) as u32,
                y: f32::from(position.y - self.bounds.top() - self.cell.height * f32::from(rect.y))
                    .max(0.0) as u32,
            },
            geometry: Some(ClientMouseGeometry {
                cols: rect.width,
                rows: rect.height,
                width_px: if pixel_width > 0 {
                    pixel_width
                } else {
                    (f32::from(self.cell.width) * f32::from(rect.width)) as u32
                },
                height_px: if pixel_height > 0 {
                    pixel_height
                } else {
                    (f32::from(self.cell.height) * f32::from(rect.height)) as u32
                },
            }),
            modifiers: modifier_bits(modifiers),
            lines,
        };
        if scrolling {
            self.send_to(Some(&target), wheel_events(event, stepwise), cx);
            if self.error.is_some() {
                self.scroll.reset();
                self.scroll_target = None;
            }
        } else {
            self.send(event, cx);
        }
    }

    fn scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_links();
        let target = self.coordinates(event.position).and_then(|(col, row)| {
            let connection = self.connection.read(cx);
            let hit = mouse_target(connection.surface.as_ref()?, col, row)?;
            // A transient projection or fence gap must not discard motion for the
            // same destination; the token still gates the actual send.
            connection
                .visible_input_token(&hit.target)
                .map(|_| hit.target)
        });
        let Some(target) = target else {
            return;
        };
        // Resolve the pane before accumulating, even when this event is less than a row.
        if self.scroll_target.as_ref() != Some(&target) {
            self.scroll.reset();
            self.scroll_target = Some(target);
        }
        let lines = self
            .scroll
            .take_lines(event.delta, f32::from(self.cell.height));
        if lines != 0 {
            let kind = if lines > 0 {
                ClientMouseKind::ScrollUp
            } else {
                ClientMouseKind::ScrollDown
            };
            self.mouse(
                event.position,
                kind,
                event.modifiers,
                lines.unsigned_abs() as u16,
                window,
                cx,
            );
        }
    }

    fn select_all(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window)
            || !self.composition.is_empty()
            || self.link_menu_open()
            || self.sidebar_menu.is_some()
            || self.workspace_menu.is_some()
            || self.tab_menu.is_some()
        {
            return;
        }
        let connection = self.connection.read(cx);
        let Some(token) = connection.input_token() else {
            return;
        };
        let Some(surface) = &connection.surface else {
            return;
        };
        let rect = if surface.popup.is_some() {
            herdr_render::popup_rect(surface)
        } else {
            surface
                .panes
                .iter()
                .find(|pane| pane.focused)
                .map(|pane| pane.inner_rect)
        };
        let Some(rect) = rect.filter(|rect| rect.width > 0 && rect.height > 0) else {
            return;
        };
        self.selection_rect = rect;
        self.selection_token = Some(token);
        self.selection = Some((
            (rect.x, rect.y),
            (rect.x + rect.width - 1, rect.y + rect.height - 1),
        ));
        self.selecting = false;
        self.selection_dragged = false;
        self.cancel_links();
        cx.notify();
    }

    fn copy(&self, window: &Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window)
            || !self.composition.is_empty()
            || self.link_menu_open()
            || self.sidebar_menu.is_some()
            || self.workspace_menu.is_some()
            || self.tab_menu.is_some()
        {
            return;
        }
        let Some((start, end)) = self.selection else {
            return;
        };
        let connection = self.connection.read(cx);
        if self.selection_token.is_none() || self.selection_token != connection.input_token() {
            return;
        }
        let Some(surface) = &connection.surface else {
            return;
        };
        let text = if let Some(popup) = &surface.popup {
            let rect = self.selection_rect;
            let local =
                |(col, row): (u16, u16)| (col.saturating_sub(rect.x), row.saturating_sub(rect.y));
            herdr_render::selected_text(
                &popup.frame,
                SurfaceRect {
                    x: 0,
                    y: 0,
                    width: rect.width,
                    height: rect.height,
                },
                local(start),
                local(end),
            )
        } else {
            herdr_render::selected_text(&surface.frame, self.selection_rect, start, end)
        };
        if !text.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            toast::show("已复制", window, cx);
        }
    }

    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        // Element listeners only run while hovered; selection must also finish outside the pane.
        let on_move = window.listener_for(
            &cx.entity(),
            |view, event: &gpui::MouseMoveEvent, window, cx| {
                view.link_move(event, window, cx);
                if let Some(drag) = view.sidebar_drag {
                    if event.pressed_button != Some(MouseButton::Left) {
                        view.sidebar_drag = None;
                        view.save_layout(cx);
                    } else {
                        match drag {
                            SidebarDrag::Width => {
                                view.sidebar_width = window_chrome::sidebar_width(
                                    f32::from(event.position.x)
                                        + window_chrome::SIDEBAR_HANDLE / 2.0,
                                    f32::from(window.viewport_size().width),
                                    window.scale_factor(),
                                )
                            }
                            SidebarDrag::Sections => {
                                let available = (f32::from(window.viewport_size().height)
                                    - SHELL_INSET
                                    - TOOLBAR_HEIGHT
                                    - window_chrome::SECTION_HANDLE)
                                    .max(1.0);
                                let desired = f32::from(event.position.y)
                                    - SHELL_INSET
                                    - TOOLBAR_HEIGHT
                                    - window_chrome::SECTION_HANDLE / 2.0;
                                view.sidebar_ratio =
                                    window_chrome::section_height(desired / available, available)
                                        / available;
                            }
                        }
                    }
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if view.selecting {
                    if event.pressed_button == Some(MouseButton::Left) {
                        view.mouse(
                            event.position,
                            ClientMouseKind::Drag(ClientMouseButton::Left),
                            event.modifiers,
                            1,
                            window,
                            cx,
                        );
                    } else {
                        view.selecting = false;
                        view.selection_dragged = false;
                    }
                    cx.stop_propagation();
                }
            },
        );
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture {
                on_move(event, window, cx);
            }
        });
        let on_up = window.listener_for(
            &cx.entity(),
            |view, event: &gpui::MouseUpEvent, window, cx| {
                if view.sidebar_drag.take().is_some() {
                    view.save_layout(cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                if view.selecting || view.links.press.is_some() {
                    if let Some(button) = mouse_button(event.button) {
                        view.mouse(
                            event.position,
                            ClientMouseKind::Up(button),
                            event.modifiers,
                            1,
                            window,
                            cx,
                        );
                    }
                    view.selecting = false;
                    view.selection_dragged = false;
                    cx.stop_propagation();
                }
            },
        );
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture {
                on_up(event, window, cx);
            }
        });
        let started = self.trace.then(std::time::Instant::now);
        if self.bounds != bounds {
            self.cancel_links();
        }
        self.bounds = bounds;
        let font_id = window.text_system().resolve_font(&self.font);
        let width = window
            .text_system()
            .advance(font_id, px(self.font_size), 'M')
            .map(|v| v.width)
            .unwrap_or(px(self.font_size * 0.6));
        self.cell = size(width.max(px(1.0)), px(self.font_size * 1.35));
        let cols = (f32::from(bounds.size.width) / f32::from(self.cell.width))
            .floor()
            .clamp(1.0, 4096.0) as u16;
        let rows = (f32::from(bounds.size.height) / f32::from(self.cell.height))
            .floor()
            .clamp(1.0, 4096.0) as u16;
        let result = self.connection.update(cx, |connection, _| {
            connection.resize(
                ClientSurfaceSize { cols, rows },
                (
                    f32::from(self.cell.width).ceil() as u32,
                    f32::from(self.cell.height).ceil() as u32,
                ),
            )
        });
        if let Err(error) = result {
            self.error = Some(error);
        }
        let mut painter = CellPainter {
            font: self.font.clone(),
            font_size: px(self.font_size),
            cell: self.cell,
            foreground: self.palette.text,
            background: self.palette.terminal,
            background_alpha: if self.palette.glass { 0.0 } else { 1.0 },
        };
        if !self.palette.glass {
            window.paint_quad(fill(bounds, rgb(self.palette.terminal)));
        }
        self.cursor_bounds = Bounds::new(bounds.origin, self.cell);
        if let Some(surface) = self.connection.read(cx).display_surface.clone() {
            if let Some((sent, revision)) = self.input_started
                && surface.surface_revision > revision
            {
                eprintln!(
                    "Herdr trace input_surface_us={}",
                    sent.elapsed().as_micros()
                );
                self.input_started = None;
            }
            let (cursor, offset) = if let Some(popup) = &surface.popup {
                let rect = herdr_render::popup_rect(&surface);
                (
                    popup.frame.cursor.as_ref(),
                    rect.map_or((0, 0), |r| (r.x, r.y)),
                )
            } else {
                (surface.frame.cursor.as_ref(), (0, 0))
            };
            if let Some(cursor) = cursor {
                self.cursor_bounds = Bounds::new(
                    bounds.origin
                        + point(
                            self.cell.width * f32::from(cursor.x.saturating_add(offset.0)),
                            self.cell.height * f32::from(cursor.y.saturating_add(offset.1)),
                        ),
                    self.cell,
                );
            }
            let focused =
                self.focus_handle.is_focused(window) && self.connection.read(cx).input_ready();
            painter.frame(
                &surface.frame,
                bounds.origin,
                shows_terminal_cursor(
                    surface.popup.is_none() && focused,
                    !self.composition.is_empty(),
                ),
                &mut self.glyph_cache,
                window,
                cx,
            );
            if let Some(popup) = &surface.popup
                && let Some(rect) = herdr_render::popup_rect(&surface)
            {
                let origin = bounds.origin
                    + point(
                        self.cell.width * f32::from(rect.x),
                        self.cell.height * f32::from(rect.y),
                    );
                painter.background_alpha = 1.0;
                painter.frame(
                    &popup.frame,
                    origin,
                    shows_terminal_cursor(focused, !self.composition.is_empty()),
                    &mut self.glyph_cache,
                    window,
                    cx,
                );
            }
            if let Some((a, b)) = self.selection {
                let mut highlight: gpui::Hsla = rgb(self.palette.accent).into();
                highlight.a = 0.25;
                for (row, columns) in herdr_render::selection_rows(self.selection_rect, a, b) {
                    for col in columns {
                        window.paint_quad(fill(
                            Bounds::new(
                                bounds.origin
                                    + point(
                                        self.cell.width * f32::from(col),
                                        self.cell.height * f32::from(row),
                                    ),
                                self.cell,
                            ),
                            highlight,
                        ));
                    }
                }
            }
        }
        self.paint_link(window);
        if !self.composition.is_empty() {
            let origin = self.cursor_bounds().origin;
            let run = TextRun {
                len: self.composition.len(),
                font: self.font.clone(),
                color: rgb(self.palette.text).into(),
                background_color: Some(rgb(self.palette.panel).into()),
                underline: Some(gpui::UnderlineStyle {
                    color: None,
                    thickness: px(1.0),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = window.text_system().shape_line(
                self.composition.clone().into(),
                px(self.font_size),
                &[run],
                None,
            );
            if let Err(error) =
                line.paint(origin, self.cell.height, TextAlign::Left, None, window, cx)
            {
                self.error = Some(error.to_string());
            }
        }
        if let Some(started) = started {
            let connection = self.connection.read(cx);
            eprintln!(
                "Herdr trace paint_us={} frame={} ready={}",
                started.elapsed().as_micros(),
                connection.display_surface.is_some(),
                connection.input_ready()
            );
        }
    }

    fn cursor_bounds(&self) -> Bounds<Pixels> {
        // Updated in paint from the frame; the candidate panel remains anchored
        // to the terminal even during a pending resize or reconnect.
        self.cursor_bounds
    }
}

impl Render for HerdrWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shared = self.owner.read(cx);
        let palette = shared.appearance.palette(TerminalApp::system_dark(window));
        if self.palette != palette {
            self.palette = palette;
            self.apply_material(window, cx);
        }
        self.update_workspace_dialog(window, cx);
        let reduced_motion = crate::window_style::reduced_motion();
        self.indicator_elapsed = if reduced_motion {
            0
        } else {
            self.indicator_started.elapsed().as_millis()
        };
        let animate = !reduced_motion && window.is_window_active() && {
            let connection = self.connection.read(cx);
            connection.connected()
                && connection.snapshot.as_ref().is_some_and(|snapshot| {
                    let sidebar = window_chrome::show_sidebar(
                        self.sidebar_visible,
                        f32::from(window.viewport_size().width),
                    );
                    (sidebar
                        && (snapshot
                            .workspaces
                            .iter()
                            .any(|space| indicators::moving(space.agent_status))
                            || snapshot.agents.iter().any(|agent| {
                                indicators::moving(agent.agent_status)
                                    && (snapshot.agent_view_label.is_none()
                                        || snapshot.agent_order.contains(&agent.pane_id))
                            })))
                        || snapshot.tabs.iter().any(|tab| {
                            Some(&tab.workspace_id) == snapshot.focused_workspace_id.as_ref()
                                && indicators::moving(tab.agent_status)
                        })
                })
        };
        if animate && self.indicator_tick.is_none() {
            self.indicator_tick = Some(cx.spawn(async move |view, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(70))
                    .await;
                if let Err(error) = view.update(cx, |view, cx| {
                    view.indicator_tick = None;
                    cx.notify();
                }) {
                    eprintln!("Update Herdr indicators: {error}");
                }
            }));
        } else if !animate {
            self.indicator_tick = None;
        }
        let entity = cx.entity();
        let p = self.palette;
        let viewport = window.viewport_size();
        let show_sidebar =
            window_chrome::show_sidebar(self.sidebar_visible, f32::from(viewport.width));
        let sidebar_width = window_chrome::sidebar_width(
            self.sidebar_width,
            f32::from(viewport.width),
            window.scale_factor(),
        );
        if !show_sidebar && self.sidebar_drag.take().is_some() {
            self.save_layout(cx);
        }
        let shell_left = if show_sidebar {
            sidebar_width
        } else {
            SHELL_INSET
        };
        let notice = self.error.clone().or_else(|| {
            let connection = self.connection.read(cx);
            connection
                .mutation_error
                .clone()
                .or_else(|| (!connection.connected()).then(|| connection.status.clone()))
        });
        let workspace_menu = self.render_workspace_menu(window, cx);
        let sidebar_menu = self.render_sidebar_menu(window, cx);
        let tab_menu = self.render_tab_menu(window, cx);
        let link_menu = self.render_link_menu(window, cx);
        let link_tooltip = self.link_tooltip();
        let background = shell_surface::background(
            &mut self.shell_corners,
            p,
            shell_left,
            false,
            shell_surface::Surround::Theme,
            window,
        );
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(background)
            .font_family(".SystemUIFont")
            .text_color(rgb(p.text))
            .tab_group()
            .tab_stop(false)
            .track_focus(&self.focus_handle)
            .key_context("Terminal HerdrTerminal")
            .on_modifiers_changed(cx.listener(
                |view, event: &gpui::ModifiersChangedEvent, window, cx| {
                    if !gpui_ghostty_terminal::view::links::command_link(event.modifiers)
                        && let Some(press) = &mut view.links.press
                    {
                        press.cancel();
                    }
                    view.link_hover(window.mouse_position(), event.modifiers, window, cx);
                },
            ))
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|view, _: &ui::FocusControls, window, cx| {
                view.focus_handle.focus(window, cx);
                window.focus_next(cx);
                cx.stop_propagation();
            }))
            .on_action(
                cx.listener(|view, _: &crate::IncreaseFont, _, cx| view.adjust_font(1.0, cx)),
            )
            .on_action(
                cx.listener(|view, _: &crate::DecreaseFont, _, cx| view.adjust_font(-1.0, cx)),
            )
            .on_action(cx.listener(|view, _: &crate::ResetFont, _, cx| {
                view.adjust_font(crate::typography::DEFAULT_FONT_SIZE - view.font_size, cx)
            }))
            .on_action(cx.listener(|view, _: &crate::ToggleSidebar, _, cx| {
                if !view.link_menu_open() {
                    view.cancel_links();
                    view.toggle_sidebar(cx);
                }
            }))
            .on_action(cx.listener(|view, _: &crate::NewSession, _, cx| view.new_tab(cx)))
            .on_action(cx.listener(|view, _: &crate::NextSession, _, cx| view.cycle_tab(true, cx)))
            .on_action(
                cx.listener(|view, _: &crate::PreviousSession, _, cx| view.cycle_tab(false, cx)),
            )
            .on_action(cx.listener(|view, _: &crate::NextPane, _, cx| view.cycle_pane(true, cx)))
            .on_action(
                cx.listener(|view, _: &crate::PreviousPane, _, cx| view.cycle_pane(false, cx)),
            )
            .on_action(cx.listener(|view, _: &crate::NextAgent, _, cx| view.cycle_agent(true, cx)))
            .on_action(
                cx.listener(|view, _: &crate::PreviousAgent, _, cx| view.cycle_agent(false, cx)),
            )
            .on_action(cx.listener(|view, action: &crate::SelectTab, _, cx| {
                view.select_tab(action.index, cx)
            }))
            .on_action(
                cx.listener(|view, _: &crate::SplitRight, _, cx| view.split_pane("right", cx)),
            )
            .on_action(cx.listener(|view, _: &crate::SplitDown, _, cx| view.split_pane("down", cx)))
            .on_action(
                cx.listener(|view, _: &crate::TogglePaneZoom, _, cx| view.toggle_pane_zoom(cx)),
            )
            .on_action(cx.listener(|view, _: &crate::ReturnLocal, _, cx| {
                if view.link_menu_open() || !view.composition.is_empty() {
                    return;
                }
                view.cancel_links();
                TerminalApp::show_main(view.owner.clone(), cx);
            }))
            .on_action(cx.listener(|view, _: &crate::ToggleSettings, window, cx| {
                if view.link_menu_open() || !view.composition.is_empty() {
                    return;
                }
                view.cancel_links();
                view.owner
                    .update(cx, |root, cx| root.open_settings(window, cx));
            }))
            .on_action(cx.listener(|view, _: &crate::CloseSession, _, cx| view.close_pane(cx)))
            .on_action(cx.listener(|view, _: &crate::CloseTab, _, cx| view.close_tab(cx)))
            .on_action(cx.listener(|view, _: &crate::CustomizeTab, window, cx| {
                view.rename_focused_tab(window, cx)
            }))
            .on_action(cx.listener(
                |view, _: &gpui_ghostty_terminal::view::SelectAll, window, cx| {
                    view.select_all(window, cx);
                    cx.stop_propagation();
                },
            ))
            .on_action(
                cx.listener(|view, _: &gpui_ghostty_terminal::view::Paste, window, cx| {
                    if view.focus_handle.is_focused(window)
                        && !view.link_menu_open()
                        && view.composition.is_empty()
                        && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                    {
                        view.send(ClientPaneInputEvent::Paste(text), cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|view, _: &gpui_ghostty_terminal::view::Copy, window, cx| {
                    if view.focus_handle.is_focused(window) {
                        view.copy(window, cx);
                    }
                    cx.stop_propagation();
                }),
            )
            .child(div().h(px(SHELL_INSET)).flex_none())
            .child(self.chrome(show_sidebar, sidebar_width, window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .when(show_sidebar, |body| {
                        body.child(self.sidebar(
                            sidebar_width,
                            f32::from(viewport.height) - SHELL_INSET - TOOLBAR_HEIGHT,
                            cx,
                        ))
                        .child(self.sidebar_resize_handle(cx))
                    })
                    .child(
                        div()
                            .id("herdr-terminal")
                            .when(link_tooltip.is_some(), |terminal| terminal.cursor_pointer())
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .when(!show_sidebar, |terminal| terminal.ml(px(SHELL_INSET)))
                            .mr(px(SHELL_INSET))
                            .mb(px(SHELL_INSET))
                            .rounded_b(px(SHELL_RADIUS))
                            .overflow_hidden()
                            .p(px(4.0))
                            .on_any_mouse_down(cx.listener(
                                |view, event: &gpui::MouseDownEvent, window, cx| {
                                    view.focus_handle.focus(window, cx);
                                    if event.first_mouse {
                                        return;
                                    }
                                    if let Some(button) = mouse_button(event.button) {
                                        view.mouse(
                                            event.position,
                                            ClientMouseKind::Down(button),
                                            event.modifiers,
                                            1,
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            ))
                            .on_mouse_move(cx.listener(
                                |view, event: &gpui::MouseMoveEvent, window, cx| {
                                    let kind = event
                                        .pressed_button
                                        .and_then(mouse_button)
                                        .map(ClientMouseKind::Drag)
                                        .unwrap_or(ClientMouseKind::Moved);
                                    view.mouse(
                                        event.position,
                                        kind,
                                        event.modifiers,
                                        1,
                                        window,
                                        cx,
                                    );
                                },
                            ))
                            .map(|mut terminal| {
                                terminal.interactivity().on_any_mouse_up(cx.listener(
                                    |view, event: &gpui::MouseUpEvent, window, cx| {
                                        if let Some(button) = mouse_button(event.button) {
                                            view.mouse(
                                                event.position,
                                                ClientMouseKind::Up(button),
                                                event.modifiers,
                                                1,
                                                window,
                                                cx,
                                            );
                                        }
                                    },
                                ));
                                terminal
                            })
                            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
                            .child(
                                canvas(
                                    |_, _, _| (),
                                    move |bounds, _, window, cx| {
                                        let focus = entity.read(cx).focus_handle.clone();
                                        window.handle_input(
                                            &focus,
                                            RepeatInputHandler(ElementInputHandler::new(
                                                bounds,
                                                entity.clone(),
                                            )),
                                            cx,
                                        );
                                        entity
                                            .update(cx, |view, cx| view.paint(bounds, window, cx));
                                    },
                                )
                                .size_full(),
                            ),
                    ),
            )
            .when_some(notice, |root, notice| {
                root.child(
                    div()
                        .absolute()
                        .bottom(px(14.0))
                        .right(px(16.0))
                        .max_w(px((f32::from(viewport.width) - 32.0).min(480.0)))
                        .p_2()
                        .rounded(px(6.0))
                        .bg(rgb(p.panel))
                        .text_size(px(11.0))
                        .child(self.language.dynamic_text(&notice)),
                )
            })
            .child(
                toast::layer(&self.toast, p, cx)
                    .top(px(TOOLBAR_CENTER - 12.0))
                    .left(px(shell_left)),
            )
            .children(workspace_menu)
            .children(sidebar_menu)
            .children(tab_menu)
            .children(link_menu)
            .children(link_tooltip)
    }
}

fn modifier_bits(m: Modifiers) -> u8 {
    u8::from(m.shift)
        | (u8::from(m.control) << 1)
        | (u8::from(m.alt) << 2)
        | (u8::from(m.platform) << 3)
}
struct MouseTarget {
    target: InputTarget,
    rect: SurfaceRect,
    pixel_width: u32,
    pixel_height: u32,
    reporting: bool,
    stepwise: bool,
}

fn mouse_target(
    surface: &herdr_protocol::PaneSurfaceFrame,
    col: u16,
    row: u16,
) -> Option<MouseTarget> {
    if let Some(popup) = &surface.popup {
        let rect = herdr_render::popup_rect(surface)
            .filter(|rect| herdr_render::contains(*rect, col, row))?;
        Some(MouseTarget {
            target: InputTarget::Popup(popup.terminal_id.clone()),
            rect,
            pixel_width: popup.pixel_width,
            pixel_height: popup.pixel_height,
            reporting: popup.mouse_reporting,
            stepwise: popup.mouse_reporting,
        })
    } else {
        let pane = surface
            .panes
            .iter()
            .find(|pane| herdr_render::contains(pane.inner_rect, col, row))?;
        Some(MouseTarget {
            target: InputTarget::Pane(pane.pane_id.clone()),
            rect: pane.inner_rect,
            pixel_width: pane.pixel_width,
            pixel_height: pane.pixel_height,
            reporting: pane.mouse_reporting,
            stepwise: pane.mouse_reporting || pane.alternate_screen_active,
        })
    }
}

fn wheel_events(mut event: ClientPaneInputEvent, stepwise: bool) -> Vec<ClientPaneInputEvent> {
    // Scrollback consumes the row count. Mouse reports and alternate-screen key
    // translation consume one step per event, so batch those steps in one message.
    let count = if stepwise && let ClientPaneInputEvent::Mouse { lines, .. } = &mut event {
        let count = usize::from(*lines);
        *lines = 1;
        count
    } else {
        1
    };
    vec![event; count]
}

fn mouse_button(button: MouseButton) -> Option<ClientMouseButton> {
    match button {
        MouseButton::Left => Some(ClientMouseButton::Left),
        MouseButton::Right => Some(ClientMouseButton::Right),
        MouseButton::Middle => Some(ClientMouseButton::Middle),
        _ => None,
    }
}

fn key_event(event: &KeyDownEvent) -> Option<ClientPaneInputEvent> {
    let key = &event.keystroke;
    let code = match key.key.as_str() {
        "enter" | "return" | "kp_enter" => ClientKeyCode::Enter,
        "escape" => ClientKeyCode::Esc,
        "backspace" => ClientKeyCode::Backspace,
        "delete" => ClientKeyCode::Delete,
        "left" => ClientKeyCode::Left,
        "right" => ClientKeyCode::Right,
        "up" => ClientKeyCode::Up,
        "down" => ClientKeyCode::Down,
        "home" => ClientKeyCode::Home,
        "end" => ClientKeyCode::End,
        "pageup" => ClientKeyCode::PageUp,
        "pagedown" => ClientKeyCode::PageDown,
        "tab" if key.modifiers.shift => ClientKeyCode::BackTab,
        "tab" => ClientKeyCode::Tab,
        "insert" => ClientKeyCode::Insert,
        name if name.starts_with('f')
            && name[1..].parse::<u8>().is_ok_and(|n| (1..=35).contains(&n)) =>
        {
            ClientKeyCode::F(name[1..].parse().ok()?)
        }
        "space" if key.modifiers.control || key.modifiers.alt => ClientKeyCode::Char(' '),
        text if (key.modifiers.control || key.modifiers.alt) && text.chars().count() == 1 => {
            ClientKeyCode::Char(text.chars().next()?)
        }
        _ => return None,
    };
    Some(ClientPaneInputEvent::Key {
        code,
        modifiers: modifier_bits(key.modifiers),
        kind: if event.is_held {
            ClientKeyKind::Repeat
        } else {
            ClientKeyKind::Press
        },
        repeat_count: 1,
        shifted_codepoint: None,
        generated_text: None,
        tracks_release: false,
        physical_key_id: None,
        windows_record: None,
    })
}

impl EntityInputHandler for HerdrWindow {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let utf16: Vec<_> = self.composition.encode_utf16().collect();
        let range = range.start.min(utf16.len())..range.end.min(utf16.len());
        let text = String::from_utf16(utf16.get(range.clone())?).ok()?;
        *adjusted = Some(range);
        Some(text)
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.composition_selection.clone(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.composition.is_empty()).then(|| 0..self.composition.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.composition.clear();
        self.composition_token = None;
        self.composition_cancelled = false;
        self.composition_selection = 0..0;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if std::mem::take(&mut self.composition_cancelled) {
            self.composition_token = None;
            self.composition.clear();
            self.composition_selection = 0..0;
            self.error = Some("Input cancelled because the selected terminal changed".into());
            cx.notify();
            return;
        }
        let token = self.composition_token.take();
        self.composition.clear();
        self.composition_selection = 0..0;
        cx.notify();
        if let Some(token) = token {
            self.selection = None;
            self.selecting = false;
            self.selection_dragged = false;
            self.selection_token = None;
            let result = self.connection.update(cx, |connection, cx| {
                let result = connection.send_composition_to(&token, text.into());
                cx.notify();
                result
            });
            self.record(result, cx);
        } else {
            self.send(ClientPaneInputEvent::TextCommit(text.into()), cx);
        }
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if text.is_empty() {
            self.unmark_text(window, cx);
            return;
        }
        // After a real destination change, discard the rest of this IME session
        // rather than attaching its next update to a different terminal.
        if self.composition_cancelled {
            return;
        }
        if self.composition_token.is_none() {
            self.composition_token = self.connection.read(cx).composition_token();
            if self.composition_token.is_none() {
                self.composition_cancelled = true;
                return;
            }
        }
        self.cancel_links();
        self.composition = text.into();
        let end = text.encode_utf16().count();
        self.composition_selection = selected.unwrap_or(end..end);
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(self.cursor_bounds())
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
    fn accepts_text_input(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.focus_handle.is_focused(window) && self.connection.read(cx).input_ready()
    }
}

struct RepeatInputHandler(ElementInputHandler<HerdrWindow>);
impl InputHandler for RepeatInputHandler {
    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }
    fn selected_text_range(
        &mut self,
        disabled: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        self.0.selected_text_range(disabled, window, cx)
    }
    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.0.marked_text_range(window, cx)
    }
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.0.text_for_range(range, adjusted, window, cx)
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0.replace_text_in_range(range, text, window, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0
            .replace_and_mark_text_in_range(range, text, selected, window, cx);
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.0.unmark_text(window, cx);
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        self.0.bounds_for_range(range, window, cx)
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        self.0.character_index_for_point(point, window, cx)
    }
    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.0.accepts_text_input(window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(key: &str, held: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).unwrap(),
            is_held: held,
            prefer_character_input: false,
        }
    }

    fn wheel_surface() -> herdr_protocol::PaneSurfaceFrame {
        use herdr_protocol::{FrameData, PaneSurfaceFrame, PaneSurfacePane};
        let pane = |id: &str, x, focused| {
            let rect = SurfaceRect {
                x,
                y: 0,
                width: 40,
                height: 24,
            };
            PaneSurfacePane {
                pane_id: id.into(),
                content_revision: 1,
                rect,
                inner_rect: rect,
                scrollbar_rect: None,
                scroll: None,
                focused,
                mouse_reporting: false,
                sgr_pixel_mouse: false,
                alternate_screen_active: false,
                pixel_width: 400,
                pixel_height: 480,
            }
        };
        PaneSurfaceFrame {
            boot_id: "test".into(),
            projection_revision: 1,
            surface_revision: 1,
            frame: FrameData {
                width: 80,
                height: 24,
                cells: vec![],
                cursor: None,
                hyperlinks: vec![],
                graphics: vec![],
            },
            panes: vec![pane("left", 0, true), pane("right", 40, false)],
            splits: vec![],
            popup: None,
            graphics: Default::default(),
        }
    }

    #[test]
    fn wheel_hit_target_uses_pointer_and_application_mode() {
        let mut surface = wheel_surface();
        let hit = mouse_target(&surface, 50, 2).unwrap();
        assert_eq!(hit.target, InputTarget::Pane("right".into()));
        assert!(!hit.stepwise);
        surface.panes[1].alternate_screen_active = true;
        assert!(mouse_target(&surface, 50, 2).unwrap().stepwise);
        surface.panes[1].alternate_screen_active = false;
        surface.panes[1].mouse_reporting = true;
        assert!(mouse_target(&surface, 50, 2).unwrap().stepwise);
        assert!(mouse_target(&surface, 80, 2).is_none());
        assert!(mouse_target(&surface, 20, 24).is_none());
    }

    #[test]
    fn wheel_popup_blocks_panes_and_uses_its_own_reporting_mode() {
        let mut surface = wheel_surface();
        surface.popup = Some(Box::new(herdr_protocol::ClientShellPopupSurface {
            terminal_id: "popup".into(),
            title: "Test".into(),
            width: None,
            height: None,
            frame: herdr_protocol::FrameData {
                width: 20,
                height: 10,
                cells: vec![],
                cursor: None,
                hyperlinks: vec![],
                graphics: vec![],
            },
            mouse_reporting: true,
            sgr_pixel_mouse: false,
            pixel_width: 200,
            pixel_height: 200,
        }));
        let rect = herdr_render::popup_rect(&surface).unwrap();
        let hit = mouse_target(&surface, rect.x, rect.y).unwrap();
        assert_eq!(hit.target, InputTarget::Popup("popup".into()));
        assert!(hit.stepwise);
        assert!(mouse_target(&surface, 0, 0).is_none());
    }

    #[test]
    fn wheel_batches_preserve_fast_and_slow_steps_in_both_directions() {
        for kind in [ClientMouseKind::ScrollUp, ClientMouseKind::ScrollDown] {
            for count in [1, 8, 24, 240] {
                let event = ClientPaneInputEvent::Mouse {
                    kind,
                    position: ClientMousePosition::Cell { column: 3, row: 4 },
                    geometry: None,
                    modifiers: 2,
                    lines: count,
                };
                assert_eq!(wheel_events(event.clone(), false), vec![event.clone()]);
                let events = wheel_events(event, true);
                assert_eq!(events.len(), usize::from(count));
                assert!(events.iter().all(|event| *event
                    == ClientPaneInputEvent::Mouse {
                        kind,
                        position: ClientMousePosition::Cell { column: 3, row: 4 },
                        geometry: None,
                        modifiers: 2,
                        lines: 1,
                    }));
            }
        }
    }

    #[test]
    fn printable_keys_are_left_to_native_text_input() {
        assert!(key_event(&event("a", false)).is_none());
        assert!(key_event(&event("shift-a", false)).is_none());
        assert!(key_event(&event("space", false)).is_none());
    }

    #[test]
    fn composition_hides_the_terminal_cursor_until_it_ends() {
        assert!(shows_terminal_cursor(true, false));
        assert!(!shows_terminal_cursor(true, true));
        assert!(!shows_terminal_cursor(false, false));
    }

    #[test]
    fn control_repeat_and_navigation_keep_semantic_identity() {
        assert!(matches!(
            key_event(&event("ctrl-c", false)),
            Some(ClientPaneInputEvent::Key {
                code: ClientKeyCode::Char('c'),
                modifiers: 2,
                kind: ClientKeyKind::Press,
                ..
            })
        ));
        assert!(matches!(
            key_event(&event("down", true)),
            Some(ClientPaneInputEvent::Key {
                code: ClientKeyCode::Down,
                kind: ClientKeyKind::Repeat,
                ..
            })
        ));
        assert!(matches!(
            key_event(&event("shift-tab", false)),
            Some(ClientPaneInputEvent::Key {
                code: ClientKeyCode::BackTab,
                modifiers: 1,
                ..
            })
        ));
        assert!(matches!(
            key_event(&event("f12", false)),
            Some(ClientPaneInputEvent::Key {
                code: ClientKeyCode::F(12),
                ..
            })
        ));
    }
}
