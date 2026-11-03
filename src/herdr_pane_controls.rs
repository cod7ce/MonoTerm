//! Pane title editing and pointer-driven daemon split resizing.
use super::HerdrWindow;
use crate::{
    herdr_connection::{InputTarget, PaneTarget, SplitTarget},
    herdr_render,
    pane_editor::{EditorEvent, PaneEditor},
};
use gpui::{
    AppContext, Bounds, Context, CursorStyle, Entity, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Size, Subscription, Window,
};
use herdr_protocol::{PaneSurfaceFrame, PaneSurfaceSplitDirection};

pub(super) struct PaneEdit {
    pub editor: Entity<PaneEditor>,
    target: PaneTarget,
    pending: Option<String>,
    _subscription: Subscription,
}

pub(super) struct SplitDrag {
    target: SplitTarget,
    bounds: Bounds<Pixels>,
    cell: Size<Pixels>,
    grab: f32,
    held: bool,
    desired: Option<f32>,
    sent: Option<f32>,
    pending: Option<String>,
}

fn response_result(response: serde_json::Value) -> Result<(), String> {
    if !response["error"].is_null() {
        Err(response["error"]["message"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| response["error"].to_string()))
    } else {
        Ok(())
    }
}

fn pane_header(surface: &PaneSurfaceFrame, col: u16, row: u16) -> Option<&str> {
    if surface.popup.is_some() {
        return None;
    }
    surface
        .panes
        .iter()
        .find(|pane| herdr_render::contains(pane.rect, col, row) && row < pane.inner_rect.y)
        .map(|pane| pane.pane_id.as_str())
}

