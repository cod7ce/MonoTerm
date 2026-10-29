use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, DispatchPhase, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ShapedLine, SharedString, Style, TextAlign, TextRun, UTF16Selection,
    UnderlineStyle, Window, actions, div, fill, point, prelude::*, px, relative, rgba, size,
};
use gpui_ghostty_terminal::view::{
    Copy as TerminalCopy, Paste as TerminalPaste, SelectAll as TerminalSelectAll,
};
use unicode_segmentation::UnicodeSegmentation;

const KEY_CONTEXT: &str = "MonoTermTextInput";
const HEIGHT: f32 = 28.0;
const FONT_SIZE: f32 = 14.0;
const CURSOR_WIDTH: f32 = 1.0;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        Paste,
        Copy,
        Cut,
        ShowCharacterPalette,
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(KEY_CONTEXT)),
        KeyBinding::new("delete", Delete, Some(KEY_CONTEXT)),
        KeyBinding::new("left", Left, Some(KEY_CONTEXT)),
        KeyBinding::new("right", Right, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(KEY_CONTEXT)),
        KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
        KeyBinding::new("end", End, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-home", SelectHome, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-end", SelectEnd, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-left", Home, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-right", End, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-shift-left", SelectHome, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-shift-right", SelectEnd, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-c", Copy, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-x", Cut, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some(KEY_CONTEXT)),
    ]);
}

pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    scroll_offset: Pixels,
    is_selecting: bool,
}

impl TextInput {
    pub fn new(value: String, placeholder: String, cx: &mut Context<Self>) -> Self {
        let content: SharedString = single_line(&value).into();
        Self {
            focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            selected_range: 0..content.len(),
            content,
            placeholder: single_line(&placeholder).into(),
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll_offset: px(0.0),
            is_selecting: false,
        }
    }

    pub fn value(&self) -> &str {
        &self.content
    }

