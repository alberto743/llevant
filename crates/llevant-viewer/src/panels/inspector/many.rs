//! Many cells edited at once (decision 0012, phase 8): the selection's
//! (or a rule's from Find) material, density (set or scaled), importances,
//! universe and other parameters, as one edit that one Undo takes back
//! ([`Command::EditCells`]); and their numbers changed, every reference to
//! them with them ([`Command::Renumber`]). What the cells have now is shown
//! beside each field: the values, and how many cells have each.
//!
//! Renumbering one surface, material, universe or cell is on its page
//! ([`renumber_one`]).

use std::collections::BTreeMap;
use std::sync::Arc;

use llevant_scene::source::{CellChange, CellFields, IdKind, HIGHEST_NUMBER};
use rayon::prelude::*;

use super::fields::{material_picker, name_text, param_hint, text_field, usual_density};
use crate::command::Command;
use crate::document::Document;
use crate::theme;

/// Values listed for a field (the most common first).
const LISTED: usize = 4;
/// Cards shown before Apply.
const SHOWN: usize = 3;
/// Parameters offered as rows at once.
const OFFERED: [&str; 3] = ["imp:n", "imp:p", "u"];

/// What is done to the density.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Density {
    Keep,
    /// As MCNP takes it (negative: g/cm³).
    Set(f64),
    Scale(f64),
}

/// What is done to a parameter.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    Keep,
    Set(String),
    Remove,
}

struct ParamRow {
    key: String,
    action: Action,
}

/// What the cells have now, read when the form opened.
#[derive(Default)]
struct Now {
    /// Material → cells (`None`: a `LIKE n BUT` cell).
    materials: BTreeMap<Option<i64>, usize>,
    /// Densities written (by bits) → cells.
    densities: BTreeMap<u64, usize>,
    /// Parameter → its values → cells; the cells without it are the rest.
    params: BTreeMap<String, BTreeMap<String, usize>>,
    /// Cells whose card is not in the input (in a READ file).
    unread: usize,
}

pub struct ManyForm {
    /// The rule the cells were selected by (the form closes when it changes).
    rule: String,
    /// Their numbers, in order.
    cells: Arc<Vec<i64>>,
    now: Now,
    material: Option<i64>,
    filter: String,
    density: Density,
    params: Vec<ParamRow>,
    new_key: String,
    /// Keys data cards give for every cell (not on cell cards).
    blocked: Vec<String>,
    /// Renumbering: the first number and the step.
    first: String,
    step: String,
    /// The model an edit was sent from: done when another is shown.
    applied: Option<usize>,
    error: Option<String>,
    /// The first cards as Apply would write them, for the changes they
    /// were made for: `(title, before, after)`.
    preview: Option<(String, Result<Vec<super::diff::Shown>, String>)>,
}

impl ManyForm {
    /// A form for the cells selected now (`None` when nothing is, or the
    /// cards are not read).
    pub fn open(doc: &Document) -> Option<ManyForm> {
        let source = doc.source.clone()?;
        let rule = doc.selection.query.as_ref()?.to_string();
        let mut cells: Vec<i64> = doc
            .selection
            .cells
            .iter()
            .map(|c| doc.model.cell_id(c))
            .collect();
        cells.sort_unstable();
        cells.dedup();
        if cells.is_empty() {
            return None;
        }
        let fields: Vec<Option<CellFields>> =
            cells.par_iter().map(|&id| source.cell_fields(id)).collect();
        let mut now = Now::default();
        let mut keys: Vec<String> = OFFERED.iter().map(|k| k.to_string()).collect();
        for f in &fields {
            let Some(f) = f else {
                now.unread += 1;
                continue;
            };
            *now.materials.entry(f.material).or_default() += 1;
            if let Some(d) = f.density {
                *now.densities.entry(d.to_bits()).or_default() += 1;
            }
            for p in &f.params {
                *now.params
                    .entry(p.key.clone())
                    .or_default()
                    .entry(p.value.clone())
                    .or_default() += 1;
                if !keys.contains(&p.key) && !p.key.contains("fill") && p.key != "lat" {
                    keys.push(p.key.clone());
                }
            }
        }
        // Free numbers: from the next thousand above the highest cell's.
        let highest = doc.model.cells().iter().map(|c| c.id).max().unwrap_or(0);
        let first = (highest / 1000 + 1) * 1000;
        Some(ManyForm {
            rule,
            cells: Arc::new(cells),
            now,
            material: None,
            filter: String::new(),
            density: Density::Keep,
            params: keys
                .into_iter()
                .map(|key| ParamRow {
                    key,
                    action: Action::Keep,
                })
                .collect(),
            new_key: String::new(),
            blocked: source.per_cell_keys(),
            first: first.to_string(),
            step: "1".into(),
            applied: None,
            error: None,
            preview: None,
        })
    }