impl HerdrWindow {
    pub(super) fn rename_focused_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone());
        if let Some(id) = id {
            self.edit_pane_name(&id, window, cx);
        }
    }

    fn edit_pane_name(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.actions_available(cx) {
            return;
        }
        let connection = self.connection.read(cx);
        if connection
            .visible_input_token(&InputTarget::Pane(id.into()))
            .is_none()
        {
            return;
        }
        let Some(target) = connection.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .panes
                .iter()
                .find(|pane| pane.pane_id == id)
                .map(|pane| PaneTarget::new(snapshot, pane))
        }) else {
            return;
        };
        let editor = cx.new(|cx| {
            PaneEditor::new(
                target.label().map(str::to_owned),
                self.language.text("Pane name").into(),
                self.palette.herdr_sidebar(),
                self.language,
                window,
                cx,
            )
        });
        let subscription = cx.subscribe_in(&editor, window, |view, _, event, window, cx| {
            match event {
                EditorEvent::Cancel => {
                    view.pane_edit = None;
                    view.focus_handle.focus(window, cx);
                }
                EditorEvent::Save(title) => {
                    let Some(edit) = &view.pane_edit else {
                        return;
                    };
                    if edit.pending.is_some() {
                        return;
                    }
                    let target = edit.target.clone();
                    let result = view.connection.update(cx, |connection, cx| {
                        let result =
                            connection.rename_pane(&target, title.as_deref().unwrap_or_default());
                        cx.notify();
                        result
                    });
                    if let Some(edit) = &mut view.pane_edit {
                        match result {
                            Ok(id) => {
                                edit.pending = Some(id);
                                edit.editor
                                    .update(cx, |editor, cx| editor.set_pending(true, cx));
                            }
                            Err(error) => edit
                                .editor
                                .update(cx, |editor, cx| editor.set_error(error, cx)),
                        }
                    }
                }
            }
            cx.notify();
        });
        editor.update(cx, |editor, cx| editor.focus(window, cx));
        self.split_drag = None;
        self.selecting = false;
        self.cancel_links();
        self.pane_edit = Some(PaneEdit {
            editor,
            target,
            pending: None,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn poll_pane_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = &self.pane_edit {
            let response = edit.pending.as_ref().and_then(|id| {
                self.connection
                    .update(cx, |connection, _| connection.take_dialog_response(id))
            });
            if let Some(response) = response {
                match response.and_then(response_result) {
                    Ok(()) => {
                        self.pane_edit = None;
                        self.focus_handle.focus(window, cx);
                    }
                    Err(error) => {
                        if let Some(edit) = &mut self.pane_edit {
                            edit.pending = None;
                            edit.editor.update(cx, |editor, cx| {
                                editor.set_error(error, cx);
                                editor.focus(window, cx);
                            });
                        }
                    }
                }
            }
        }
        self.flush_split_drag(cx);
    }

    pub(super) fn pane_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.split_drag.is_some() {
            cx.stop_propagation();
            return true;
        }
        if !self.actions_available(cx) {
            return false;
        }
        let Some((col, row)) = self.coordinates(event.position) else {
            return false;
        };
        if event.button == MouseButton::Left
            && let Some(target) = self.connection.read(cx).split_target(col, row)
        {
            let local = event.position - self.bounds.origin;
            let pointer = target.pointer(f32::from(local.x), f32::from(local.y));
            let grab =
                target.edge(f32::from(self.cell.width), f32::from(self.cell.height)) - pointer;
            self.split_drag = Some(SplitDrag {
                target,
                bounds: self.bounds,
                cell: self.cell,
                grab,
                held: true,
                desired: None,
                sent: None,
                pending: None,
            });
            self.selection = None;
            self.selecting = false;
            self.cancel_links();
            cx.stop_propagation();
            cx.notify();
            return true;
        }
        let id = self
            .connection
            .read(cx)
            .surface
            .as_ref()
            .and_then(|surface| pane_header(surface, col, row).map(str::to_owned));
        let Some(id) = id else {
            return false;
        };
        if event.button == MouseButton::Right
            || (event.button == MouseButton::Left && event.click_count == 2)
        {
            self.edit_pane_name(&id, window, cx);
        } else if event.button == MouseButton::Left {
            let result = self.connection.update(cx, |connection, cx| {
                let result = connection.focus_pane(&id);
                cx.notify();
                result
            });
            self.record(result, cx);
        }
        cx.stop_propagation();
        true
    }

    pub(super) fn split_cursor(
        &self,
        position: Point<Pixels>,
        cx: &Context<Self>,
    ) -> Option<CursorStyle> {
        let direction = if let Some(drag) = self.split_drag.as_ref().filter(|drag| drag.held) {
            drag.target.split.direction
        } else {
            if !self.actions_available(cx) {
                return None;
            }
            let (col, row) = self.coordinates(position)?;
            self.connection
                .read(cx)
                .split_target(col, row)?
                .split
                .direction
        };
        Some(match direction {
            PaneSurfaceSplitDirection::Horizontal => CursorStyle::ResizeLeftRight,
            PaneSurfaceSplitDirection::Vertical => CursorStyle::ResizeUpDown,
        })
    }

    pub(super) fn split_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(drag) = self.split_drag.as_mut().filter(|drag| drag.held) else {
            return false;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.split_drag = None;
        } else {
            let local = event.position - drag.bounds.origin;
            let pointer = drag.target.pointer(f32::from(local.x), f32::from(local.y));
            drag.desired = drag.target.ratio(
                pointer + drag.grab,
                f32::from(drag.cell.width),
                f32::from(drag.cell.height),
            );
            self.flush_split_drag(cx);
        }
        cx.stop_propagation();
        cx.notify();
        true
    }

    pub(super) fn split_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self
            .split_drag
            .as_mut()
            .filter(|drag| drag.held && event.button == MouseButton::Left)
        else {
            return false;
        };
        let local = event.position - drag.bounds.origin;
        let pointer = drag.target.pointer(f32::from(local.x), f32::from(local.y));
        // A stationary click must not send a rounded ratio back to the daemon.
        if drag.desired.is_some() || drag.sent.is_some() {
            drag.desired = drag.target.ratio(
                pointer + drag.grab,
                f32::from(drag.cell.width),
                f32::from(drag.cell.height),
            );
        }
        drag.held = false;
        self.flush_split_drag(cx);
        cx.stop_propagation();
        cx.notify();
        true
    }

    fn flush_split_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.split_drag else {
            return;
        };
        let connection = self.connection.read(cx);
        if !connection.connected()
            || self.workspace_menu.is_some()
            || self.sidebar_menu.is_some()
            || self.tab_menu.is_some()
            || self.pane_edit.is_some()
            || self.sidebar_drag.is_some()
            || drag.bounds != self.bounds
            || drag.cell != self.cell
            || connection.snapshot.as_ref().is_none_or(|snapshot| {
                snapshot.boot_id != drag.target.boot_id
                    || snapshot.focused_tab_id.as_ref() != Some(&drag.target.tab_id)
            })
        {
            self.split_drag = None;
            return;
        }
        if let Some(id) = &drag.pending {
            let response = self
                .connection
                .update(cx, |connection, _| connection.take_dialog_response(id));
            let Some(response) = response else {
                return;
            };
            if let Err(error) = response.and_then(response_result) {
                self.split_drag = None;
                self.record(Err(error), cx);
                return;
            }
            drag.pending = None;
        }
        let connection = self.connection.read(cx);
        // Acknowledgement may precede the new projection. Keep only the newest
        // unsent pointer position while waiting for the ordinary input fence.
        if !connection.input_ready() {
            return;
        }
        let valid = connection
            .snapshot
            .as_ref()
            .zip(connection.surface.as_ref())
            .is_some_and(|(snapshot, surface)| drag.target.validate(snapshot, surface).is_ok());
        if !valid {
            self.split_drag = None;
            return;
        }
        let Some(ratio) = drag.desired.filter(|ratio| Some(*ratio) != drag.sent) else {
            if !drag.held {
                self.split_drag = None;
            }
            return;
        };
        let result = self.connection.update(cx, |connection, cx| {
            let result = connection.set_split_ratio(&drag.target, ratio);
            cx.notify();
            result
        });
        match result {
            Ok(id) => {
                drag.sent = Some(ratio);
                drag.pending = Some(id);
            }
            Err(error) => {
                self.split_drag = None;
                self.record(Err(error), cx);
            }
        }
    }
}
