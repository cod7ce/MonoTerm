//! Link gestures use the same input fence as terminal input, plus the displayed frame.
use super::*;
use gpui_ghostty_terminal::view::links::{Link, LinkPress, command_link};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    link: Link,
    token: InputToken,
    revision: u64,
}

#[derive(Default)]
pub(super) struct LinkState {
    pub press: Option<LinkPress<Target>>,
    hover: Option<Target>,
    menu: Option<(Point<Pixels>, Target)>,
}

impl HerdrWindow {
    pub(super) fn link_menu_open(&self) -> bool {
        self.links.menu.is_some()
    }

    pub(super) fn cancel_links(&mut self) {
        if let Some(press) = &mut self.links.press {
            press.cancel();
        }
        self.links.hover = None;
        self.links.menu = None;
    }

    fn link_target(&self, position: Point<Pixels>, window: &Window, cx: &App) -> Option<Target> {
        if !self.focus_handle.is_focused(window)
            || !window.is_window_active()
            || !self.composition.is_empty()
            || self.composition_cancelled
            || self.sidebar_menu.is_some()
            || self.workspace_menu.is_some()
            || self.sidebar_drag.is_some()
        {
            return None;
        }
        let connection = self.connection.read(cx);
        let token = connection.input_token()?;
        let surface = connection.surface.as_ref()?;
        if connection.display_surface.as_ref()?.surface_revision != surface.surface_revision {
            return None;
        }
        let (col, row) = self.coordinates(position)?;
        let link = if let Some(popup) = &surface.popup {
            let rect = herdr_render::popup_rect(surface)?;
            if !herdr_render::contains(rect, col, row) {
                return None;
            }
            let mut link = herdr_render::link_at(
                &popup.frame,
                SurfaceRect {
                    x: 0,
                    y: 0,
                    width: rect.width,
                    height: rect.height,
                },
                col - rect.x,
                row - rect.y,
            )?;
            for (col, row) in &mut link.cells {
                *col += rect.x;
                *row += rect.y;
            }
            link
        } else {
            let pane = surface
                .panes
                .iter()
                .find(|pane| herdr_render::contains(pane.inner_rect, col, row))?;
            if token.target() != &InputTarget::Pane(pane.pane_id.clone()) {
                return None;
            }
            herdr_render::link_at(&surface.frame, pane.inner_rect, col, row)?
        };
        Some(Target {
            link,
            token,
            revision: surface.surface_revision,
        })
    }

    pub(super) fn validate_links(&mut self, cx: &App) {
        let connection = self.connection.read(cx);
        let current = |target: &Target| {
            connection.input_token().as_ref() == Some(&target.token)
                && connection
                    .surface
                    .as_ref()
                    .is_some_and(|surface| surface.surface_revision == target.revision)
        };
        if self
            .links
            .press
            .as_ref()
            .is_some_and(|press| !current(&press.target))
            || self
                .links
                .hover
                .as_ref()
                .is_some_and(|target| !current(target))
            || self
                .links
                .menu
                .as_ref()
                .is_some_and(|(_, target)| !current(target))
        {
            self.cancel_links();
        }
    }

