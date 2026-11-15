//! The hex editor an array tag opens, laid out as in UMT: a toolbar (Overwrite or Insert,
//! bytes per line, close), then the offsets, the bytes in hex and the same bytes as text.
//! Import and Export read and write the bytes as a raw file.

use eframe::egui::{
    self, Align2, Color32, Event, FontId, Key, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2,
};

use crate::icons::Icon;
use crate::theme;
use crate::widgets;

const ROW_H: f32 = 17.0;
const TOOLBAR_H: f32 = 40.0;
/// The bytes-per-line choices after "Auto".
const LINE_CHOICES: [usize; 5] = [8, 16, 32, 64, 128];

/// What happened in a frame, for the tag the editor belongs to.
pub struct Outcome {
    /// The area under the toolbar.
    pub body: Rect,
    /// The bytes were edited (or replaced by an import).
    pub changed: bool,
    /// The X was clicked.
    pub close: bool,
    /// A message for the user.
    pub note: Option<String>,
}

impl Default for Outcome {
    fn default() -> Self {
        Outcome { body: Rect::NOTHING, changed: false, close: false, note: None }
    }
}

pub struct HexEditor {
    /// Size of one array value in bytes: 1, 4 or 8. Insert and delete work in whole values.
    width: usize,
    bytes: Vec<u8>,
    cursor: usize,
    /// Which digit of the byte at the cursor is typed next: 0 high, 1 low.
    nibble: usize,
    /// Where a selection started, when there is one.
    anchor: Option<usize>,
    insert: bool,
    /// Bytes per line; 0 is automatic.
    per_line: usize,
    /// Scroll the cursor into view on the next frame.
    reveal: bool,
    /// End (exclusive) of the value being filled in after Insert made room for it.
    fill_until: Option<usize>,
    /// Drawn for the first time: the click that opened the editor is not for its bytes.
    fresh: bool,
}

