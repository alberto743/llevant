//! The inspector (decision 0012): what is picked, explained, on the right of
//! every workspace. Its first page is what is picked: the selection, the
//! point double-clicked and its cell, in sections (its fields, place,
//! size, its card). Every material, surface, universe and
//! cell named there opens a page of its own (`›`, or a click on it in a
//! card), with back and forward as in a browser; a new pick starts again
//! from its page.
//!
//! Names are links as everywhere ([`crate::context`]): a click selects,
//! Ctrl+click adds or removes, a double-click frames, a right-click opens
//! the menu.

mod card;
mod cell;
mod cut;
mod diff;
pub(crate) mod fields;
mod geometry;
mod many;
mod material;
mod mesh;
mod new;
mod picked;
mod surface;
pub mod tokens;
pub use mesh::covered;
mod universe;

use crate::command::Command;
use crate::document::Document;
use crate::{icons, theme};

/// What a page of the inspector is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    /// What is picked: the selection, the point and its cell.
    Picked,
    /// MCNP numbers.
    Cell(i64),
    Material(i64),
    Surface(i64),
    Universe(i64),
    /// A new card of this kind.
    New(llevant_scene::source::CardKind),
}

impl Subject {
    pub fn title(self) -> String {
        match self {
            Subject::Picked => "picked".into(),
            Subject::Cell(id) => format!("c {id}"),
            Subject::Material(0) => "void".into(),
            Subject::Material(id) => format!("m {id}"),
            Subject::Surface(id) => format!("s {id}"),
            Subject::Universe(u) => format!("u={u}"),
            Subject::New(k) => format!("new {}", k.name()),
        }
    }
}

/// What identifies a pick: the point double-clicked and the selection's
/// rule. A new one takes the inspector back to its page.
type PickKey = (Option<[u64; 3]>, Option<String>);

/// The inspector's pages visited and what is cached for them.
#[derive(Default)]
pub struct Inspector {
    history: Vec<Subject>,
    at: usize,
    pick: Option<PickKey>,
    /// Cells using a surface (one pass over every cell's program): for the
    /// surface, in the model, shown last; and, for a universe opened on its
    /// own, the numbers of those of the whole model outside it.
    users: Option<(i64, usize, Vec<u32>, Vec<i64>)>,
    /// The cells the mesh selected covers: for that mesh, in the model.
    covers: Option<(crate::mesh::MeshState, usize, Vec<u32>)>,
    /// The fields of the card shown, and drafts of others.
    forms: fields::Forms,
    /// The cells selected being edited together.
    many: Option<many::ManyForm>,
    /// Open that form for the selection when it is next shown.
    many_wanted: bool,
    /// A new card being made.
    new: Option<new::NewForm>,
}

impl Inspector {
    /// Whether there is anything to show.
    pub fn has_content(&self, doc: &Document) -> bool {
        cut::has_content(doc)
            || self.current() != Subject::Picked
            || picked::has_content(doc)
            || doc.editing.mesh_selected.is_some()
    }

    /// The surfaces to mark on the slice: those of the geometry being
    /// edited, if its form was shown in pass `pass`.
    pub fn marks(&self, pass: u64) -> Option<crate::marks::MarkRequest> {
        self.forms
            .marks(pass)
            .or_else(|| self.new.as_ref().and_then(|f| f.marks(pass)))
    }

    /// The last edit sent did not land: the form open again to retry.
    pub fn edit_failed(&mut self, why: &str) {
        self.forms.edit_failed(why);
        if let Some(m) = &mut self.many {
            m.edit_failed(why);
        }
        if let Some(n) = &mut self.new {
            n.edit_failed(why);
        }
    }

    /// Edit the cells selected together (the selection just made, from
    /// Find): the form opens on the inspector's first page.
    pub fn edit_selection(&mut self) {
        self.many_wanted = true;
    }

    fn current(&self) -> Subject {
        self.history
            .get(self.at)
            .copied()
            .unwrap_or(Subject::Picked)
    }

    /// Open a page of `s`, after the one shown (dropping those ahead).
    pub fn open(&mut self, s: Subject) {
        if self.history.is_empty() {
            self.history.push(Subject::Picked);
        }
        if self.current() == s {
            return;
        }
        self.history.truncate(self.at + 1);
        self.history.push(s);
        self.at = self.history.len() - 1;
    }

