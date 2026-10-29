//! Cell-coordinate rendering for daemon-owned Herdr surfaces.
use gpui::{
    App, Bounds, Font, FontStyle, FontWeight, Pixels, Point, ShapedLine, Size, TextAlign, TextRun,
    UnderlineStyle, Window, fill, point, px, rgb, size,
};
use herdr_protocol::{CellData, FrameData, PaneSurfaceFrame, SurfaceRect};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    rc::Rc,
};

const ANSI: [u32; 16] = [
    0x1c1c1c, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666,
    0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
];

pub fn color(encoded: u32, default: u32) -> u32 {
    let index = encoded as u8;
    match encoded >> 24 {
        0 => match index {
            1..=16 => ANSI[usize::from(index - 1)],
            _ => default,
        },
        1 => match index {
            0..=15 => ANSI[usize::from(index)],
            16..=231 => {
                let n = u32::from(index) - 16;
                let level = |v| if v == 0 { 0 } else { 55 + v * 40 };
                (level(n / 36) << 16) | (level(n / 6 % 6) << 8) | level(n % 6)
            }
            _ => {
                let v = 8 + (u32::from(index) - 232) * 10;
                v * 0x010101
            }
        },
        2 => encoded & 0xffffff,
        _ => default,
    }
}

pub fn colors(cell: &CellData, foreground: u32, background: u32) -> (u32, u32) {
    let (mut fg, mut bg) = (color(cell.fg, foreground), color(cell.bg, background));
    if cell.modifier & (1 << 6) != 0 {
        std::mem::swap(&mut fg, &mut bg);
    }
    if cell.modifier & (1 << 1) != 0 {
        fg = ((fg & 0xfefefe) >> 1) + ((bg & 0xfefefe) >> 1);
    }
    if cell.modifier & (1 << 7) != 0 {
        fg = bg;
    }
    (fg, bg)
}

pub fn contains(rect: SurfaceRect, col: u16, row: u16) -> bool {
    col >= rect.x
        && row >= rect.y
        && u32::from(col) < u32::from(rect.x) + u32::from(rect.width)
        && u32::from(row) < u32::from(rect.y) + u32::from(rect.height)
}

pub fn popup_rect(surface: &PaneSurfaceFrame) -> Option<SurfaceRect> {
    let frame = &surface.popup.as_ref()?.frame;
    Some(SurfaceRect {
        x: surface.frame.width.saturating_sub(frame.width) / 2,
        y: surface.frame.height.saturating_sub(frame.height) / 2,
        width: frame.width,
        height: frame.height,
    })
}

pub fn selection_rows(
    rect: SurfaceRect,
    start: (u16, u16),
    end: (u16, u16),
) -> impl Iterator<Item = (u16, std::ops::Range<u16>)> {
    let right = rect.x.saturating_add(rect.width);
    let bottom = rect.y.saturating_add(rect.height);
    let clamp = |(col, row): (u16, u16)| {
        (
            row.clamp(rect.y, bottom.saturating_sub(1).max(rect.y)),
            col.clamp(rect.x, right.saturating_sub(1).max(rect.x)),
        )
    };
    let (a, b) = (clamp(start), clamp(end));
    let (a, b) = (a.min(b), a.max(b));
    (a.0..=b.0)
        .filter(move |_| rect.width > 0 && rect.height > 0)
        .map(move |row| {
            let left = if row == a.0 { a.1 } else { rect.x };
            let end = if row == b.0 {
                b.1.saturating_add(1)
            } else {
                right
            };
            (row, left..end)
        })
}

