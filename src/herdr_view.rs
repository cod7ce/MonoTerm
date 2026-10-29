//! Native interactive window for Herdr's daemon-owned terminal surface.
use crate::{
    TerminalApp,
    appearance::Palette,
    herdr_connection::{HerdrConnection, InputTarget, InputToken},
    herdr_render::{self, CellPainter, GlyphCache},
    locale::Language,
    toast, ui,
};
use gpui::{
    App, Bounds, Context, DispatchPhase, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Font, InputHandler, KeyDownEvent, Modifiers, MouseButton, Pixels, Point, Render,
    Subscription, TextAlign, TextRun, UTF16Selection, Window, WindowHandle, canvas, div, fill,
    point, prelude::*, px, rgb, rgba, size,
};
use herdr_protocol::{
    ClientKeyCode, ClientKeyKind, ClientMouseButton, ClientMouseGeometry, ClientMouseKind,
    ClientMousePosition, ClientPaneInputEvent, ClientSurfaceSize, SurfaceRect,
};
use std::ops::Range;

#[path = "herdr_chrome.rs"]
mod chrome;
#[path = "herdr_navigation.rs"]
mod navigation;
#[path = "herdr_sidebar.rs"]
mod sidebar;

pub(crate) struct HerdrWindow {
    connection: Entity<HerdrConnection>,
    owner: WindowHandle<TerminalApp>,
    focus_handle: FocusHandle,
    font: Font,
    font_size: f32,
    glyph_cache: GlyphCache,
    palette: Palette,
    toast: Entity<toast::Toast>,
    blur_radius: u32,
    language: Language,
    sidebar_visible: bool,
    sidebar_scroll: [gpui::ScrollHandle; 2],
    tab_scroll: gpui::ScrollHandle,
    history: navigation::FocusHistory,
    bounds: Bounds<Pixels>,
    cell: gpui::Size<Pixels>,
    composition: String,
    composition_selection: Range<usize>,
    composition_token: Option<InputToken>,
    composition_cancelled: bool,
    cursor_bounds: Bounds<Pixels>,
    selection: Option<((u16, u16), (u16, u16))>,
    selecting: bool,
    selection_dragged: bool,
    selection_rect: SurfaceRect,
    selection_token: Option<InputToken>,
    scroll_remainder: f32,
    error: Option<String>,
    trace: bool,
    input_started: Option<(std::time::Instant, u64)>,
    _subscriptions: Vec<Subscription>,
}