impl HexEditor {
    pub fn new(bytes: Vec<u8>, width: usize) -> Self {
        HexEditor {
            width,
            bytes,
            cursor: 0,
            nibble: 0,
            anchor: None,
            insert: false,
            per_line: 0,
            reveal: false,
            fill_until: None,
            fresh: true,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Replaces the bytes (after an Undo, say), keeping the cursor where it can be.
    pub fn set_bytes(&mut self, bytes: Vec<u8>) {
        self.bytes = bytes;
        self.cursor = self.cursor.min(self.bytes.len().saturating_sub(1));
        self.anchor = None;
        self.nibble = 0;
    }

    fn selection(&self) -> Option<(usize, usize)> {
        self.anchor
            .filter(|a| *a != self.cursor)
            .map(|a| (a.min(self.cursor), a.max(self.cursor)))
    }

    fn move_to(&mut self, to: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = to.min(self.bytes.len().saturating_sub(1));
        self.nibble = 0;
        self.reveal = true;
    }

    /// The start of the value the byte `i` belongs to.
    fn value_start(&self, i: usize) -> usize {
        i - i % self.width
    }

    /// Types one hex digit at the cursor. In Insert mode the first digit of a value makes room
    /// for a whole new value (one byte, or four or eight for the wider arrays), which the
    /// digits after it then fill in.
    fn type_digit(&mut self, d: u8) -> bool {
        self.anchor = None;
        if self.insert && self.nibble == 0 && self.fill_until.is_none_or(|end| self.cursor >= end) {
            let at = self.value_start(self.cursor.min(self.bytes.len()));
            self.bytes.splice(at..at, std::iter::repeat_n(0, self.width));
            self.cursor = at;
            self.fill_until = Some(at + self.width);
        }
        if self.cursor >= self.bytes.len() {
            return false;
        }
        let b = &mut self.bytes[self.cursor];
        *b = if self.nibble == 0 { (*b & 0x0F) | (d << 4) } else { (*b & 0xF0) | d };
        if self.nibble == 1 {
            self.nibble = 0;
            self.cursor += 1;
            if self.fill_until.is_some_and(|end| self.cursor >= end) {
                self.fill_until = None;
            }
            if !self.insert {
                self.cursor = self.cursor.min(self.bytes.len() - 1);
            }
        } else {
            self.nibble = 1;
        }
        self.reveal = true;
        true
    }

    /// Removes the selected values, or the one before (`back`) or at the cursor, in Insert mode.
    fn delete(&mut self, back: bool) -> bool {
        if !self.insert {
            return false;
        }
        self.fill_until = None;
        let w = self.width;
        if let Some((a, b)) = self.selection() {
            let (from, to) = (self.value_start(a), (self.value_start(b) + w).min(self.bytes.len()));
            self.bytes.drain(from..to);
            self.cursor = from.min(self.bytes.len().saturating_sub(1));
            self.anchor = None;
            self.nibble = 0;
            return true;
        }
        let here = self.value_start(self.cursor.min(self.bytes.len().saturating_sub(1)));
        let at = if back { here.checked_sub(w) } else { Some(here) };
        match at.filter(|i| i + w <= self.bytes.len()) {
            Some(i) => {
                self.bytes.drain(i..i + w);
                self.cursor = i.min(self.bytes.len().saturating_sub(1));
                self.nibble = 0;
                true
            }
            None => false,
        }
    }

    fn paste(&mut self, text: &str) -> bool {
        let digits: Vec<u8> = text.chars().filter_map(|c| c.to_digit(16)).map(|d| d as u8).collect();
        let bytes: Vec<u8> = digits.chunks_exact(2).map(|p| p[0] << 4 | p[1]).collect();
        if bytes.is_empty() {
            return false;
        }
        self.fill_until = None;
        if self.insert && bytes.len() % self.width == 0 {
            // Whole values go in before the one at the cursor.
            let at = self.value_start(self.cursor.min(self.bytes.len()));
            let n = bytes.len();
            self.bytes.splice(at..at, bytes);
            self.cursor = (at + n).min(self.bytes.len().saturating_sub(1));
        } else {
            for b in bytes {
                if self.cursor < self.bytes.len() {
                    self.bytes[self.cursor] = b;
                    self.cursor = (self.cursor + 1).min(self.bytes.len() - 1);
                }
            }
        }
        self.nibble = 0;
        self.anchor = None;
        true
    }
    fn copy_text(&self) -> String {
        let (a, b) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.bytes
            .get(a..=b.min(self.bytes.len().saturating_sub(1)))
            .unwrap_or_default()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Draws the editor over the whole of `ui`'s rectangle. `name` is the tag's name, for the
    /// file names of Import and Export.
    ///
    /// `views` names the ways to look at the array when there is more than the hex dump (the
    /// first is the hex dump itself); `view` is the one chosen. For any but the first, the caller
    /// draws the picture in `Outcome::body`.
    pub fn ui(&mut self, ui: &mut Ui, name: &str, read_only: bool, views: &[&str], view: &mut usize) -> Outcome {
        let mut out = Outcome::default();
        let full = ui.max_rect();
        let bar = Rect::from_min_size(full.min, vec2(full.width(), TOOLBAR_H));
        ui.painter().rect_filled(bar, 0.0, theme::BG);
        ui.painter().line_segment(
            [bar.left_bottom(), bar.right_bottom()],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        self.toolbar(ui, bar, name, read_only, views, view, &mut out);
        let body = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
        out.body = body;
        if *view == 0 {
            self.body(ui, body, read_only, &mut out);
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn toolbar(
        &mut self,
        ui: &mut Ui,
        bar: Rect,
        name: &str,
        read_only: bool,
        views: &[&str],
        view: &mut usize,
        out: &mut Outcome,
    ) {
        // A narrow window draws the whole bar smaller.
        let k = (bar.width() / 660.0).clamp(0.65, 1.0);
        theme::set_ui_scale(k);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(vec2(8.0, 5.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 8.0 * k;
        ui.style_mut().override_font_id = Some(theme::regular(14.0 * k));
        // Other ways to look at the array (a map's picture, light layers) beside the hex dump.
        if views.len() > 1 {
            ui.label(egui::RichText::new("View:").color(theme::TEXT_MUTED));
            widgets::segmented(ui, view, views, 0.0, true);
            let x = ui.cursor().left() + 6.0;
            ui.painter().line_segment(
                [pos2(x, bar.top() + 9.0), pos2(x, bar.bottom() - 9.0)],
                Stroke::new(1.0_f32, theme::BORDER),
            );
            ui.add_space(12.0);
        }
        if *view == 0 {
            ui.label(egui::RichText::new("Mode:").color(theme::TEXT_MUTED));
            let mut mode = self.insert as usize;
            if widgets::segmented(ui, &mut mode, &["Overwrite", "Insert"], 0.0, !read_only) {
                self.insert = mode == 1;
                self.fill_until = None;
            }
            let x = ui.cursor().left() + 6.0;
            ui.painter().line_segment(
                [pos2(x, bar.top() + 9.0), pos2(x, bar.bottom() - 9.0)],
                Stroke::new(1.0_f32, theme::BORDER),
            );
            ui.add_space(12.0 * k);
            ui.label(egui::RichText::new("Bytes Per Line:").color(theme::TEXT_MUTED));
            self.lines_picker(ui);
        }
        // Import, Export and the close cross sit at the right.
        let mut right = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(vec2(8.0, 5.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        right.spacing_mut().item_spacing.x = 8.0 * k;
        if widgets::square_icon_button(&mut right, Icon::Close, "Back to the tags").clicked() {
            out.close = true;
        }
        if widgets::compact_button(&mut right, "Export", true)
            .on_hover_text("Save the bytes to a file")
            .clicked()
        {
            self.export(name, out);
        }
        if widgets::compact_button(&mut right, "Import", !read_only)
            .on_hover_text("Replace the bytes with the contents of a file")
            .clicked()
        {
            self.import(out);
        }
        theme::set_ui_scale(1.0);
    }

    fn lines_picker(&mut self, ui: &mut Ui) {
        let id = ui.make_persistent_id("hex-per-line");
        let k = theme::ui_scale();
        let (r, resp) = ui.allocate_exact_size(vec2(96.0, 28.0) * k, Sense::click());
        let open = widgets::dropdown_open(ui, id);
        ui.painter().rect_filled(
            r,
            widgets::dropdown_corners(open),
            if resp.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON },
        );
        let label = if self.per_line == 0 { "Auto".to_string() } else { self.per_line.to_string() };
        ui.painter().text(
            r.left_center() + vec2(10.0 * k, 0.0),
            Align2::LEFT_CENTER,
            label,
            theme::regular(14.0 * k),
            theme::TEXT,
        );
        crate::icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(r.right_center() - vec2(12.0 * k, 0.0), egui::Vec2::splat(10.0 * k)),
            if open { Icon::ChevronUp } else { Icon::ChevronDown },
            theme::TEXT_MUTED,
        );
        if resp.clicked() {
            ui.memory_mut(|m| m.toggle_popup(id));
        }
        let tints = (theme::BUTTON_HOVER, Color32::from_white_alpha(14));
        let mut pick = None;
        widgets::dropdown(ui, id, &resp, theme::BUTTON, None, |ui, _| {
            let choices = std::iter::once(0).chain(LINE_CHOICES);
            let count = LINE_CHOICES.len() + 1;
            for (i, n) in choices.enumerate() {
                let text = if n == 0 { "Auto".to_string() } else { n.to_string() };
                if widgets::dropdown_item(ui, self.per_line == n, i + 1 == count, tints, &text).clicked() {
                    pick = Some(n);
                }
            }
        });
        if let Some(n) = pick {
            self.per_line = n;
        }
    }

    fn import(&mut self, out: &mut Outcome) {
        let Some(path) = rfd::FileDialog::new().set_title("Import bytes").pick_file() else {
            return;
        };
        match std::fs::read(&path) {
            Err(e) => out.note = Some(format!("Could not read the file: {e}")),
            Ok(b) if b.len() % self.width != 0 => {
                out.note = Some(format!(
                    "The file is {} bytes, not a whole number of {}-byte values",
                    b.len(),
                    self.width
                ));
            }
            Ok(b) => {
                out.note = Some(format!("Imported {} bytes", b.len()));
                self.bytes = b;
                self.cursor = 0;
                self.anchor = None;
                self.nibble = 0;
                out.changed = true;
            }
        }
    }

    fn export(&self, name: &str, out: &mut Outcome) {
        let name = if name.is_empty() { "array" } else { name };
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export bytes")
            .set_file_name(format!("{name}.bin"))
            .save_file()
        else {
            return;
        };
        out.note = Some(match std::fs::write(&path, &self.bytes) {
            Ok(()) => format!("Exported {} bytes", self.bytes.len()),
            Err(e) => format!("Could not write the file: {e}"),
        });
    }

    fn body(&mut self, ui: &mut Ui, body: Rect, read_only: bool, out: &mut Outcome) {
        let font = FontId::monospace(13.0);
        let cw = ui.fonts(|f| f.glyph_width(&font, '0'));
        let digits = format!("{:X}", self.bytes.len().saturating_sub(1)).len().max(4);
        let off_w = (digits as f32 + 2.0) * cw;
        let left = body.left() + 6.0;
        let avail = body.width() - 12.0 - off_w - 3.0 * cw - 14.0;
        let per_line = if self.per_line > 0 {
            self.per_line
        } else {
            ((avail / (4.0 * cw)).floor() as usize).max(1)
        };
        let rows = self.bytes.len().div_ceil(per_line).max(1);
        let hex_x = left + off_w;
        let ascii_x = hex_x + per_line as f32 * 3.0 * cw + 2.0 * cw;
        let content_h = rows as f32 * ROW_H + 8.0;
        let len = self.bytes.len();

        let offset_colour = Color32::from_rgb(0x4E, 0x6A, 0x96);
        let zero_colour = Color32::from_rgb(0x3A, 0x48, 0x62);
        let byte_colour = Color32::from_rgb(0x7F, 0x97, 0xBF);
        let text_colour = Color32::from_rgb(0x9A, 0xAE, 0xCF);

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        let ui = &mut child;
        let mut scroll = egui::ScrollArea::vertical().auto_shrink(false).id_salt("hex-scroll");
        if std::mem::take(&mut self.reveal) {
            let top = (self.cursor / per_line) as f32 * ROW_H;
            let view = body.height();
            let state = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::Id::new("hex-scroll")));
            let current = state.map_or(0.0, |s| s.offset.y);
            if top < current {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + ROW_H > current + view {
                scroll = scroll.vertical_scroll_offset(top + ROW_H - view);
            }
        }
        scroll.show_viewport(ui, |ui, viewport| {
            let (rect, resp) = ui.allocate_exact_size(vec2(body.width(), content_h), Sense::click_and_drag());
            let id = resp.id;
            ui.ctx().memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        tab: false,
                        escape: false,
                    },
                )
            });
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            let origin = rect.min;
            // A pointer position as a byte of the bytes shown, and which digit it is over.
            let byte_at = |pos: Pos2| -> Option<(usize, usize)> {
                let row = ((pos.y - origin.y - 4.0) / ROW_H).floor().max(0.0) as usize;
                let (col, nib) = if pos.x >= ascii_x - cw {
                    (((pos.x - ascii_x) / cw).floor().max(0.0) as usize, 0)
                } else {
                    let rel = (pos.x - hex_x).max(0.0);
                    ((rel / (3.0 * cw)).floor() as usize, ((rel % (3.0 * cw)) > cw) as usize)
                };
                let i = row * per_line + col.min(per_line - 1);
                (len > 0).then(|| (i.min(len - 1), nib))
            };
            let first_frame = std::mem::take(&mut self.fresh);
            if !first_frame && (resp.drag_started() || resp.clicked()) {
                resp.request_focus();
                if let Some((i, nib)) = resp.interact_pointer_pos().and_then(byte_at) {
                    let shift = ui.input(|i| i.modifiers.shift);
                    self.move_to(i, shift);
                    self.nibble = nib;
                    self.reveal = false;
                    if !shift {
                        self.anchor = Some(i);
                    }
                }
            } else if !first_frame && resp.dragged() {
                if let Some((i, _)) = resp.interact_pointer_pos().and_then(byte_at) {
                    if self.cursor != i {
                        self.cursor = i;
                        self.nibble = 0;
                    }
                }
            }
            if resp.has_focus() {
                self.keys(ui, per_line, ((body.height() / ROW_H) as usize).max(1), read_only, out);
            }
            let painter = ui.painter_at(body);
            // The line between the bytes and their text.
            painter.line_segment(
                [pos2(ascii_x - cw, body.top()), pos2(ascii_x - cw, body.bottom())],
                Stroke::new(1.0_f32, theme::BORDER),
            );
            let first = ((viewport.min.y - 4.0) / ROW_H).floor().max(0.0) as usize;
            let last = (((viewport.max.y) / ROW_H).ceil() as usize + 1).min(rows);
            let selection = self.selection();
            for r in first..last {
                let y = origin.y + 4.0 + r as f32 * ROW_H;
                painter.text(
                    pos2(left, y),
                    Align2::LEFT_TOP,
                    format!("{:0w$X}", r * per_line, w = digits),
                    font.clone(),
                    offset_colour,
                );
                for c in 0..per_line {
                    let i = r * per_line + c;
                    let Some(b) = self.bytes.get(i).copied() else { break };
                    let bx = hex_x + c as f32 * 3.0 * cw;
                    if selection.is_some_and(|(a, z)| i >= a && i <= z) {
                        let cell = Rect::from_min_size(pos2(bx - cw * 0.5, y), vec2(3.0 * cw, ROW_H));
                        painter.rect_filled(cell, 0.0, theme::accent_wash(0.35));
                        let ch = Rect::from_min_size(pos2(ascii_x + c as f32 * cw, y), vec2(cw, ROW_H));
                        painter.rect_filled(ch, 0.0, theme::accent_wash(0.35));
                    }
                    painter.text(
                        pos2(bx, y),
                        Align2::LEFT_TOP,
                        format!("{b:02X}"),
                        font.clone(),
                        if b == 0 { zero_colour } else { byte_colour },
                    );
                    let ch = if (0x20..0x7F).contains(&b) { b as char } else { '.' };
                    painter.text(
                        pos2(ascii_x + c as f32 * cw, y),
                        Align2::LEFT_TOP,
                        ch,
                        font.clone(),
                        if ch == '.' { zero_colour } else { text_colour },
                    );
                }
            }
            // The cursor: a green line under the digit being typed, and under its character.
            if !self.bytes.is_empty() || self.insert {
                let (r, c) = (self.cursor / per_line, self.cursor % per_line);
                let y = origin.y + 4.0 + r as f32 * ROW_H + ROW_H - 2.0;
                let green = theme::EDITOR_ACCENT;
                let dx = hex_x + c as f32 * 3.0 * cw + self.nibble as f32 * cw;
                painter.line_segment([pos2(dx, y), pos2(dx + cw, y)], Stroke::new(2.0_f32, green));
                let ax = ascii_x + c as f32 * cw;
                painter.line_segment([pos2(ax, y), pos2(ax + cw, y)], Stroke::new(2.0_f32, green));
            }
        });
    }

    fn keys(&mut self, ui: &mut Ui, per_line: usize, page: usize, read_only: bool, out: &mut Outcome) {
        let events = ui.input(|i| i.events.clone());
        for e in events {
            match e {
                Event::Text(t) if !read_only => {
                    for d in t.chars().filter_map(|c| c.to_digit(16)) {
                        out.changed |= self.type_digit(d as u8);
                    }
                }
                Event::Paste(t) if !read_only => out.changed |= self.paste(&t),
                Event::Copy | Event::Cut => {
                    let text = self.copy_text();
                    ui.ctx().copy_text(text);
                }
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let extend = modifiers.shift;
                    let c = self.cursor;
                    match key {
                        Key::ArrowLeft => self.move_to(c.saturating_sub(1), extend),
                        Key::ArrowRight => self.move_to(c + 1, extend),
                        Key::ArrowUp => self.move_to(c.saturating_sub(per_line), extend),
                        Key::ArrowDown => self.move_to(c + per_line, extend),
                        Key::PageUp => self.move_to(c.saturating_sub(per_line * page), extend),
                        Key::PageDown => self.move_to(c + per_line * page, extend),
                        Key::Home if modifiers.command => self.move_to(0, extend),
                        Key::End if modifiers.command => self.move_to(usize::MAX, extend),
                        Key::Home => self.move_to(c - c % per_line, extend),
                        Key::End => self.move_to(c - c % per_line + per_line - 1, extend),
                        Key::A if modifiers.command => {
                            self.anchor = Some(0);
                            self.cursor = self.bytes.len().saturating_sub(1);
                        }
                        Key::Backspace if !read_only => out.changed |= self.delete(true),
                        Key::Delete if !read_only => out.changed |= self.delete(false),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_overwrites_a_byte_digit_by_digit() {
        let mut h = HexEditor::new(vec![0, 0, 0], 1);
        assert!(h.type_digit(0xA));
        assert_eq!(h.bytes(), &[0xA0, 0, 0]);
        assert!(h.type_digit(0x5));
        assert_eq!(h.bytes(), &[0xA5, 0, 0]);
        assert_eq!(h.cursor, 1);
    }

    #[test]
    fn insert_adds_and_delete_removes_bytes() {
        let mut h = HexEditor::new(vec![1, 2], 1);
        h.insert = true;
        h.type_digit(0xF);
        h.type_digit(0x0);
        assert_eq!(h.bytes(), &[0xF0, 1, 2]);
        h.cursor = 1;
        assert!(h.delete(false));
        assert_eq!(h.bytes(), &[0xF0, 2]);
    }

    #[test]
    fn wide_arrays_insert_and_delete_whole_values() {
        let mut h = HexEditor::new(vec![0, 0, 0, 1, 0, 0, 0, 2], 4);
        h.insert = true;
        h.cursor = 4;
        // The first digit makes room for a whole four-byte value; the rest fill it in.
        for d in [0xA, 0xB, 0xC, 0xD, 0xE, 0xF, 0x1, 0x2] {
            h.type_digit(d);
        }
        assert_eq!(h.bytes(), &[0, 0, 0, 1, 0xAB, 0xCD, 0xEF, 0x12, 0, 0, 0, 2]);
        h.cursor = 5;
        assert!(h.delete(false));
        assert_eq!(h.bytes(), &[0, 0, 0, 1, 0, 0, 0, 2]);
    }

    #[test]
    fn paste_reads_hex_with_any_separators() {
        let mut h = HexEditor::new(vec![0; 4], 1);
        assert!(h.paste("de AD, be-ef"));
        assert_eq!(h.bytes(), &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert!(!h.paste("zz"));
    }

    #[test]
    fn copy_gives_the_selection_as_hex() {
        let mut h = HexEditor::new(vec![0x10, 0x20, 0x30], 1);
        h.anchor = Some(0);
        h.cursor = 1;
        assert_eq!(h.copy_text(), "10 20");
    }
}