    /// Back to what is picked when the pick changes (the first pick seen is
    /// only recorded: a page asked for before it stays).
    fn follow(&mut self, doc: &Document) {
        let key: PickKey = (
            doc.focus.as_ref().map(|f| f.point.map(f64::to_bits)),
            doc.selection.query.as_ref().map(|q| q.to_string()),
        );
        if self.pick.is_none() {
            self.pick = Some(key);
        } else if self.pick.as_ref() != Some(&key) {
            self.pick = Some(key);
            self.history = vec![Subject::Picked];
            self.at = 0;
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        self.follow(doc);
        if self.many.as_ref().is_some_and(|m| !m.current(doc)) {
            self.many = None;
        }
        if std::mem::take(&mut self.many_wanted) {
            self.many = many::ManyForm::open(doc);
            self.history = vec![Subject::Picked];
            self.at = 0;
        }
        if self.history.len() > 1 {
            self.nav(ui);
            ui.separator();
        }
        // A surface moved and not written, while another page is shown:
        // where to apply it.
        if let Some(d) = doc.drag.as_ref().filter(|d| !d.committing) {
            let pass = ui.ctx().cumulative_pass_nr();
            let scrubbed = ui
                .data(|m| m.get_temp::<(i64, u64)>(egui::Id::new("surface scrub shown")))
                .is_some_and(|(s, p)| s == d.surface && p + 1 >= pass);
            if !scrubbed {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("surface {} moved, not written", d.surface),
                    );
                    if ui.small_button("Show").clicked() {
                        self.open(Subject::Surface(d.surface));
                    }
                    if ui.small_button("Cancel").clicked() {
                        out.push(Command::EndDrag { commit: false });
                    }
                });
                ui.separator();
            }
        }
        cut::section(ui, doc, out);
        let mut go = None;
        self.forms.strip(ui, doc, &mut go);
        match self.current() {
            Subject::Picked => {
                picked::page(ui, doc, &mut self.forms, &mut self.many, &mut go, out);
                if let Some(k) = doc.editing.mesh_selected {
                    let covers = self.covers(doc, k);
                    mesh::section_of(ui, doc, k, &covers, out);
                }
            }
            Subject::Cell(id) => match doc.model.cell_index(id) {
                Some(c) => cell::page(ui, doc, c, &mut self.forms, &mut go, out),
                None => missing(ui, "cell", id),
            },
            Subject::Material(id) => material::page(ui, doc, id, &mut self.forms, &mut go, out),
            Subject::Surface(id) => {
                let (users, outside) = self.users(doc, id);
                surface::page(ui, doc, id, users, outside, &mut go, out)
            }
            Subject::Universe(u) => universe::page(ui, doc, u, &mut go, out),
            Subject::New(kind) => {
                if self
                    .new
                    .as_ref()
                    .is_none_or(|f| f.kind() != kind || !f.current(doc))
                {
                    self.new = Some(new::NewForm::new(doc, kind));
                }
                if let Some(f) = &mut self.new {
                    f.ui(ui, doc, out);
                }
            }
        }
        if let Some(s) = go {
            self.open(s);
        }
    }

    /// Back, forward, and the pages visited.
    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let back = ui
                .add_enabled(self.at > 0, egui::Button::new(icons::BACK).small())
                .on_hover_text("Back");
            crate::app::frame::named(&back, "Inspector back");
            if back.clicked() {
                self.at -= 1;
            }
            let fwd = ui
                .add_enabled(
                    self.at + 1 < self.history.len(),
                    egui::Button::new(icons::FORWARD).small(),
                )
                .on_hover_text("Forward");
            crate::app::frame::named(&fwd, "Inspector forward");
            if fwd.clicked() {
                self.at += 1;
            }
            for (k, s) in self.history.iter().enumerate() {
                if k > 0 {
                    ui.label(egui::RichText::new("›").weak());
                }
                let text = theme::data(s.title());
                let text = if k == self.at {
                    text.strong()
                } else {
                    text.weak()
                };
                if ui
                    .add(egui::Button::new(text).frame(false).small())
                    .clicked()
                {
                    self.at = k;
                }
            }
        });
    }

    /// The cells mesh `k` covers, computed once per mesh (as it is) and
    /// model.
    fn covers(&mut self, doc: &Document, k: usize) -> Vec<u32> {
        let Some(m) = doc.scene.meshes.get(k) else {
            return Vec::new();
        };
        let model = std::sync::Arc::as_ptr(&doc.model) as usize;
        let fresh = matches!(&self.covers, Some((s, p, _)) if s == m && *p == model);
        if !fresh {
            self.covers = Some((m.clone(), model, mesh::covered(doc, m)));
        }
        self.covers.as_ref().map_or_else(Vec::new, |c| c.2.clone())
    }

    /// The cells using surface `id`, computed once per surface and model.
    fn users(&mut self, doc: &Document, id: i64) -> (&[u32], &[i64]) {
        let model = std::sync::Arc::as_ptr(&doc.model) as usize;
        let fresh = matches!(&self.users, Some((s, m, _, _)) if *s == id && *m == model);
        if !fresh {
            let cells = doc.index.surface_cells(&doc.model, &[id]);
            let outside = match &doc.whole {
                Some((w, x)) => doc.outside(x.surface_cells(w, &[id])),
                None => Vec::new(),
            };
            self.users = Some((id, model, cells, outside));
        }
        self.users
            .as_ref()
            .map_or((&[], &[]), |(_, _, v, o)| (v.as_slice(), o.as_slice()))
    }
}