pub fn selected_text(
    frame: &FrameData,
    rect: SurfaceRect,
    start: (u16, u16),
    end: (u16, u16),
) -> String {
    selection_rows(rect, start, end)
        .map(|(row, columns)| {
            let mut line = String::new();
            for col in columns {
                if row < frame.height
                    && col < frame.width
                    && let Some(cell) = frame
                        .cells
                        .get(usize::from(row) * usize::from(frame.width) + usize::from(col))
                    && !cell.skip
                {
                    line.push_str(&cell.symbol);
                }
            }
            line.trim_end_matches(' ').to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const MAX_CACHED_GLYPHS: usize = 2048;
const MAX_CACHED_SYMBOL_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphStyle {
    foreground: u32,
    modifiers: u16,
}

impl GlyphStyle {
    fn new(cell: &CellData, foreground: u32) -> Self {
        Self {
            foreground,
            // Other modifiers are already reflected in the resolved color, or are
            // painted independently (strikethrough), so need no separate layout.
            modifiers: cell.modifier & (1 | (1 << 2) | (1 << 3)),
        }
    }
}

/// Retain one cache per window and pass it to every surface painted in that window.
#[derive(Default)]
pub struct GlyphCache {
    metrics: Option<(Font, Pixels, Size<Pixels>)>,
    lines: HashMap<GlyphStyle, HashMap<Rc<str>, Rc<ShapedLine>>>,
    insertion_order: VecDeque<(GlyphStyle, Rc<str>)>,
}

impl GlyphCache {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.insertion_order.clear();
        self.metrics = None;
    }

    fn prepare(&mut self, font: &Font, font_size: Pixels, cell: Size<Pixels>) {
        if self
            .metrics
            .as_ref()
            .is_none_or(|(cached_font, cached_size, cached_cell)| {
                cached_font != font || *cached_size != font_size || *cached_cell != cell
            })
        {
            self.clear();
            self.metrics = Some((font.clone(), font_size, cell));
        }
    }

    fn line(
        &mut self,
        symbol: &str,
        style: GlyphStyle,
        shape: impl FnOnce() -> ShapedLine,
    ) -> Rc<ShapedLine> {
        if let Some(line) = self.lines.get(&style).and_then(|lines| lines.get(symbol)) {
            return Rc::clone(line);
        }
        let line = Rc::new(shape());
        // Bound retained text/layout memory even for arbitrarily long combining
        // sequences received from the daemon. Oversized symbols still render.
        if symbol.len() > MAX_CACHED_SYMBOL_BYTES {
            return line;
        }
        if self.insertion_order.len() == MAX_CACHED_GLYPHS
            && let Some((old_style, old_symbol)) = self.insertion_order.pop_front()
            && let Some(lines) = self.lines.get_mut(&old_style)
        {
            lines.remove(old_symbol.as_ref());
            if lines.is_empty() {
                self.lines.remove(&old_style);
            }
        }
        let symbol: Rc<str> = symbol.into();
        self.lines
            .entry(style)
            .or_default()
            .insert(Rc::clone(&symbol), Rc::clone(&line));
        self.insertion_order.push_back((style, symbol));
        line
    }
}

fn background_runs(
    cells: &[CellData],
    foreground: u32,
    background: u32,
) -> impl Iterator<Item = (Range<usize>, u32)> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        let (_, color) = colors(cells.get(start)?, foreground, background);
        let mut end = start + 1;
        while end < cells.len() && colors(&cells[end], foreground, background).1 == color {
            end += 1;
        }
        let columns = start..end;
        start = end;
        Some((columns, color))
    })
}

pub struct CellPainter {
    pub font: Font,
    pub font_size: Pixels,
    pub cell: Size<Pixels>,
    pub foreground: u32,
    pub background: u32,
    /// Opacity of runs matching `background`; zero leaves the window backdrop visible.
    pub background_alpha: f32,
}

