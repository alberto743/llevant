//! A cell's geometry edited as text, with help (decision 0012, phase 8):
//! its words coloured as in cards, surfaces and cells the input has not got
//! flagged as you type, surface numbers completed with their type and
//! comment (Tab, or a click; ↑ ↓ to choose), and the surfaces it names
//! marked on the slice ([`crate::marks`]). A surface number under the
//! pointer is underlined, named with its type and comment, and marked in
//! the views; Ctrl+click opens its page.

use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use llevant_scene::source::{geometry_words, CardKind, Flag, Source, Word};

use super::card::{COMPLEMENT, SURFACE};
use crate::document::Document;

/// Completions listed at most.
const LISTED: usize = 6;

pub struct GeometryEditor {
    pub text: String,
    /// The geometry as the card has it, its words one space apart.
    was: String,
    /// Every surface of the input with its type (read once, on first use).
    surfaces: Option<Vec<(i64, String)>>,
    /// The completions offered last frame, and the one chosen.
    offered: Vec<i64>,
    chosen: usize,
    /// The number being typed (where its digits start, the digits, the
    /// cursor after them), kept while a completion is clicked (which takes
    /// the focus from the text).
    typed: Option<(usize, String, usize)>,
    /// Whether the text has the keyboard (its surfaces are marked then).
    pub focused: bool,
    /// The surface number under the pointer, and where it is (bytes).
    pub hovered: Option<i64>,
    hover_span: Option<std::ops::Range<usize>>,
}