    /// Focus the input. Call explicitly with mutable `App` access; take care not
    /// to hold an entity read guard across this call.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn set_value(&mut self, value: String, cx: &mut Context<Self>) {
        self.content = single_line(&value).into();
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.scroll_offset = px(0.0);
        self.is_selecting = false;
        cx.notify();
    }

    pub fn has_marked_text(&self) -> bool {
        self.marked_range.is_some()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let anchor = if self.selection_reversed {
            self.selected_range.end
        } else {
            self.selected_range.start
        };
        self.selected_range = anchor.min(offset)..anchor.max(offset);
        self.selection_reversed = offset < anchor;
        self.marked_range = None;
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            previous_grapheme_boundary(&self.content, self.cursor_offset())
        } else {
            floor_grapheme_boundary(&self.content, self.selected_range.start)
        };
        self.move_to(offset, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            next_grapheme_boundary(&self.content, self.cursor_offset())
        } else {
            ceil_grapheme_boundary(&self.content, self.selected_range.end)
        };
        self.move_to(offset, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(
            previous_grapheme_boundary(&self.content, self.cursor_offset()),
            cx,
        );
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(
            next_grapheme_boundary(&self.content, self.cursor_offset()),
            cx,
        );
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let range = deletion_range(&self.content, self.selected_range.clone(), true);
        self.replace_range(range, "");
        cx.notify();
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let range = deletion_range(&self.content, self.selected_range.clone(), false);
        self.replace_range(range, "");
        cx.notify();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty()
            && let Some(text) = self.content.get(self.selected_range.clone())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.copy(&Copy, window, cx);
            self.replace_range(self.selected_range.clone(), "");
            cx.notify();
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        self.is_selecting = true;
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
        cx.stop_propagation();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() || !self.focus_handle.is_focused(window) {
            self.is_selecting = false;
        }
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
            cx.stop_propagation();
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds, self.last_layout.as_ref()) else {
            return self.cursor_offset();
        };
        let x = position.x - bounds.left() + self.scroll_offset;
        // Shaping may expose indices inside a cluster; only grapheme boundaries are carets.
        let index = line.closest_index_for_x(x).min(self.content.len());
        let left = floor_grapheme_boundary(&self.content, index);
        let right = ceil_grapheme_boundary(&self.content, index);
        if (line.x_for_index(right) - x).abs() < (line.x_for_index(left) - x).abs() {
            right
        } else {
            left
        }
    }

    fn replacement_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        range_utf16
            .map(|range| range_from_utf16(&self.content, range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone())
    }

    fn replace_range(&mut self, range: Range<usize>, text: &str) {
        let range = safe_byte_range(&self.content, range);
        let mut content = self.content.to_string();
        content.replace_range(range.clone(), text);
        self.content = content.into();
        let cursor = range.start + text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = range_from_utf16(&self.content, range_utf16);
        *actual_range = Some(range_to_utf16(&self.content, range.clone()));
        self.content.get(range).map(str::to_owned)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: range_to_utf16(&self.content, self.selected_range.clone()),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range
            .clone()
            .map(|range| range_to_utf16(&self.content, range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked_range.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = safe_byte_range(&self.content, self.replacement_range(range_utf16));
        self.replace_range(range, &single_line(text));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = safe_byte_range(&self.content, self.replacement_range(range_utf16));
        let text = single_line(text);
        let selection = marked_selection(&text, range.start, selected_range_utf16);
        self.replace_range(range.clone(), &text);
        self.marked_range = (!text.is_empty()).then_some(range.start..range.start + text.len());
        self.selected_range = selection;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = range_from_utf16(&self.content, range_utf16);
        let bounds = self.last_bounds.unwrap_or(bounds);
        // An IME can query before the next paint. Stale glyph indices must not be used.
        let (start, end) = if self.content.is_empty() {
            (bounds.left(), bounds.left())
        } else if let Some(line) = &self.last_layout {
            (
                bounds.left() + line.x_for_index(range.start) - self.scroll_offset,
                bounds.left() + line.x_for_index(range.end) - self.scroll_offset,
            )
        } else {
            (bounds.left(), bounds.left())
        };
        let left = start.min(end).max(bounds.left()).min(bounds.right());
        let right = (start.max(end) + px(CURSOR_WIDTH))
            .max(left)
            .min(bounds.right());
        Some(Bounds::from_corners(
            point(left, bounds.top()),
            point(right, bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        if !bounds.contains(&point) {
            return None;
        }
        if !self.content.is_empty() && self.last_layout.is_none() {
            return None;
        }
        Some(offset_to_utf16(
            &self.content,
            self.index_for_mouse_position(point),
        ))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: ShapedLine,
    origin: Point<Pixels>,
    scroll_offset: Pixels,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = px(HEIGHT).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let style = window.text_style();
        let empty = input.content.is_empty();
        let text = if empty {
            input.placeholder.clone()
        } else {
            input.content.clone()
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: if empty {
                style.color.opacity(0.45)
            } else {
                style.color
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = &input.marked_range {
            let start = marked.start.min(marked.end).min(text.len());
            let end = marked.start.max(marked.end).min(text.len());
            vec![
                TextRun {
                    len: start,
                    ..run.clone()
                },
                TextRun {
                    len: end - start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![run]
        };
        let line = window
            .text_system()
            .shape_line(text, px(FONT_SIZE), &runs, None);
        let cursor_x = if empty {
            px(0.0)
        } else {
            line.x_for_index(input.cursor_offset())
        };
        // A freshly selected long title should reveal its beginning, not its trailing
        // caret. While typing, the collapsed caret remains the scrolling reference.
        let scroll_reference = if input.selected_range.is_empty() || input.is_selecting {
            cursor_x
        } else {
            line.x_for_index(input.selected_range.start)
        };
        let width = if empty { px(0.0) } else { line.width };
        let scroll_offset = visible_scroll_offset(
            input.scroll_offset,
            scroll_reference,
            width,
            bounds.size.width,
        );
        let origin = point(bounds.left() - scroll_offset, bounds.top());
        let cursor = input.selected_range.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(origin.x + cursor_x, bounds.top()),
                    size(px(CURSOR_WIDTH), bounds.size.height),
                ),
                style.color,
            )
        });
        let selection = (!input.selected_range.is_empty()).then(|| {
            let start = line.x_for_index(input.selected_range.start);
            let end = line.x_for_index(input.selected_range.end);
            fill(
                Bounds::from_corners(
                    point(origin.x + start.min(end), bounds.top()),
                    point(origin.x + start.max(end), bounds.bottom()),
                ),
                rgba(0x528bcc48),
            )
        });
        PrepaintState {
            line,
            origin,
            scroll_offset,
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );

        // Div's mouse-move listener only runs while hovered. Capture ongoing selections
        // at the window so dragging beyond the field still extends and scrolls them.
        let on_move = window.listener_for(&self.input, TextInput::on_mouse_move);
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture {
                on_move(event, window, cx);
            }
        });
        let on_up = window.listener_for(&self.input, TextInput::on_mouse_up);
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                on_up(event, window, cx);
            }
        });

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            if let Err(error) = prepaint.line.paint(
                prepaint.origin,
                px(HEIGHT),
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                eprintln!("Failed to paint text input: {error}");
            }
            if focus.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });

        self.input.update(cx, |input, _| {
            input.scroll_offset = prepaint.scroll_offset;
            input.last_bounds = Some(bounds);
            input.last_layout = Some(prepaint.line.clone());
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .cursor(CursorStyle::IBeam)
            .w_full()
            .min_w(px(0.0))
            .h(px(HEIGHT))
            .flex_shrink_0()
            .text_size(px(FONT_SIZE))
            .line_height(px(HEIGHT))
            .overflow_hidden()
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(|input, _: &TerminalCopy, window, cx| {
                input.copy(&Copy, window, cx);
            }))
            .on_action(cx.listener(|input, _: &TerminalPaste, window, cx| {
                input.paste(&Paste, window, cx);
            }))
            .on_action(cx.listener(|input, _: &TerminalSelectAll, window, cx| {
                input.select_all(&SelectAll, window, cx);
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn single_line(text: &str) -> String {
    // Keep UTF-16 lengths unchanged so IME-relative selections survive normalization.
    // Title trimming and length limits belong to the model when the editor saves.
    text.chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn offset_from_utf16(text: &str, offset: usize, round_up: bool) -> usize {
    let mut utf16 = 0;
    for (index, ch) in text.char_indices() {
        if offset == utf16 {
            return index;
        }
        let next = utf16 + ch.len_utf16();
        if offset < next {
            return if round_up {
                index + ch.len_utf8()
            } else {
                index
            };
        }
        utf16 = next;
    }
    text.len()
}

fn offset_to_utf16(text: &str, offset: usize) -> usize {
    text.char_indices()
        .take_while(|(index, ch)| index + ch.len_utf8() <= offset)
        .map(|(_, ch)| ch.len_utf16())
        .sum()
}

fn range_from_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    let start = range.start.min(range.end);
    let end = range.start.max(range.end);
    if start == end {
        let offset = offset_from_utf16(text, start, true);
        offset..offset
    } else {
        offset_from_utf16(text, start, false)..offset_from_utf16(text, end, true)
    }
}

fn range_to_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    offset_to_utf16(text, range.start)..offset_to_utf16(text, range.end)
}

fn safe_byte_range(text: &str, range: Range<usize>) -> Range<usize> {
    let mut start = range.start.min(range.end).min(text.len());
    let mut end = range.start.max(range.end).min(text.len());
    if start == end {
        while !text.is_char_boundary(start) {
            start += 1;
        }
        return start..start;
    }
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    while !text.is_char_boundary(end) {
        end += 1;
    }
    start..end
}

fn marked_selection(
    text: &str,
    start: usize,
    selection_utf16: Option<Range<usize>>,
) -> Range<usize> {
    // The selection is relative to the new marked text, not to the document or
    // the previous marked range. Both endpoints share the same insertion origin.
    let relative = selection_utf16
        .map(|range| range_from_utf16(text, range))
        .unwrap_or(text.len()..text.len());
    start + relative.start..start + relative.end
}

fn previous_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(index, _)| (index < offset.min(text.len())).then_some(index))
        .unwrap_or(0)
}

fn next_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(index, _)| (index > offset.min(text.len())).then_some(index))
        .unwrap_or(text.len())
}