impl CellPainter {
    fn border(
        &self,
        symbol: &str,
        position: Point<Pixels>,
        color: u32,
        window: &mut Window,
    ) -> bool {
        let (left, right, up, down) = match symbol {
            "─" | "━" => (true, true, false, false),
            "│" | "┃" => (false, false, true, true),
            "┌" | "╭" => (false, true, false, true),
            "┐" | "╮" => (true, false, false, true),
            "└" | "╰" => (false, true, true, false),
            "┘" | "╯" => (true, false, true, false),
            "├" => (false, true, true, true),
            "┤" => (true, false, true, true),
            "┬" => (true, true, false, true),
            "┴" => (true, true, true, false),
            "┼" => (true, true, true, true),
            _ => return false,
        };
        let center = point(
            (self.cell.width * 0.5).floor(),
            (self.cell.height * 0.5).floor(),
        );
        let thickness = px(if matches!(symbol, "━" | "┃") {
            2.0
        } else {
            1.0
        });
        if left || right {
            let x = if left { px(0.0) } else { center.x };
            let end = if right {
                self.cell.width
            } else {
                center.x + thickness
            };
            window.paint_quad(fill(
                Bounds::new(position + point(x, center.y), size(end - x, thickness)),
                rgb(color),
            ));
        }
        if up || down {
            let y = if up { px(0.0) } else { center.y };
            let end = if down {
                self.cell.height
            } else {
                center.y + thickness
            };
            window.paint_quad(fill(
                Bounds::new(position + point(center.x, y), size(thickness, end - y)),
                rgb(color),
            ));
        }
        true
    }