/// `text`'s words one space apart (line breaks are spaces).
fn plain(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl GeometryEditor {
    pub fn new(text: &str) -> GeometryEditor {
        GeometryEditor {
            text: plain(text),
            was: plain(text),
            surfaces: None,
            offered: Vec::new(),
            chosen: 0,
            typed: None,
            focused: false,
            hovered: None,
            hover_span: None,
        }
    }

    pub fn changed(&self) -> bool {
        plain(&self.text) != self.was
    }

    /// The surfaces it names (to mark), each once.
    pub fn surfaces(&self) -> Vec<i64> {
        let mut v: Vec<i64> = geometry_words(&self.text)
            .into_iter()
            .filter_map(|w| match w.kind {
                Word::Surface { id, .. } => Some(id),
                _ => None,
            })
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// What is wrong with it (nothing when unchanged: the card reads).
    pub fn flags(&self, source: &Source) -> Vec<Flag> {
        if self.changed() {
            source.check_geometry(&self.text)
        } else {
            Vec::new()
        }
    }

    /// The text and its help; the surface Ctrl+clicked, if one was.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        salt: impl std::hash::Hash + std::fmt::Debug,
    ) -> Option<i64> {
        let source = doc.source.clone()?;
        let id = egui::Id::new(("geometry editor", salt));
        let focused = ui.memory(|m| m.has_focus(id));
        // Keys for the completions offered, before the text takes them.
        let mut accept = false;
        if focused && !self.offered.is_empty() {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    self.chosen = (self.chosen + 1) % self.offered.len();
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    self.chosen = (self.chosen + self.offered.len() - 1) % self.offered.len();
                }
                accept = i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
            });
        }
        let strong = ui.visuals().strong_text_color();
        let plain_colour = ui.visuals().text_color();
        let bad = ui.visuals().error_fg_color;
        let src = source.clone();
        // (The card's own words read: checked only once changed.)
        let was = self.was.clone();
        let span = self.hover_span.clone();
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let check = plain(buf.as_str()) != was;
            let mut job = coloured(buf.as_str(), &src, [plain_colour, strong, bad], wrap, check);
            // The surface under the pointer, underlined.
            if let Some(s) = &span {
                for sec in &mut job.sections {
                    if sec.byte_range.start.0 < s.end && s.start < sec.byte_range.end.0 {
                        sec.format.underline = egui::Stroke::new(1.5, SURFACE);
                    }
                }
            }
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let out = egui::TextEdit::multiline(&mut self.text)
            .id(id)
            .code_editor()
            .desired_rows(2)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter)
            .show(ui);
        crate::panels::inspector::fields::name_text(&out.response.response, "Geometry");
        self.focused = out.response.response.has_focus();
        // The surface number under the pointer.
        let under = out
            .response
            .response
            .hover_pos()
            .and_then(|p| surface_at(&self.text, &out.galley, p - out.galley_pos));
        if under.as_ref().map(|u| &u.1) != self.hover_span.as_ref() {
            ui.ctx().request_repaint();
        }
        self.hovered = under.as_ref().map(|u| u.0);
        self.hover_span = under.map(|u| u.1);
        let mut opened = None;
        match self.hovered {
            Some(s) => {
                let kind = source
                    .surface_coeffs(s)
                    .map(|(k, _)| k.to_uppercase())
                    .unwrap_or_else(|| "not in the input".into());
                let note = super::comment(doc, CardKind::Surface, s)
                    .map(|c| format!(" · {c}"))
                    .unwrap_or_default();
                let r = out.response.response.clone().on_hover_text(format!(
                    "s {s} · {kind}{note}\nMarked in the views · Ctrl+click: its page"
                ));
                if ui.input(|i| i.modifiers.command) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    if r.clicked() {
                        opened = Some(s);
                    }
                }
            }
            None => {
                out.response.response.clone().on_hover_text(
                    "The cell's geometry: surfaces with their sense (-1 inside, 2 outside), \
                     ( ) to group, : for either, #n not cell n, #( ) not the group. \
                     Surface numbers complete as you type (Tab, or click one; ↑ ↓ to choose). \
                     Hover a surface to see it, Ctrl+click it for its page.",
                );
            }
        }

        // The number being typed at the cursor, and the surfaces it may be.
        let cursor = out
            .cursor_range
            .filter(|r| r.primary.index == r.secondary.index)
            .map(|r| r.primary.index.0);
        let clicking = ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
        if out.response.response.has_focus() {
            self.typed = cursor.and_then(|c| typed_number(&self.text, c).map(|(s, d)| (s, d, c)));
        } else if !clicking {
            self.typed = None;
        }
        let typed = self.typed.clone();
        // Every surface of the input, read once something is typed.
        if typed.is_some() && self.surfaces.is_none() {
            self.surfaces = Some(source.surface_list());
        }
        let list: &[(i64, String)] = self.surfaces.as_deref().unwrap_or_default();
        let offered: Vec<i64> = match &typed {
            Some((_, prefix, _)) => {
                let exact = prefix.parse::<i64>().ok();
                let v: Vec<i64> = list
                    .iter()
                    .map(|e| e.0)
                    .filter(|id| id.to_string().starts_with(prefix.as_str()))
                    .take(LISTED)
                    .collect();
                // A number typed in full that no other starts with:
                // nothing to offer.
                if v.len() == 1 && Some(v[0]) == exact {
                    Vec::new()
                } else {
                    v
                }
            }
            None => Vec::new(),
        };
        if offered != self.offered {
            self.chosen = 0;
        }
        self.offered = offered;
        let mut pick = accept.then_some(self.chosen);
        if !self.offered.is_empty() {
            egui::Frame::popup(ui.style())
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    for (k, &sid) in self.offered.iter().enumerate() {
                        let kind = list
                            .binary_search_by_key(&sid, |e| e.0)
                            .ok()
                            .map(|i| list[i].1.clone())
                            .unwrap_or_default();
                        let note = super::comment(doc, CardKind::Surface, sid).unwrap_or_default();
                        let text = crate::theme::data(format!("{sid:<8} {kind:<5} {note}"));
                        let r = ui.add(egui::Button::selectable(k == self.chosen, text).small());
                        crate::app::frame::named(&r, &format!("Complete {sid}"));
                        if r.clicked() {
                            pick = Some(k);
                        }
                    }
                });
        }
        if let (Some(k), Some((start, _, c))) = (pick, typed) {
            if let Some(&sid) = self.offered.get(k) {
                let from = byte_at(&self.text, start);
                let to = byte_at(&self.text, c);
                let n = sid.to_string();
                self.text.replace_range(from..to, &n);
                let mut state = out.state;
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(
                        start + n.chars().count(),
                    ))));
                state.store(ui.ctx(), id);
                ui.memory_mut(|m| m.request_focus(id));
                self.offered.clear();
                self.typed = None;
                ui.ctx().request_repaint();
            }
        }

        for f in self.flags(&source) {
            ui.colored_label(bad, &f.message);
        }
        opened
    }
}

/// The surface number of `text` at `at` (relative to its galley), and its
/// span (bytes).
fn surface_at(
    text: &str,
    galley: &egui::Galley,
    at: egui::Vec2,
) -> Option<(i64, std::ops::Range<usize>)> {
    if !galley.rect.contains(at.to_pos2()) {
        return None;
    }
    // The character under the pointer: the one before the cursor nearest
    // it when the pointer is left of that cursor.
    let c = galley.cursor_from_pos(at);
    let x = galley.pos_from_cursor(c).min.x;
    let i = c.index.0;
    let ch = if at.x < x { i.checked_sub(1)? } else { i };
    let b = byte_at(text, ch);
    if b >= text.len() {
        return None;
    }
    geometry_words(text).into_iter().find_map(|w| match w.kind {
        Word::Surface { id, .. } if w.span.contains(&b) => Some((id, w.span)),
        _ => None,
    })
}

