//! Cards as written in the input: shown as words coloured by what they
//! name, each surface, cell, material and universe opening its page; edited
//! as text (Apply writes the input, not saved, and compiles again); a
//! surface's coefficients as fields to drag, the views following live.

use llevant_scene::source::CardKind;

use super::tokens::{tokens, Kind};
use super::Subject;
use crate::command::Command;
use crate::document::Document;
use crate::{icons, theme};

/// Colours of the words of a card, beyond the text's.
pub(super) const SURFACE: egui::Color32 = egui::Color32::from_rgb(96, 160, 240);
pub(super) const COMPLEMENT: egui::Color32 = egui::Color32::from_rgb(232, 140, 60);
const UNIVERSE: egui::Color32 = egui::Color32::from_rgb(176, 130, 230);
const MATERIAL: egui::Color32 = egui::Color32::from_rgb(110, 190, 120);

/// The card's text as coloured words; a click on one that names something
/// opens its page.
fn words(ui: &mut egui::Ui, doc: &Document, kind: CardKind, text: &str, go: &mut Option<Subject>) {
    let weak = ui.visuals().weak_text_color();
    let strong = ui.visuals().strong_text_color();
    let source = doc.source.as_deref();
    for line in tokens(kind, text) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for p in line {
                let (colour, subject, tip) = match p.kind {
                    Kind::Plain | Kind::Value | Kind::Density => (None, None, None),
                    Kind::Comment | Kind::Keyword => (Some(weak), None, None),
                    Kind::Name | Kind::Operator => (Some(strong), None, None),
                    Kind::Nuclide => (Some(MATERIAL), None, None),
                    Kind::Material(0) => (Some(weak), None, Some("void".to_string())),
                    Kind::Material(m) => (
                        Some(MATERIAL),
                        Some(Subject::Material(m)),
                        Some(format!("material {m}")),
                    ),
                    Kind::Surface { id, negative } => {
                        let kind = source
                            .and_then(|s| s.surface_coeffs(id))
                            .map_or(String::new(), |(k, _)| format!(" ({})", k.to_uppercase()));
                        let side = if negative {
                            "inside (−)"
                        } else {
                            "outside (+)"
                        };
                        (
                            Some(SURFACE),
                            Some(Subject::Surface(id)),
                            Some(format!("surface {id}{kind}: the cell is on its {side}")),
                        )
                    }
                    Kind::Cell(c) => (
                        Some(COMPLEMENT),
                        Some(Subject::Cell(c)),
                        Some(format!("cell {c}")),
                    ),
                    Kind::Universe(u) => (
                        Some(UNIVERSE),
                        Some(Subject::Universe(u)),
                        Some(format!("universe {u}")),
                    ),
                    Kind::Transform(t) => (
                        Some(theme::PIN),
                        None,
                        Some(format!("transform TR{t} (its card is below)")),
                    ),
                };
                let mut rt = theme::data(&p.text);
                if let Some(c) = colour {
                    rt = rt.color(c);
                }
                if p.kind == Kind::Comment {
                    rt = rt.italics();
                }
                match subject {
                    Some(s) => {
                        let r = ui.add(egui::Label::new(rt).sense(egui::Sense::click()));
                        let r = r.on_hover_text(format!(
                            "{}: click to inspect",
                            tip.unwrap_or_default()
                        ));
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if r.clicked() {
                            *go = Some(s);
                        }
                    }
                    None => {
                        let r = ui.label(rt);
                        if let Some(t) = tip {
                            r.on_hover_text(t);
                        }
                    }
                }
            }
        });
    }
}

/// The cards of the `TRn` transforms the card of `kind` `id` names (as
/// [`card`], editable).
pub fn transform_cards(
    ui: &mut egui::Ui,
    doc: &Document,
    kind: CardKind,
    id: i64,
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) {
    let Some(s) = &doc.source else { return };
    for n in s.transforms_of(kind, id) {
        card(ui, doc, CardKind::Transform, n, true, go, out);
    }
}