fn missing(ui: &mut egui::Ui, what: &str, id: i64) {
    ui.label(egui::RichText::new(format!("No {what} {id} in the model shown.")).weak());
}

/// A page's title: the name, large, and the comment of its card.
fn title(ui: &mut egui::Ui, name: &str, swatch: Option<egui::Color32>, comment: Option<String>) {
    ui.horizontal(|ui| {
        if let Some(c) = swatch {
            let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 2.0, c);
        }
        ui.label(
            egui::RichText::new(name)
                .font(egui::FontId::monospace(17.0))
                .strong(),
        );
    });
    if let Some(c) = comment {
        ui.label(egui::RichText::new(c).weak());
    }
}

/// A section of a page: folded or open at first (egui remembers it).
fn section<R>(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    open: bool,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let text = egui::RichText::new(title.to_uppercase())
        .size(11.5)
        .strong()
        .color(ui.visuals().weak_text_color());
    egui::CollapsingHeader::new(text)
        .id_salt(salt)
        .default_open(open)
        .show(ui, add)
        .body_returned
}

/// For a universe opened on its own: a warning that `what` is also used by
/// the cells `outside` it (MCNP numbers), which an edit changes too.
fn shared_outside(ui: &mut egui::Ui, doc: &Document, what: &str, outside: &[i64]) {
    let (Some(u), false) = (doc.universe, outside.is_empty()) else {
        return;
    };
    let n = outside.len();
    let mut ids: Vec<String> = outside.iter().take(8).map(|c| c.to_string()).collect();
    if n > 8 {
        ids.push(format!("+{} more", n - 8));
    }
    ui.colored_label(
        ui.visuals().warn_fg_color,
        format!(
            "{} also used by {n} cell{} outside u={u} ({}): editing it changes them too",
            what,
            if n == 1 { "" } else { "s" },
            ids.join(" ")
        ),
    )
    .on_hover_text(
        "Surfaces and materials are shared by every cell that names them, in any universe",
    );
}

/// The `›` that opens `s`'s page.
fn drill(ui: &mut egui::Ui, s: Subject, go: &mut Option<Subject>) {
    let r = ui
        .add(egui::Button::new(icons::CLOSED).small().frame(false))
        .on_hover_text(format!("Inspect {}", s.title()));
    crate::app::frame::named(&r, &format!("Inspect {}", s.title()));
    if r.clicked() {
        *go = Some(s);
    }
}

/// The comment on a card's first line (`$ …`), or on the line before it.
fn comment(doc: &Document, kind: llevant_scene::source::CardKind, id: i64) -> Option<String> {
    let card = doc.source.as_ref()?.card(kind, id)?;
    let first = card.text.lines().next()?;
    let (_, c) = first.split_once('$')?;
    let c = c.trim();
    (!c.is_empty()).then(|| c.to_string())
}

/// A density as MCNP gives it (negative: g/cm³, positive: atoms/b·cm), and
/// in g/cm³ when the atomic mass is known.
fn density(d: f64, atomic_mass: Option<f64>) -> String {
    if d < 0.0 {
        format!("{} g/cm³", -d)
    } else {
        match atomic_mass.and_then(|a| llevant_scene::materials::grams_per_cm3(d, a)) {
            Some(g) => format!("{d} at/b·cm · {g:.4} g/cm³"),
            None => format!("{d} at/b·cm"),
        }
    }
}

/// The mean atomic mass of material `id` (from its look), if known.
fn atomic_mass(doc: &Document, id: i64) -> Option<f64> {
    doc.looks
        .materials
        .iter()
        .find(|l| l.id == id)
        .map(|l| l.mix.atomic_mass)
        .filter(|&a| a > 0.0)
}

/// A mass, in the unit that reads best.
fn mass(g: f64) -> String {
    if g >= 1e6 {
        format!("{:.4} t", g / 1e6)
    } else if g >= 1e3 {
        format!("{:.4} kg", g / 1e3)
    } else {
        format!("{g:.4} g")
    }
}