/// The byte offset of character `c` of `text`.
fn byte_at(text: &str, c: usize) -> usize {
    text.char_indices().nth(c).map_or(text.len(), |(b, _)| b)
}

/// The surface number being typed just before character `cursor`: where
/// its digits start (a character index) and the digits. Not after `#` (a
/// cell) or `.` (a facet).
fn typed_number(text: &str, cursor: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    if cursor > chars.len() {
        return None;
    }
    if chars
        .get(cursor)
        .is_some_and(|c| c.is_ascii_digit() || *c == '.')
    {
        return None;
    }
    let mut start = cursor;
    while start > 0 && chars[start - 1].is_ascii_digit() {
        start -= 1;
    }
    if start == cursor {
        return None;
    }
    if start > 0 && matches!(chars[start - 1], '#' | '.') {
        return None;
    }
    if start > 0 && !(chars[start - 1].is_whitespace() || "()+-:".contains(chars[start - 1])) {
        return None;
    }
    Some((start, chars[start..cursor].iter().collect()))
}

/// Geometry text as words coloured as in cards, flagged words (if
/// `check`) in `colours[2]` and underlined.
fn coloured(
    text: &str,
    source: &Arc<Source>,
    colours: [egui::Color32; 3],
    wrap: f32,
    check: bool,
) -> LayoutJob {
    let [plain, strong, bad] = colours;
    let font = egui::FontId::monospace(12.5);
    let checked = if check {
        source.check_geometry(text)
    } else {
        Vec::new()
    };
    let flags: Vec<std::ops::Range<usize>> = checked
        .into_iter()
        .filter(|f| f.span.len() < text.len() || text.is_empty())
        .map(|f| f.span)
        .collect();
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let format = |colour: egui::Color32, flagged: bool| {
        let mut f = TextFormat::simple(font.clone(), if flagged { bad } else { colour });
        if flagged {
            f.underline = egui::Stroke::new(1.5, bad);
        }
        f
    };
    let mut at = 0;
    for w in geometry_words(text) {
        if w.span.start > at {
            job.append(&text[at..w.span.start], 0.0, format(plain, false));
        }
        let colour = match w.kind {
            Word::Surface { .. } => SURFACE,
            Word::Cell(_) | Word::Complement => COMPLEMENT,
            Word::Open | Word::Close | Word::Union => strong,
            Word::Bad => bad,
        };
        let flagged = flags
            .iter()
            .any(|f| f.start < w.span.end && w.span.start < f.end);
        job.append(&text[w.span.clone()], 0.0, format(colour, flagged));
        at = w.span.end;
    }
    if at < text.len() {
        job.append(&text[at..], 0.0, format(plain, false));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_being_typed() {
        assert_eq!(typed_number("-41", 3), Some((1, "41".into())));
        assert_eq!(typed_number("1 (41", 5), Some((3, "41".into())));
        assert_eq!(typed_number("1 #41", 5), None, "a cell");
        assert_eq!(typed_number("1 4.2", 5), None, "a facet");
        assert_eq!(typed_number("1 412", 4), None, "in the middle of one");
        assert_eq!(typed_number("1 ", 2), None);
        assert_eq!(typed_number("ab12", 4), None);
        assert!(!GeometryEditor::new("-1  2\n 3").changed());
        assert!(GeometryEditor {
            text: "-1 2 4".into(),
            ..GeometryEditor::new("-1 2 3")
        }
        .changed());
        assert_eq!(GeometryEditor::new("-1 2 (-1:3) #4").surfaces(), [1, 2, 3]);
    }

    #[test]
    fn the_surface_under_the_pointer() {
        let ctx = egui::Context::default();
        // The frame's textures are not uploaded: dropped on purpose.
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let text = "-301 302 (#4 : -303)";
        let galley = ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                text.into(),
                egui::FontId::monospace(12.5),
                egui::Color32::WHITE,
            )
        });
        // The middle of character `k`.
        let at = |k: usize| {
            let a = galley.pos_from_cursor(CCursor::new(k)).min.x;
            let b = galley.pos_from_cursor(CCursor::new(k + 1)).min.x;
            egui::vec2(0.5 * (a + b), 0.5 * galley.rect.height())
        };
        let id = |k: usize| surface_at(text, &galley, at(k)).map(|s| s.0);
        assert_eq!(id(0), Some(301), "its sign");
        assert_eq!(id(3), Some(301));
        assert_eq!(id(4), None, "a space");
        assert_eq!(id(5), Some(302));
        assert_eq!(id(11), None, "a cell");
        assert_eq!(id(17), Some(303));
        assert_eq!(id(19), None, "a parenthesis");
        assert_eq!(surface_at(text, &galley, egui::vec2(-5.0, 2.0)), None);
    }
}