/// The card of `kind` `id` as written in the input, its words coloured and
/// linked; Edit makes it text to change, and Apply writes it into the input
/// (not saved) and compiles again.
pub fn card(
    ui: &mut egui::Ui,
    doc: &Document,
    kind: CardKind,
    id: i64,
    open: bool,
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) {
    let what = kind.name();
    let card = doc.source.as_ref().and_then(|s| s.card(kind, id));
    let card = card.as_ref();
    let title = match card {
        Some(c) => format!("{what} {id} card · line {}", c.line),
        None if doc.source.is_none() => format!("{what} {id} card (reading the input…)"),
        None => format!("{what} {id} card (not in the input)"),
    };
    // The text being edited, and whether it was applied (closed once the
    // card reads so).
    let key = egui::Id::new(("card edit", what, id));
    let mut editing: Option<(String, bool)> = ui.data(|d| d.get_temp(key));
    if let (Some((text, true)), Some(c)) = (&editing, card) {
        if text.trim_end() == c.text.trim_end() {
            editing = None;
        }
    }
    let header = egui::CollapsingHeader::new(egui::RichText::new(title).weak())
        .id_salt((what, id, "card"))
        .default_open(open);
    let header = if editing.is_some() {
        header.open(Some(true))
    } else {
        header
    };
    header.show(ui, |ui| {
        let Some(c) = card else { return };
        let text = c.text.trim_end();
        match &mut editing {
            None => {
                egui::ScrollArea::vertical()
                    .id_salt((what, id, "card text"))
                    .max_height(220.0)
                    .show(ui, |ui| words(ui, doc, kind, text, go));
                ui.horizontal(|ui| {
                    if ui.small_button(icons::with(icons::COPY, "Copy")).clicked() {
                        ui.ctx().copy_text(text.to_string());
                    }
                    let r = ui.add_enabled(
                        doc.input_editable.is_ok(),
                        egui::Button::new("Edit card").small(),
                    );
                    let r = match doc.input_editable {
                        Ok(()) => r.on_hover_text(
                            "Change the card's text: Apply compiles the input again (only what changed), not saved until you save it",
                        ),
                        Err(why) => r.on_disabled_hover_text(why),
                    };
                    if r.clicked() {
                        editing = Some((text.to_string(), false));
                    }
                });
            }
            Some((buffer, applied)) => {
                let rows = buffer.lines().count().clamp(2, 12);
                let r = ui.add(
                    egui::TextEdit::multiline(buffer)
                        .id_salt((what, id, "card editor"))
                        .code_editor()
                        .desired_rows(rows)
                        .desired_width(f64::INFINITY as f32),
                );
                super::fields::name_text(&r, "Card text");
                if r.changed() {
                    *applied = false;
                }
                let long = buffer
                    .lines()
                    .filter(|l| llevant_scene::source::lines::columns(l) > llevant_scene::source::MAX_COLUMNS)
                    .count();
                if long > 0 {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!(
                            "{long} line{} over {} columns",
                            if long == 1 { "" } else { "s" },
                            llevant_scene::source::MAX_COLUMNS
                        ),
                    );
                }
                if buffer.trim_end() != text.trim_end() && !*applied {
                    super::diff::cards(ui, &[(format!("{what} {id}"), text.to_string(), buffer.clone())], 0);
                    super::diff::after_apply(ui);
                }
                let mut close = false;
                ui.horizontal(|ui| {
                    let changed = buffer.trim_end() != text;
                    let ok = doc.input_editable.is_ok() && changed && !*applied && long == 0;
                    let r = ui.add_enabled(ok, egui::Button::new("Apply").small());
                    let r = match doc.input_editable {
                        Err(why) => r.on_disabled_hover_text(why),
                        Ok(()) if *applied => r.on_disabled_hover_text("being applied"),
                        Ok(()) => r.on_hover_text("Write it into the input and compile again"),
                    };
                    if r.clicked() {
                        out.push(Command::EditCard(kind, id, buffer.trim_end().to_string()));
                        *applied = true;
                    }
                    if ui.small_button("Cancel").clicked() {
                        close = true;
                    }
                });
                if close {
                    editing = None;
                }
            }
        }
    });
    ui.data_mut(|d| match editing {
        Some(e) => {
            d.insert_temp(key, e);
        }
        None => d.remove::<(String, bool)>(key),
    });
}