    pub fn frame(
        &self,
        frame: &FrameData,
        origin: Point<Pixels>,
        focused: bool,
        cache: &mut GlyphCache,
        window: &mut Window,
        cx: &mut App,
    ) {
        let width = usize::from(frame.width);
        if width == 0 {
            return;
        }
        cache.prepare(&self.font, self.font_size, self.cell);
        let cells = &frame.cells[..frame.cells.len().min(width * usize::from(frame.height))];
        // Paint all backgrounds before glyphs: a wide glyph may overlap the next
        // cell, and its continuation still needs the correct background.
        for (row, cells) in cells.chunks(width).enumerate() {
            for (columns, bg) in background_runs(cells, self.foreground, self.background) {
                let mut color: gpui::Hsla = rgb(bg).into();
                if bg == self.background {
                    color.a = self.background_alpha;
                }
                if color.a <= 0.0 {
                    continue;
                }
                let position = origin
                    + point(
                        self.cell.width * columns.start as f32,
                        self.cell.height * row as f32,
                    );
                window.paint_quad(fill(
                    Bounds::new(
                        position,
                        size(self.cell.width * columns.len() as f32, self.cell.height),
                    ),
                    color,
                ));
            }
        }
        for (index, cell) in cells.iter().enumerate() {
            if cell.skip
                || cell.symbol.is_empty()
                || cell.symbol == " "
                || cell.modifier & (1 << 7) != 0
            {
                continue;
            }
            let col = index % width;
            let row = index / width;
            let position =
                origin + point(self.cell.width * col as f32, self.cell.height * row as f32);
            let (fg, _) = colors(cell, self.foreground, self.background);
            if self.border(&cell.symbol, position, fg, window) {
                continue;
            }
            let style = GlyphStyle::new(cell, fg);
            let line = cache.line(&cell.symbol, style, || {
                let mut font = self.font.clone();
                if style.modifiers & 1 != 0 {
                    font.weight = FontWeight::BOLD;
                }
                if style.modifiers & (1 << 2) != 0 {
                    font.style = FontStyle::Italic;
                }
                let text: String = cell.symbol.chars().filter(|c| !c.is_control()).collect();
                let run = TextRun {
                    len: text.len(),
                    font,
                    color: rgb(fg).into(),
                    background_color: None,
                    underline: (style.modifiers & (1 << 3) != 0).then_some(UnderlineStyle {
                        color: None,
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    strikethrough: None,
                };
                window
                    .text_system()
                    .shape_line(text.into(), self.font_size, &[run], None)
            });
            if let Err(error) = line.paint(
                position,
                self.cell.height,
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                eprintln!("Herdr glyph: {error}");
            }
            if cell.modifier & (1 << 8) != 0 {
                window.paint_quad(fill(
                    Bounds::new(
                        position + point(px(0.0), self.cell.height * 0.55),
                        size(self.cell.width, px(1.0)),
                    ),
                    rgb(fg),
                ));
            }
        }
        if focused
            && let Some(cursor) = frame
                .cursor
                .as_ref()
                .filter(|c| c.visible && c.x < frame.width && c.y < frame.height)
        {
            let position = origin
                + point(
                    self.cell.width * f32::from(cursor.x),
                    self.cell.height * f32::from(cursor.y),
                );
            let (offset, extent) = match cursor.shape {
                3 | 4 => (
                    point(px(0.0), self.cell.height - px(2.0)),
                    size(self.cell.width, px(2.0)),
                ),
                5 | 6 => (point(px(0.0), px(0.0)), size(px(2.0), self.cell.height)),
                _ => (point(px(0.0), px(0.0)), self.cell),
            };
            let mut color: gpui::Hsla = rgb(self.foreground).into();
            color.a = if cursor.shape == 0 { 0.35 } else { 0.85 };
            window.paint_quad(fill(Bounds::new(position + offset, extent), color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(symbol: &str) -> CellData {
        CellData {
            symbol: symbol.into(),
            fg: 0,
            bg: 0,
            modifier: 0,
            skip: false,
            hyperlink: None,
        }
    }

    #[test]
    fn glyph_cache_reuses_layouts_and_separates_painted_styles() {
        let mut cache = GlyphCache::default();
        let mut cell = cell("a");
        let style = GlyphStyle::new(&cell, 0xffffff);
        let first = cache.line(&cell.symbol, style, ShapedLine::default);
        let second = cache.line(&cell.symbol, style, || panic!("cached glyph reshaped"));
        assert!(Rc::ptr_eq(&first, &second));
        for modifier in [1, 1 << 2, 1 << 3] {
            cell.modifier = modifier;
            let styled = cache.line(
                &cell.symbol,
                GlyphStyle::new(&cell, 0xffffff),
                ShapedLine::default,
            );
            assert!(!Rc::ptr_eq(&first, &styled));
        }
        cell.modifier = 0;
        let recolored = cache.line(
            &cell.symbol,
            GlyphStyle::new(&cell, 0xabcdef),
            ShapedLine::default,
        );
        assert!(!Rc::ptr_eq(&first, &recolored));
        cell.modifier = 1 << 8;
        let struck = cache.line(&cell.symbol, GlyphStyle::new(&cell, 0xffffff), || {
            panic!("strikethrough needs no layout")
        });
        assert!(Rc::ptr_eq(&first, &struck));
    }

    #[test]
    fn glyph_cache_bounds_entries_and_does_not_retain_oversized_symbols() {
        let mut cache = GlyphCache::default();
        let style = GlyphStyle::new(&cell(""), 0xffffff);
        for index in 0..=MAX_CACHED_GLYPHS {
            cache.line(&index.to_string(), style, ShapedLine::default);
        }
        assert_eq!(cache.insertion_order.len(), MAX_CACHED_GLYPHS);
        assert_eq!(
            cache.lines.values().map(HashMap::len).sum::<usize>(),
            MAX_CACHED_GLYPHS
        );
        assert!(!cache.lines[&style].contains_key("0"));
        cache.line("1", style, || panic!("unrelated glyph was evicted"));
        let long = "a".repeat(MAX_CACHED_SYMBOL_BYTES + 1);
        let first = cache.line(&long, style, ShapedLine::default);
        let second = cache.line(&long, style, ShapedLine::default);
        assert!(!Rc::ptr_eq(&first, &second));
        assert_eq!(cache.insertion_order.len(), MAX_CACHED_GLYPHS);
    }

    #[test]
    fn glyph_cache_invalidates_font_size_and_cell_metrics() {
        let mut cache = GlyphCache::default();
        let style = GlyphStyle::new(&cell("x"), 0xffffff);
        let font = gpui::font("monospace");
        let metrics = size(px(8.0), px(16.0));
        cache.prepare(&font, px(12.0), metrics);
        let first = cache.line("x", style, ShapedLine::default);
        cache.prepare(&font, px(12.0), metrics);
        assert!(Rc::ptr_eq(
            &first,
            &cache.line("x", style, || panic!("unchanged metrics invalidated cache"))
        ));
        for (font, font_size, cell_size) in [
            (font.clone().bold(), px(12.0), metrics),
            (font.clone().bold(), px(14.0), metrics),
            (font.bold(), px(14.0), size(px(9.0), px(18.0))),
        ] {
            cache.prepare(&font, font_size, cell_size);
            assert!(cache.lines.is_empty());
            assert!(cache.insertion_order.is_empty());
            cache.line("x", style, ShapedLine::default);
        }
        cache.clear();
        assert!(cache.lines.is_empty());
        assert!(cache.metrics.is_none());
    }

    #[test]
    fn background_runs_cover_continuations_and_resolve_reverse_colors() {
        let mut cells = vec![cell("你"), cell(""), cell("x"), cell("y"), cell("z")];
        cells[1].skip = true;
        cells[2].bg = 0x02000000;
        cells[3].fg = 0x02123456;
        cells[3].modifier = (1 << 6) | (1 << 7);
        cells[4].bg = 0x02123456;
        assert_eq!(
            background_runs(&cells, 0xffffff, 0).collect::<Vec<_>>(),
            vec![(0..3, 0), (3..5, 0x123456)],
        );
        let rows = cells
            .chunks(3)
            .map(|row| background_runs(row, 0xffffff, 0).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(rows, vec![vec![(0..3, 0)], vec![(0..2, 0x123456)]]);
    }

    #[test]
    fn uniform_background_requires_one_run_per_row() {
        let cells = vec![cell(" "); 80 * 24];
        let runs = cells
            .chunks(80)
            .map(|row| background_runs(row, 0xffffff, 0).count())
            .sum::<usize>();
        assert_eq!(runs, 24);
    }

    #[test]
    fn wire_colors_are_not_argb() {
        assert_eq!(color(0x02123456, 0), 0x123456);
        assert_eq!(color(0x01000010, 0), 0);
        assert_eq!(color(0x010000e7, 0), 0xffffff);
        assert_eq!(color(0x010000ff, 0), 0xeeeeee);
        assert_eq!(color(0, 0xabcdef), 0xabcdef);
        assert_eq!(color(1, 0), ANSI[0]);
    }
    #[test]
    fn reversed_hidden_and_dim_apply_to_resolved_colors() {
        let mut cell = CellData {
            symbol: "x".into(),
            fg: 0x02ffffff,
            bg: 0x02000000,
            modifier: 1 << 6,
            skip: false,
            hyperlink: None,
        };
        assert_eq!(colors(&cell, 0, 0), (0, 0xffffff));
        cell.modifier |= 1 << 7;
        assert_eq!(colors(&cell, 0, 0), (0xffffff, 0xffffff));
        cell.modifier = 1 << 1;
        assert_eq!(colors(&cell, 0, 0), (0x7f7f7f, 0));
    }
    #[test]
    fn selection_stays_in_pane_and_skips_wide_continuations() {
        let mut cells: Vec<_> = "|你·a|XX||bc |YY|"
            .chars()
            .map(|ch| CellData {
                symbol: ch.to_string(),
                fg: 0,
                bg: 0,
                modifier: 0,
                skip: false,
                hyperlink: None,
            })
            .collect();
        cells[2].skip = true;
        let frame = FrameData {
            cells,
            width: 8,
            height: 2,
            cursor: None,
            hyperlinks: vec![],
            graphics: vec![],
        };
        let rect = SurfaceRect {
            x: 1,
            y: 0,
            width: 3,
            height: 2,
        };
        assert_eq!(selected_text(&frame, rect, (1, 0), (7, 1)), "你a\nbc");
        assert_eq!(selected_text(&frame, rect, (7, 1), (1, 0)), "你a\nbc");
        let empty = SurfaceRect { width: 0, ..rect };
        assert!(selected_text(&frame, empty, (1, 0), (1, 1)).is_empty());
    }

    #[test]
    fn hit_test_uses_half_open_cell_rectangles() {
        let rect = SurfaceRect {
            x: 5,
            y: 3,
            width: 10,
            height: 4,
        };
        assert!(contains(rect, 5, 3));
        assert!(contains(rect, 14, 6));
        assert!(!contains(rect, 15, 6));
        assert!(!contains(rect, 5, 7));
    }
}