    /// Whether it still stands for what is selected (and has not landed).
    pub fn current(&self, doc: &Document) -> bool {
        let rule = doc.selection.query.as_ref().map(|q| q.to_string());
        let landed = self
            .applied
            .is_some_and(|m| m != Arc::as_ptr(&doc.model) as usize);
        !landed && (rule.as_deref() == Some(self.rule.as_str()) || self.applied.is_some())
    }

    /// The last edit did not land: ready to try again.
    pub fn edit_failed(&mut self, why: &str) {
        if self.applied.take().is_some() {
            self.error = Some(why.to_string());
        }
    }

    /// The cells void now.
    fn void(&self) -> usize {
        self.now.materials.get(&Some(0)).copied().unwrap_or(0)
    }

    /// The changes made to every cell, in order; or why they cannot be.
    fn changes(&self) -> Result<Vec<CellChange>, String> {
        let mut v = Vec::new();
        let voided = self.material == Some(0);
        if let Some(m) = self.material {
            v.push(CellChange::Material(m));
        }
        match self.density {
            Density::Keep => {
                if self.material.is_some_and(|m| m != 0) && self.void() > 0 {
                    return Err(format!(
                        "{} of the cells are void: give the density they take",
                        self.void()
                    ));
                }
            }
            _ if voided => {}
            Density::Set(d) => v.push(CellChange::Density(d)),
            Density::Scale(k) if k > 0.0 && k != 1.0 => v.push(CellChange::ScaleDensity(k)),
            Density::Scale(_) => {}
        }
        for p in &self.params {
            match &p.action {
                Action::Keep => {}
                Action::Set(s) if s.trim().is_empty() => {
                    return Err(format!("{} needs a value", p.key.to_uppercase()))
                }
                Action::Set(s) => v.push(CellChange::Param {
                    key: p.key.clone(),
                    value: Some(s.trim().to_string()),
                }),
                Action::Remove => v.push(CellChange::Param {
                    key: p.key.clone(),
                    value: None,
                }),
            }
        }
        Ok(v)
    }