    pub(super) fn link_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(press) = &mut self.links.press {
            press.moved((event.position.x.into(), event.position.y.into()));
            if event.pressed_button != Some(MouseButton::Left) || !command_link(event.modifiers) {
                press.cancel();
            }
        }
        self.link_hover(event.position, event.modifiers, window, cx);
    }

    pub(super) fn link_hover(
        &mut self,
        position: Point<Pixels>,
        modifiers: Modifiers,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let next = if command_link(modifiers)
            && self.links.menu.is_none()
            && !self.selecting
            && self.pane_edit.is_none()
            && self.split_drag.is_none()
        {
            self.link_target(position, window, cx)
        } else {
            None
        };
        if next != self.links.hover {
            self.links.hover = next;
            cx.notify();
        }
    }

    pub(super) fn link_mouse(
        &mut self,
        position: Point<Pixels>,
        kind: ClientMouseKind,
        modifiers: Modifiers,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(kind, ClientMouseKind::Down(_) | ClientMouseKind::Up(_)) {
            // Browser activation must drain queued navigation/disconnect events too.
            self.connection.update(cx, |connection, cx| {
                if connection.poll() {
                    cx.notify();
                }
            });
            self.validate_links(cx);
        }
        if matches!(kind, ClientMouseKind::Up(ClientMouseButton::Left))
            && let Some(press) = self.links.press.take()
        {
            let current = self.link_target(position, window, cx);
            let url = press.target.link.url.clone();
            if press.release(
                current.as_ref(),
                (position.x.into(), position.y.into()),
                command_link(modifiers),
            ) {
                cx.open_url(&url);
            }
            cx.notify();
            return true;
        }
        if matches!(kind, ClientMouseKind::Drag(ClientMouseButton::Left))
            && self.links.press.is_some()
        {
            return true;
        }
        if matches!(kind, ClientMouseKind::Down(_)) {
            self.links.press = None;
            self.links.menu = None;
            if matches!(kind, ClientMouseKind::Down(ClientMouseButton::Left))
                && command_link(modifiers)
                && let Some(target) = self.link_target(position, window, cx)
            {
                self.selecting = false;
                self.links.press = Some(LinkPress::new(
                    target,
                    (position.x.into(), position.y.into()),
                ));
                cx.notify();
                return true;
            }
            if matches!(kind, ClientMouseKind::Down(ClientMouseButton::Right))
                && let Some(target) = self.link_target(position, window, cx)
            {
                let connection = self.connection.read(cx);
                let surface = connection.surface.as_ref().unwrap();
                let reporting = surface.popup.as_ref().map_or_else(
                    || {
                        surface
                            .panes
                            .iter()
                            .any(|pane| pane.focused && pane.mouse_reporting)
                    },
                    |popup| popup.mouse_reporting,
                );
                if !reporting || modifiers.shift {
                    self.links.hover = None;
                    self.links.menu = Some((position, target));
                    cx.notify();
                    return true;
                }
            }
        }
        matches!(kind, ClientMouseKind::Up(ClientMouseButton::Right)) && self.links.menu.is_some()
    }

    pub(super) fn paint_link(&self, window: &mut Window) {
        if let Some(target) = &self.links.hover {
            for &(col, row) in &target.link.cells {
                window.paint_quad(fill(
                    Bounds::new(
                        self.bounds.origin
                            + point(
                                self.cell.width * f32::from(col),
                                self.cell.height * (f32::from(row) + 1.0) - px(1.0),
                            ),
                        size(self.cell.width, px(1.0)),
                    ),
                    rgb(self.palette.text),
                ));
            }
        }
    }

    pub(super) fn link_tooltip(&self) -> Option<gpui::Div> {
        self.links.hover.as_ref().map(|target| {
            gpui_ghostty_terminal::view::links::link_tooltip(
                target.link.url.clone(),
                rgb(self.palette.text).into(),
                rgb(self.palette.panel).into(),
                self.bounds.size.width - px(8.0),
            )
            .left(self.bounds.left() + px(4.0))
        })
    }

    pub(super) fn render_link_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (position, target) = self.links.menu.as_ref()?;
        let target = target.clone();
        let viewport = window.viewport_size();
        Some(
            div()
                .id("herdr-link-menu")
                .absolute()
                .left(position.x.min((viewport.width - px(184.0)).max(px(0.0))))
                .top(position.y.min((viewport.height - px(36.0)).max(px(0.0))))
                .w(px(184.0))
                .p_2()
                .rounded_md()
                .bg(rgb(self.palette.panel))
                .border_1()
                .border_color(rgb(self.palette.border))
                .text_size(px(12.0))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |view, _, window, cx| {
                        view.connection.update(cx, |connection, cx| {
                            if connection.poll() {
                                cx.notify();
                            }
                        });
                        view.validate_links(cx);
                        if view
                            .links
                            .menu
                            .as_ref()
                            .is_some_and(|(_, current)| current == &target)
                        {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                target.link.url.clone(),
                            ));
                            toast::show("已复制", window, cx);
                        }
                        view.links.menu = None;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                    view.links.menu = None;
                    cx.notify();
                }))
                .child(self.language.text("Copy link address"))
                .into_any_element(),
        )
    }
}