fn floor_grapheme_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .take_while(|(index, _)| *index <= offset)
        .last()
        .map_or(0, |(index, _)| index)
}

fn ceil_grapheme_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(index, _)| (index >= offset).then_some(index))
        .unwrap_or(text.len())
}

fn deletion_range(text: &str, selection: Range<usize>, backwards: bool) -> Range<usize> {
    let selection = safe_byte_range(text, selection);
    if !selection.is_empty() {
        floor_grapheme_boundary(text, selection.start)..ceil_grapheme_boundary(text, selection.end)
    } else if backwards {
        previous_grapheme_boundary(text, selection.start)
            ..ceil_grapheme_boundary(text, selection.end)
    } else {
        floor_grapheme_boundary(text, selection.start)..next_grapheme_boundary(text, selection.end)
    }
}

fn visible_scroll_offset(
    current: Pixels,
    caret: Pixels,
    width: Pixels,
    viewport: Pixels,
) -> Pixels {
    let available = (viewport - px(CURSOR_WIDTH)).max(px(0.0));
    let maximum = (width - available).max(px(0.0));
    current
        .max(caret - available)
        .min(caret)
        .max(px(0.0))
        .min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_ranges_round_trip_at_scalar_boundaries() {
        let text = "a😀中e\u{301}";
        for byte_offset in text
            .char_indices()
            .map(|(index, _)| index)
            .chain([text.len()])
        {
            let utf16 = offset_to_utf16(text, byte_offset);
            assert_eq!(offset_from_utf16(text, utf16, false), byte_offset);
            assert_eq!(offset_from_utf16(text, utf16, true), byte_offset);
        }
        assert_eq!(range_from_utf16(text, 1..3), 1..5);
        assert_eq!(range_to_utf16(text, 1..5), 1..3);
    }

    #[test]
    fn malformed_ranges_do_not_split_surrogates_or_utf8() {
        let text = "a😀中";
        assert_eq!(range_from_utf16(text, 2..2), 5..5);
        assert_eq!(range_from_utf16(text, 2..3), 1..5);
        // A reversed range is malformed input from the platform; keep it safe.
        let (start, end) = (3, 2);
        assert_eq!(range_from_utf16(text, start..end), 1..5);
        assert_eq!(range_from_utf16(text, 0..usize::MAX), 0..text.len());
        assert_eq!(
            range_from_utf16(text, usize::MAX..usize::MAX),
            text.len()..text.len()
        );
        assert_eq!(safe_byte_range(text, 2..4), 1..5);
        assert_eq!(safe_byte_range(text, 3..3), 5..5);
        assert_eq!(safe_byte_range(text, 9..usize::MAX), text.len()..text.len());
        assert_eq!(range_from_utf16("", 0..usize::MAX), 0..0);
    }

    #[test]
    fn marked_selection_is_relative_to_replacement_text() {
        let prefix = "中😀";
        let new_text = "é😀字";
        assert_eq!(marked_selection(new_text, prefix.len(), Some(1..3)), 9..13);
        assert_eq!(marked_selection(new_text, prefix.len(), Some(3..3)), 13..13);
        assert_eq!(marked_selection(new_text, prefix.len(), None), 16..16);
        assert_eq!(
            marked_selection(new_text, prefix.len(), Some(0..usize::MAX)),
            7..16
        );
        assert_eq!(marked_selection("", prefix.len(), Some(0..2)), 7..7);
    }

    #[test]
    fn replacing_a_marked_range_uses_its_start_for_both_selection_ends() {
        let mut text = String::from("prefix:にほん:tail");
        let replaced = 7..16;
        let replacement = "日本😀";
        text.replace_range(replaced.clone(), replacement);
        let selection = marked_selection(replacement, replaced.start, Some(2..4));
        assert_eq!(text, "prefix:日本😀:tail");
        assert_eq!(selection, 13..17);
        assert_eq!(text.get(selection), Some("😀"));
    }

    #[test]
    fn movement_and_deletion_preserve_extended_graphemes() {
        let clusters = ["a", "e\u{301}", "👩🏽‍💻", "🇨🇳", "中"];
        let text = clusters.concat();
        let mut start = 0;
        for cluster in clusters {
            let end = start + cluster.len();
            assert_eq!(next_grapheme_boundary(&text, start), end);
            assert_eq!(previous_grapheme_boundary(&text, end), start);
            assert_eq!(deletion_range(&text, end..end, true), start..end);
            assert_eq!(deletion_range(&text, start..start, false), start..end);
            start = end;
        }
        assert_eq!(previous_grapheme_boundary(&text, 0), 0);
        assert_eq!(next_grapheme_boundary(&text, text.len()), text.len());
        assert_eq!(deletion_range(&text, 0..0, true), 0..0);
        assert_eq!(
            deletion_range(&text, text.len()..text.len(), false),
            text.len()..text.len()
        );
        assert_eq!(deletion_range("e\u{301}", 1..1, true), 0..3);
        assert_eq!(deletion_range("e\u{301}", 1..1, false), 0..3);
    }

    #[test]
    fn single_line_normalization_preserves_ime_offsets_and_has_no_length_limit() {
        let text = "a\r\n\t中\u{85}\u{2028}\u{2029}😀";
        let normalized = single_line(text);
        assert_eq!(normalized, "a   中   😀");
        assert_eq!(
            normalized.encode_utf16().count(),
            text.encode_utf16().count()
        );
        let long = "😀".repeat(300);
        assert_eq!(single_line(&long), long);
    }

    #[test]
    fn scrolling_keeps_caret_visible_and_recovers_after_shortening() {
        assert_eq!(
            visible_scroll_offset(px(0.0), px(200.0), px(200.0), px(80.0)),
            px(121.0)
        );
        assert_eq!(
            visible_scroll_offset(px(121.0), px(20.0), px(200.0), px(80.0)),
            px(20.0)
        );
        assert_eq!(
            visible_scroll_offset(px(121.0), px(10.0), px(20.0), px(80.0)),
            px(0.0)
        );
        assert_eq!(
            visible_scroll_offset(px(121.0), px(0.0), px(0.0), px(80.0)),
            px(0.0)
        );
        assert_eq!(
            visible_scroll_offset(px(0.0), px(10.0), px(20.0), px(0.0)),
            px(10.0)
        );
    }
}