    /// The new numbers, `(old, new)` in order; or why they cannot be.
    fn renumbering(&self, doc: &Document) -> Result<Vec<(i64, i64)>, String> {
        let first: i64 = self
            .first
            .trim()
            .parse()
            .map_err(|_| "the first number is not a whole number")?;
        let step: i64 = self
            .step
            .trim()
            .parse()
            .ok()
            .filter(|s| *s >= 1)
            .ok_or("the step is not a whole number from 1")?;
        let last = first.saturating_add(step.saturating_mul(self.cells.len() as i64 - 1));
        if first < 1 || last > HIGHEST_NUMBER {
            return Err(format!("cell numbers go from 1 to {HIGHEST_NUMBER}"));
        }
        let pairs: Vec<(i64, i64)> = self
            .cells
            .iter()
            .enumerate()
            .map(|(k, &id)| (id, first + step * k as i64))
            .collect();
        if pairs.iter().all(|(a, b)| a == b) {
            return Err("they have these numbers".into());
        }
        if let Some((_, t)) = pairs
            .iter()
            .find(|(a, b)| a != b && doc.model.cell_index(*b).is_some())
        {
            return Err(format!(
                "cell {t} is in use (numbers being freed are not taken: pick free ones)"
            ));
        }
        Ok(pairs)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let n = self.cells.len();
        let pending = self.applied.is_some();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("EDIT {n} CELLS"))
                .size(11.5)
                .strong()
                .weak(),
        );
        if self.now.unread > 0 {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                format!(
                    "{} of them are not in the input (in a READ file?): the edit cannot be made",
                    self.now.unread
                ),
            );
        }
        ui.add_enabled_ui(!pending, |ui| {
            self.material_row(ui, doc);
            self.density_row(ui, doc);
            for k in 0..self.params.len() {
                self.param_row(ui, k);
            }
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.new_key)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("tmp, vol…")
                        .desired_width(90.0),
                );
                name_text(&r, "New parameter key of the cells");
                let key = self.new_key.trim().to_ascii_lowercase();
                let blocked = self.blocked.contains(&key);
                let known = self.params.iter().any(|p| p.key == key);
                let r = ui.add_enabled(
                    !key.is_empty() && !blocked && !known,
                    egui::Button::new("Add a parameter").small(),
                );
                let r = if blocked {
                    r.on_disabled_hover_text(format!(
                        "a data card gives {} for every cell",
                        key.to_uppercase()
                    ))
                } else {
                    r
                };
                if r.clicked() {
                    self.params.push(ParamRow {
                        key,
                        action: Action::Set(String::new()),
                    });
                    self.new_key.clear();
                }
            });
        });
        let changes = self.changes();
        let count = changes.as_ref().map_or(0, |c| c.len());
        // The first cards as Apply would write them.
        if let (Ok(c), false, Some(source)) = (&changes, pending, doc.source.as_ref()) {
            if !c.is_empty() {
                let key = format!("{c:?}");
                if self.preview.as_ref().is_none_or(|p| p.0 != key) {
                    let first = &self.cells[..self.cells.len().min(SHOWN)];
                    let cards = source.cells_with(first, c).map(|v| {
                        v.into_iter()
                            .map(|(_, id, after)| {
                                let before = source.cell(id).map(|c| c.text).unwrap_or_default();
                                (format!("cell {id}"), before, after)
                            })
                            .collect()
                    });
                    self.preview = Some((key, cards));
                }
                match &self.preview {
                    Some((_, Ok(cards))) => {
                        super::diff::cards(ui, cards, n.saturating_sub(cards.len()));
                        super::diff::after_apply(ui);
                    }
                    Some((_, Err(e))) => {
                        ui.colored_label(ui.visuals().warn_fg_color, e);
                    }
                    None => {}
                }
            }
        }
        ui.horizontal(|ui| {
            let ok = doc.input_editable.is_ok()
                && count > 0
                && !pending
                && self.now.unread == 0
                && changes.is_ok();
            let r = ui.add_enabled(ok, egui::Button::new(format!("Apply to {n} cells")));
            crate::app::frame::named(&r, "Apply to the cells");
            let r = match (&doc.input_editable, &changes) {
                (Err(why), _) => r.on_disabled_hover_text(*why),
                (_, Err(e)) => r.on_disabled_hover_text(e.as_str()),
                _ => r.on_hover_text(
                    "Write the fields changed into every card, compiled again, as one edit (not saved; one Undo takes it back)",
                ),
            };
            if r.clicked() {
                if let Ok(c) = changes.clone() {
                    out.push(Command::EditCells(self.cells.to_vec(), c));
                    self.applied = Some(Arc::as_ptr(&doc.model) as usize);
                    self.error = None;
                }
            }
            match &changes {
                Err(e) if count == 0 => {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                }
                Err(e) => {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                }
                Ok(c) if c.is_empty() => {
                    ui.label(egui::RichText::new("nothing changed yet").weak());
                }
                Ok(c) => {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {} · {n} cards",
                            c.len(),
                            if c.len() == 1 { "field" } else { "fields" }
                        ))
                        .weak(),
                    );
                }
            }
        });
        self.renumber_ui(ui, doc, out);
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if pending {
            ui.horizontal(|ui| {
                crate::logo::small_spinner(ui);
                ui.label(egui::RichText::new("writing the cards and compiling…").weak());
            });
        }
    }

    fn material_row(&mut self, ui: &mut egui::Ui, doc: &Document) {
        ui.horizontal(|ui| {
            field_name(ui, "Material");
            let mut keep = self.material.is_none();
            if choice(ui, keep, "keep", "Material: keep") {
                keep = true;
                self.material = None;
            }
            if keep {
                if choice(ui, false, "set", "Material: set") {
                    self.material = Some(self.most_common_material().unwrap_or(0));
                }
            } else {
                let mut m = self.material.unwrap_or(0);
                if material_picker(ui, doc, "many material", &mut m, &mut self.filter) {
                    self.material = Some(m);
                    if m != 0 && self.void() > 0 && self.density == Density::Keep {
                        if let Some(d) = usual_density(doc, m) {
                            self.density = Density::Set(d);
                        }
                    }
                }
            }
        });
        let listed: Vec<String> = largest(&self.now.materials)
            .into_iter()
            .map(|(m, c)| match m {
                Some(0) => format!("void ×{c}"),
                Some(m) => format!("m{m} ×{c}"),
                None => format!("LIKE ×{c}"),
            })
            .collect();
        now_label(ui, &listed, self.now.materials.len());
    }

    fn most_common_material(&self) -> Option<i64> {
        largest(&self.now.materials).first().and_then(|e| e.0)
    }

    fn density_row(&mut self, ui: &mut egui::Ui, _doc: &Document) {
        ui.add_enabled_ui(self.material != Some(0), |ui| {
            ui.horizontal(|ui| {
                field_name(ui, "Density");
                let common = largest(&self.now.densities)
                    .first()
                    .map_or(-1.0, |e| f64::from_bits(e.0));
                for (label, d) in [
                    ("keep", Density::Keep),
                    ("set", Density::Set(common)),
                    ("scale", Density::Scale(1.0)),
                ] {
                    let on = std::mem::discriminant(&self.density) == std::mem::discriminant(&d);
                    if choice(ui, on, label, &format!("Density: {label}")) && !on {
                        self.density = d;
                    }
                }
                match &mut self.density {
                    Density::Keep => {}
                    Density::Set(d) => {
                        let grams = *d < 0.0;
                        let mut v = d.abs();
                        let speed = (v * 0.002).max(1e-4);
                        let r = ui.add(
                            egui::DragValue::new(&mut v)
                                .speed(speed)
                                .range(0.0..=f64::INFINITY)
                                .max_decimals(6),
                        );
                        if r.changed() && v > 0.0 {
                            *d = if grams { -v } else { v };
                        }
                        for (g, name) in [(true, "g/cm³"), (false, "at/b·cm")] {
                            if ui
                                .add(egui::Button::selectable(grams == g, name).small())
                                .clicked()
                            {
                                *d = if g { -v } else { v };
                            }
                        }
                    }
                    Density::Scale(k) => {
                        ui.label("×");
                        ui.add(
                            egui::DragValue::new(k)
                                .speed(0.001)
                                .range(1e-6..=1e6)
                                .max_decimals(6),
                        )
                        .on_hover_text(
                            "Each cell's density times this, in the unit it is written in",
                        );
                    }
                }
            });
        });
        let listed: Vec<String> = largest(&self.now.densities)
            .into_iter()
            .map(|(b, c)| {
                let d = f64::from_bits(b);
                if d < 0.0 {
                    format!("{} g/cm³ ×{c}", -d)
                } else {
                    format!("{d:e} at/b·cm ×{c}")
                }
            })
            .collect();
        now_label(ui, &listed, self.now.densities.len());
    }

    fn param_row(&mut self, ui: &mut egui::Ui, k: usize) {
        let n = self.cells.len() - self.now.unread;
        let key = self.params[k].key.clone();
        let label = key.to_uppercase();
        let blocked = self.blocked.contains(&key);
        ui.add_enabled_ui(!blocked, |ui| {
            ui.horizontal(|ui| {
                field_name(ui, &label).on_hover_text(param_hint(&key));
                let row = &mut self.params[k];
                let on =
                    |a: &Action, b: &Action| std::mem::discriminant(a) == std::mem::discriminant(b);
                for (name, a) in [
                    ("keep", Action::Keep),
                    ("set", Action::Set(String::new())),
                    ("remove", Action::Remove),
                ] {
                    if choice(ui, on(&row.action, &a), name, &format!("{label}: {name}"))
                        && !on(&row.action, &a)
                    {
                        row.action = a;
                    }
                }
                if let Action::Set(v) = &mut row.action {
                    let r = text_field(ui, true, v, 90.0);
                    name_text(&r, &format!("{label} of the cells"));
                }
            });
        })
        .response
        .on_disabled_hover_text(format!("a data card gives {label} for every cell"));
        let values = self.now.params.get(&key);
        let with: usize = values.map_or(0, |v| v.values().sum());
        let mut listed: Vec<String> = values
            .map(|v| {
                largest(v)
                    .into_iter()
                    .map(|(s, c)| format!("{s} ×{c}"))
                    .collect()
            })
            .unwrap_or_default();
        if with < n {
            listed.push(format!("none ×{}", n - with));
        }
        now_label(
            ui,
            &listed,
            values.map_or(0, |v| v.len()) + usize::from(with < n),
        );
    }

    fn renumber_ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let n = self.cells.len();
        let pending = self.applied.is_some();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Number them from");
            let r = text_field(ui, !pending, &mut self.first, 80.0);
            name_text(&r, "First number");
            ui.label("by");
            let r = text_field(ui, !pending, &mut self.step, 40.0);
            name_text(&r, "Step");
        });
        let pairs = self.renumbering(doc);
        ui.horizontal_wrapped(|ui| {
            let ok = doc.input_editable.is_ok() && pairs.is_ok() && !pending;
            let r = ui.add_enabled(ok, egui::Button::new("Renumber"));
            crate::app::frame::named(&r, "Renumber the cells");
            let r = match (&doc.input_editable, &pairs) {
                (Err(why), _) => r.on_disabled_hover_text(*why),
                (_, Err(e)) => r.on_disabled_hover_text(e.as_str()),
                _ => r.on_hover_text(
                    "Give the cells these numbers in order, with every reference to them (#n, LIKE n, tallies…), as one edit",
                ),
            };
            if r.clicked() {
                if let Ok(p) = pairs.clone() {
                    out.push(Command::Renumber(IdKind::Cell, p));
                    self.applied = Some(Arc::as_ptr(&doc.model) as usize);
                    self.error = None;
                }
            }
            match &pairs {
                Ok(p) => {
                    let shown: Vec<String> = p
                        .iter()
                        .take(3)
                        .map(|(a, b)| format!("{a} → {b}"))
                        .collect();
                    let more = if n > 3 {
                        format!(", … {} → {}", p[n - 1].0, p[n - 1].1)
                    } else {
                        String::new()
                    };
                    ui.label(theme::data(format!("{}{more}", shown.join(", "))).weak());
                }
                Err(e) => {
                    ui.label(egui::RichText::new(e.as_str()).weak());
                }
            }
        });
    }
}