/// Surface `id`'s coefficients as fields to drag (or type): the view
/// follows live where it can ([`Command::DragSurface`]); the card as it
/// would be written is shown, and Apply writes and compiles it
/// ([`Command::EndDrag`]); Cancel, or Esc while dragging, takes it back.
pub fn scrub(ui: &mut egui::Ui, doc: &Document, id: i64, out: &mut Vec<Command>) {
    let Some((kind, written)) = doc.source.as_ref().and_then(|s| s.surface_coeffs(id)) else {
        return;
    };
    let drag = doc.drag.as_ref();
    let here = drag.filter(|d| d.surface == id);
    let coeffs = here.map_or(&written, |d| &d.coeffs);
    if coeffs.len() != written.len() {
        return;
    }
    // Esc pressed during this drag: the pointer is ignored until released.
    let key = egui::Id::new(("surface scrub cancelled", id));
    let pointer_down = ui.input(|i| i.pointer.any_down());
    let mut cancelled: bool = ui.data(|d| d.get_temp(key)).unwrap_or(false) && pointer_down;
    let held = here.is_some_and(|d| !d.committing);
    let enabled = (doc.input_editable.is_ok() || held)
        && drag.is_none_or(|d| d.surface == id && !d.committing)
        && !cancelled;
    let weak = |s: String| egui::RichText::new(s).weak();
    let mut next = coeffs.clone();
    let mut changed = false;
    let mut dragging = false;
    // Named as the type's coefficients are (x₀, R…), the position on hover.
    let names = llevant_scene::source::coefficient_names(&kind, next.len());
    let n = next.len();
    let upper = kind.to_uppercase();
    egui::Grid::new(("surface coefficients grid", id))
        .num_columns(4)
        .spacing([6.0, 3.0])
        .show(ui, |ui| {
            for (k, v) in next.iter_mut().enumerate() {
                let name = names
                    .get(k)
                    .map_or_else(|| format!("#{}", k + 1), |s| s.to_string());
                ui.label(theme::data(&name).weak());
                let speed = (v.abs() * 0.002).max(0.005);
                let r = ui.add_enabled(
                    enabled,
                    egui::DragValue::new(v)
                        .speed(speed)
                        .max_decimals(6)
                        .update_while_editing(false),
                );
                let r = match doc.input_editable {
                    Err(why) => r.on_disabled_hover_text(why),
                    Ok(()) => r.on_hover_text(format!(
                        "{name}: coefficient {} of {n} of {upper}. Drag to move the surface (the view follows), or click to type; written into the input by Apply, Esc while dragging takes it back",
                        k + 1
                    )),
                };
                changed |= r.changed();
                dragging |= r.dragged();
                if k % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    if changed && enabled {
        out.push(Command::DragSurface(id, next));
    }
    if dragging && here.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        out.push(Command::EndDrag { commit: false });
        cancelled = true;
    }
    ui.data_mut(|d| d.insert_temp(key, cancelled));
    // Shown in this pass (the inspector's banner for a surface moved and
    // not written is then not needed).
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|d| d.insert_temp(egui::Id::new("surface scrub shown"), (id, pass)));
    // Moved, not written: the card as Apply would write it.
    if let (Some(d), Some(source)) = (here.filter(|d| !d.committing), doc.source.as_ref()) {
        let moved = d.coeffs != written;
        if moved {
            if let (Some(before), Ok(after)) = (
                source.surface(id),
                source.surface_with_coeffs(id, &d.coeffs),
            ) {
                super::diff::cards(ui, &[(format!("surface {id}"), before.text, after)], 0);
                super::diff::after_apply(ui);
            }
        }
        ui.horizontal(|ui| {
            let r = ui.add_enabled(moved && !dragging, egui::Button::new("Apply"));
            crate::app::frame::named(&r, "Apply coefficients");
            let r = r.on_hover_text("Write the coefficients into the card and compile again");
            if r.clicked() {
                out.push(Command::EndDrag { commit: true });
            }
            let r = ui.button("Cancel");
            crate::app::frame::named(&r, "Cancel coefficients");
            if r.on_hover_text("Put the surface back where the card has it")
                .clicked()
            {
                out.push(Command::EndDrag { commit: false });
            }
        });
    }
    if let Some(d) = here {
        let status = if d.committing {
            "writing the card and compiling…".to_string()
        } else if let Some(p) = &d.problem {
            p.clone()
        } else if d.live {
            format!("live: step {:.1} ms", d.step_ms)
        } else {
            "preparing the live preview…".to_string()
        };
        ui.label(weak(status));
    }
}