impl HerdrWindow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        connection: Entity<HerdrConnection>,
        owner: WindowHandle<TerminalApp>,
        font: Font,
        font_size: f32,
        palette: Palette,
        blur_radius: u32,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observation = cx.observe(&connection, |view, _, cx| {
            let connection = view.connection.read(cx);
            view.history.observe(
                connection.snapshot.as_ref().map(|s| s.boot_id.as_str()),
                connection
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.focused_pane_id.as_deref()),
                connection.input_ready(),
            );
            if view.composition_token.is_some()
                && view.composition_token != view.connection.read(cx).input_token()
            {
                view.composition.clear();
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
            let result = view.connection.update(cx, |connection, cx| {
                let result = connection.set_focus(window.is_window_active());
                cx.notify();
                result
            });
            view.record(result, cx);
        });
        let close_connection = connection.clone();
        window.on_window_should_close(cx, move |_, cx| {
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
            sidebar_visible: true,
            sidebar_scroll: std::array::from_fn(|_| gpui::ScrollHandle::new()),
            tab_scroll: gpui::ScrollHandle::new(),
            history: navigation::FocusHistory::default(),
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
            scroll_remainder: 0.0,
            error: None,
            trace: std::env::var_os("MONOTERM_HERDR_TRACE").is_some(),
            input_started: None,
            _subscriptions: vec![observation, activation],
        };
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
        let owner = self.owner;
        cx.defer(move |cx| {
            if let Err(error) = owner.update(cx, |root, _, cx| root.adjust_font(delta, cx)) {
                eprintln!("Herdr font: {error}");
            }
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
        token: Option<&InputToken>,
        event: ClientPaneInputEvent,
        cx: &mut Context<Self>,
    ) {
        if self.selection.take().is_some() {
            cx.notify();
        }
        self.selecting = false;
        self.selection_dragged = false;
        self.selection_token = None;
        let result = self.connection.update(cx, |connection, cx| {
            let revision = connection.revision();
            let result = match token {
                Some(token) => connection.send_input_to(token, [event]),
                None => connection.send_input([event]),
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
        self.send_to(None, event, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.connection.update(cx, |connection, cx| {
            let result = connection
                .set_surface_active(false)
                .and_then(|()| connection.set_focus(false));
            cx.notify();
            result
        });
        self.record(result, cx);
        window.remove_window();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        if !self.focus_handle.is_focused(window) {
            if key.key == "escape" && !key.modifiers.modified() {
                self.focus_handle.focus(window, cx);
            } else {
                ui::traverse_controls(event, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        if key.modifiers.platform {
            match key.key.as_str() {
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.send(ClientPaneInputEvent::Paste(text), cx);
                    }
                }
                "c" => self.copy(window, cx),
                "w" => self.close(window, cx),
                "," => {
                    let owner = self.owner;
                    cx.defer(move |cx| {
                        if let Err(error) =
                            owner.update(cx, |root, window, cx| root.open_settings(window, cx))
                        {
                            eprintln!("Herdr settings: {error}");
                        }
                    });
                }
                _ => return,
            }
            cx.stop_propagation();
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
        let target = connection.input_target();
        let (rect, pixel_width, pixel_height, reporting) = if let Some(popup) = &surface.popup {
            let Some(rect) = herdr_render::popup_rect(&surface)
                .filter(|rect| herdr_render::contains(*rect, col, row))
            else {
                return;
            };
            (
                rect,
                popup.pixel_width,
                popup.pixel_height,
                popup.mouse_reporting,
            )
        } else {
            let Some(pane) = surface
                .panes
                .iter()
                .find(|pane| herdr_render::contains(pane.inner_rect, col, row))
            else {
                return;
            };
            if target != Some(InputTarget::Pane(pane.pane_id.clone())) {
                if matches!(kind, ClientMouseKind::Down(_)) {
                    let result = self.connection.update(cx, |connection, cx| {
                        let result = connection.focus_pane(&pane.pane_id);
                        cx.notify();
                        result
                    });
                    self.record(result, cx);
                }
                return;
            }
            (
                pane.inner_rect,
                pane.pixel_width,
                pane.pixel_height,
                pane.mouse_reporting,
            )
        };
        let scrolling = matches!(
            kind,
            ClientMouseKind::ScrollUp
                | ClientMouseKind::ScrollDown
                | ClientMouseKind::ScrollLeft
                | ClientMouseKind::ScrollRight
        );
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
        self.send(
            ClientPaneInputEvent::Mouse {
                kind,
                position: ClientMousePosition::Pixels {
                    column,
                    row,
                    x: f32::from(
                        position.x - self.bounds.left() - self.cell.width * f32::from(rect.x),
                    )
                    .max(0.0) as u32,
                    y: f32::from(
                        position.y - self.bounds.top() - self.cell.height * f32::from(rect.y),
                    )
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
            },
            cx,
        );
    }

    fn copy(&self, window: &Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
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
                if view.selecting {
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
            painter.frame(
                &surface.frame,
                bounds.origin,
                surface.popup.is_none()
                    && self.focus_handle.is_focused(window)
                    && self.connection.read(cx).input_ready(),
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
                    self.focus_handle.is_focused(window) && self.connection.read(cx).input_ready(),
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
        let entity = cx.entity();
        let p = self.palette;
        let status = self
            .error
            .clone()
            .unwrap_or_else(|| self.connection.read(cx).status.clone());
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba((p.shell << 8) | (p.alpha * 255.0) as u32))
            .text_color(rgb(p.text))
            .track_focus(&self.focus_handle)
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
                view.adjust_font(15.0 - view.font_size, cx)
            }))
            .on_action(cx.listener(|view, _: &crate::ToggleSidebar, _, cx| view.toggle_sidebar(cx)))
            .on_action(cx.listener(|view, _: &crate::NewSession, _, cx| {
                let owner = view.owner;
                cx.defer(move |cx| {
                    if let Err(error) = owner.update(cx, |root, window, cx| {
                        root.new_session(window, cx);
                        window.activate_window();
                    }) {
                        eprintln!("Herdr new terminal: {error}");
                    }
                });
            }))
            .on_action(cx.listener(|view, _: &crate::ReturnLocal, _, cx| {
                let owner = view.owner;
                cx.defer(move |cx| {
                    if let Err(error) =
                        owner.update(cx, |root, window, cx| root.return_local(window, cx))
                    {
                        eprintln!("Herdr local terminals: {error}");
                    }
                });
            }))
            .on_action(cx.listener(|view, _: &crate::ToggleSettings, _, cx| {
                let owner = view.owner;
                cx.defer(move |cx| {
                    if let Err(error) =
                        owner.update(cx, |root, window, cx| root.open_settings(window, cx))
                    {
                        eprintln!("Herdr settings: {error}");
                    }
                });
            }))
            .on_action(
                cx.listener(|view, _: &crate::CloseSession, window, cx| view.close(window, cx)),
            )
            .on_action(cx.listener(|view, _: &crate::CloseTab, window, cx| view.close(window, cx)))
            .on_action(
                cx.listener(|view, _: &gpui_ghostty_terminal::view::Paste, window, cx| {
                    if view.focus_handle.is_focused(window)
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
            .child(self.chrome(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .when(self.sidebar_visible, |body| {
                        body.child(self.sidebar(window, cx))
                    })
                    .child(
                        div()
                            .id("herdr-terminal")
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .overflow_hidden()
                            .on_any_mouse_down(cx.listener(
                                |view, event: &gpui::MouseDownEvent, window, cx| {
                                    view.focus_handle.focus(window, cx);
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
                            .on_scroll_wheel(cx.listener(
                                |view, event: &gpui::ScrollWheelEvent, window, cx| {
                                    view.scroll_remainder +=
                                        f32::from(event.delta.pixel_delta(view.cell.height).y)
                                            / f32::from(view.cell.height);
                                    let lines =
                                        view.scroll_remainder.abs().floor().min(100.0) as u16;
                                    if lines > 0 {
                                        let kind = if view.scroll_remainder > 0.0 {
                                            ClientMouseKind::ScrollUp
                                        } else {
                                            ClientMouseKind::ScrollDown
                                        };
                                        view.scroll_remainder -=
                                            view.scroll_remainder.signum() * f32::from(lines);
                                        view.mouse(
                                            event.position,
                                            kind,
                                            event.modifiers,
                                            lines,
                                            window,
                                            cx,
                                        );
                                    }
                                },
                            ))
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
            .child(self.status_bar(status, cx))
            .child(
                toast::layer(&self.toast, p, cx)
                    .top(px((chrome::HEADER_HEIGHT - 24.0) / 2.0))
                    .when(self.sidebar_visible, |layer| {
                        layer.left(px(sidebar::SIDEBAR_WIDTH))
                    }),
            )
    }
}

fn modifier_bits(m: Modifiers) -> u8 {
    u8::from(m.shift)
        | (u8::from(m.control) << 1)
        | (u8::from(m.alt) << 2)
        | (u8::from(m.platform) << 3)
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
            self.error = Some("Input cancelled because the selected terminal changed".into());
            cx.notify();
            return;
        }
        let token = self
            .composition_token
            .take()
            .or_else(|| self.connection.read(cx).input_token());
        if !self.composition.is_empty() {
            self.composition.clear();
            cx.notify();
        }
        self.composition_selection = 0..0;
        if let Some(token) = token {
            self.send_to(
                Some(&token),
                ClientPaneInputEvent::TextCommit(text.into()),
                cx,
            );
        } else {
            self.error = Some("Waiting for the selected terminal".into());
            cx.notify();
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
        if self.composition_token.is_none() {
            self.composition_token = self.connection.read(cx).input_token();
        }
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

    #[test]
    fn printable_keys_are_left_to_native_text_input() {
        assert!(key_event(&event("a", false)).is_none());
        assert!(key_event(&event("shift-a", false)).is_none());
        assert!(key_event(&event("space", false)).is_none());
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