/// One of a field's choices (named `name` for the app's tests); true if
/// clicked.
fn choice(ui: &mut egui::Ui, on: bool, text: &str, name: &str) -> bool {
    let r = ui.selectable_label(on, text);
    crate::app::frame::named(&r, name);
    r.clicked()
}

/// Entries by how many cells have them, most first.
fn largest<K: Clone + Ord>(m: &BTreeMap<K, usize>) -> Vec<(K, usize)> {
    let mut v: Vec<(K, usize)> = m.iter().map(|(k, c)| (k.clone(), *c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// What the cells have now, the most common first.
fn now_label(ui: &mut egui::Ui, listed: &[String], distinct: usize) {
    let mut text = listed
        .iter()
        .take(LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if distinct > LISTED {
        text.push_str(&format!(", +{} more", distinct - LISTED));
    }
    ui.horizontal(|ui| {
        ui.add_space(FIELD_NAME + ui.spacing().item_spacing.x);
        ui.add(egui::Label::new(theme::data(format!("now {text}")).weak()).truncate())
            .on_hover_text(text);
    });
    ui.add_space(2.0);
}

/// Width of a field's name, before its choices.
const FIELD_NAME: f32 = 64.0;

/// A field's name, in a column of its own.
fn field_name(ui: &mut egui::Ui, name: &str) -> egui::Response {
    let h = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(
        egui::vec2(FIELD_NAME, h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(FIELD_NAME);
            ui.add(egui::Label::new(name).truncate())
        },
    )
    .inner
}

/// A new number for `kind` `id` (its definition and every reference),
/// on the page of a cell, surface, material or universe.
pub fn renumber_one(
    ui: &mut egui::Ui,
    doc: &Document,
    kind: IdKind,
    id: i64,
    out: &mut Vec<Command>,
) {
    let key = egui::Id::new(("renumber", format!("{kind:?}"), id));
    let mut text: String = ui
        .data_mut(|d| d.get_temp::<String>(key))
        .unwrap_or_else(|| id.to_string());
    let name = match kind {
        IdKind::Cell => "cell",
        IdKind::Surface => "surface",
        IdKind::Material => "material",
        IdKind::Universe => "universe",
        IdKind::Transform => "transform",
        IdKind::Tally => "tally",
    };
    let free = |n: i64| match kind {
        IdKind::Cell => doc.model.cell_index(n).is_none(),
        IdKind::Universe => !doc.model.units().iter().any(|u| u.universe == n),
        _ => doc.source.as_ref().is_none_or(|s| {
            let k = match kind {
                IdKind::Surface => llevant_scene::source::CardKind::Surface,
                IdKind::Material => llevant_scene::source::CardKind::Material,
                _ => llevant_scene::source::CardKind::Transform,
            };
            s.card(k, n).is_none()
        }),
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Number").weak());
        let r = text_field(ui, doc.input_editable.is_ok(), &mut text, 80.0);
        name_text(&r, &format!("New number of {name} {id}"));
        let wanted = text.trim().parse::<i64>().ok();
        let why = match wanted {
            None => Some("not a whole number".to_string()),
            Some(n) if n == id => None,
            Some(n)
                if !((1..=HIGHEST_NUMBER).contains(&n) || (kind == IdKind::Material && n == 0)) =>
            {
                Some(format!("numbers go from 1 to {HIGHEST_NUMBER}"))
            }
            Some(n) if !free(n) => Some(format!("{name} {n} is in use")),
            Some(_) => None,
        };
        let changed = wanted.is_some_and(|n| n != id);
        if changed || why.is_some() {
            let ok = why.is_none() && doc.input_editable.is_ok();
            let r = ui.add_enabled(ok, egui::Button::new("Renumber").small());
            crate::app::frame::named(&r, &format!("Renumber {name} {id}"));
            let r = match &why {
                Some(w) => r.on_disabled_hover_text(w.as_str()),
                None => r.on_hover_text(format!(
                    "Give {name} {id} this number, with every reference to it, as one edit"
                )),
            };
            if r.clicked() {
                if let Some(n) = wanted {
                    out.push(Command::Renumber(kind, vec![(id, n)]));
                }
            }
            if let Some(w) = why {
                ui.label(egui::RichText::new(w).weak());
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(key, text));
}
