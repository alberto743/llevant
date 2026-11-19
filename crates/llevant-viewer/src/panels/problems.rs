//! What is wrong with the model, IDE-style: what the compiler reported
//! (with input lines), the gaps and overlaps the slice has run into while
//! looking around, and a full check of a region for them as a job (as
//! `llevant check`). Each problem goes to where it is: the view framed on
//! it, the slice through it, its cells selected. On top, how they show in
//! 3D: gaps seen through, and the problems found drawn over everything
//! (`overlay`), the same switches as the 3D view's Display.

use llevant::{Label, Status, Subject};
use llevant_scene::query::Query;
use llevant_scene::source::{CutPlan, Source};
use std::sync::Arc;

use crate::command::Command;
use crate::context::{cell_link, link};
use crate::document::Document;
use crate::jobs::JobKind;
use crate::panels::region::RegionPicker;
use crate::selection::SelectMode;
use crate::{icons, theme};

#[derive(Default)]
pub struct ProblemsPanel {
    pub region: RegionPicker,
    /// Cut cells out of each other as `A #B`, without shortening the
    /// geometry by Boolean algebra.
    pub plain_cuts: bool,
    /// The last cut previewed: egui draws a tooltip every frame, and a plan
    /// can take long (the Boolean algebra of large cells).
    previews: PreviewCache,
}

/// A cut plan by what it was made from: the input (its `Source` and
/// revision), the two cells and `simplify`.
#[derive(Default)]
struct PreviewCache(Option<(PreviewKey, Result<CutPlan, String>)>);

type PreviewKey = (usize, u64, i64, i64, bool);

impl PreviewCache {
    fn plan(
        &mut self,
        source: &Arc<Source>,
        keep: i64,
        cut: i64,
        simplify: bool,
    ) -> &Result<CutPlan, String> {
        let key = (
            Arc::as_ptr(source) as usize,
            source.revision(),
            keep,
            cut,
            simplify,
        );
        if self.0.as_ref().is_none_or(|(k, _)| *k != key) {
            self.0 = Some((key, source.cut_plan(keep, cut, simplify)));
        }
        &self.0.as_ref().expect("just set").1
    }
}

/// Go to a problem: frame `b` (or a box around `p`), slice through `p`,
/// select its cells.
fn go(doc: &Document, label: &Label, b: Option<llevant::Aabb>, p: llevant::Vec3) -> Vec<Command> {
    let mut out = Vec::new();
    let b = b.unwrap_or_else(|| {
        let r = 0.05
            * (0..3)
                .map(|k| doc.region.hi[k] - doc.region.lo[k])
                .fold(0.0, f64::max);
        llevant::Aabb {
            lo: p.map(|x| x - r),
            hi: p.map(|x| x + r),
        }
    });
    out.push(Command::FrameBox(b));
    out.push(Command::SliceThrough(p));
    let mut ids: Vec<i64> = label
        .involved
        .iter()
        .map(|&c| doc.model.cell_id(c))
        .collect();
    if let Some(s) = label.path.last() {
        ids.push(doc.model.cell_id(s.cell));
    }
    if !ids.is_empty() {
        out.push(Command::Select(Query::Cells(ids), SelectMode::Replace));
    }
    out.push(Command::Focus(Some(crate::pick::pick_point(&doc.model, p))));
    out
}

/// At most this many cells of a problem are named in its row; the rest
/// are counted (and all of them are in the row's tooltip).
const CELLS_SHOWN: usize = 6;

/// One problem as two lines, so that it fits a narrow side panel: the "go"
/// button, the status and (right-aligned) the volume; under them, its
/// cells as links, wrapping, the first [`CELLS_SHOWN`]. Odd rows are on a
/// faint background. True when "go" was clicked.
#[allow(clippy::too_many_arguments)]
fn problem_row(
    ui: &mut egui::Ui,
    doc: &Document,
    k: usize,
    l: &Label,
    volume: Option<(String, String)>,
    simplify: bool,
    previews: &mut PreviewCache,
    out: &mut Vec<Command>,
) -> bool {
    let m = &doc.model;
    let fill = if k % 2 == 1 {
        ui.visuals().faint_bg_color
    } else {
        egui::Color32::TRANSPARENT
    };
    let mut go = false;
    egui::Frame::new()
        .fill(fill)
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                go = ui
                    .small_button(icons::GO_TO)
                    .on_hover_text("Go there")
                    .clicked();
                let c = match l.status {
                    Status::Gap => theme::GAP_MARK,
                    _ => theme::PROBLEM_MARK,
                };
                ui.colored_label(c, l.status.name())
                    .on_hover_text(m.describe(l));
                if let Some((v, sigma)) = &volume {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::data(v)).on_hover_text(sigma);
                    });
                }
            });
            let mut cells: Vec<u32> = l.path.last().map(|s| s.cell).into_iter().collect();
            for &c in &l.involved {
                if !cells.contains(&c) {
                    cells.push(c);
                }
            }
            if cells.is_empty() {
                ui.label(theme::data(m.describe(l)));
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (j, &c) in cells.iter().take(CELLS_SHOWN).enumerate() {
                    if j > 0 {
                        ui.label(egui::RichText::new("·").weak());
                    }
                    cell_link(ui, doc, c, out);
                }
                if cells.len() > CELLS_SHOWN {
                    ui.label(
                        egui::RichText::new(format!("+{} more", cells.len() - CELLS_SHOWN)).weak(),
                    )
                    .on_hover_text(m.describe(l));
                }
            });
            // An overlap of two cells: either can be cut out of the other.
            if l.status == Status::Overlap && l.involved.len() == 2 {
                let (a, b) = (m.cell_id(l.involved[0]), m.cell_id(l.involved[1]));
                ui.horizontal_wrapped(|ui| {
                    for (keep, cut) in [(a, b), (b, a)] {
                        let r = ui
                            .small_button(format!("{} #{cut} from {keep}", icons::CLIP))
                            .on_hover_ui(|ui| cut_preview(ui, doc, previews, keep, cut, simplify));
                        if r.clicked() {
                            out.push(Command::CutCell {
                                keep,
                                cut,
                                simplify,
                            });
                        }
                    }
                });
            }
        });
    go
}

/// What cutting `cut` out of `keep` would write: the card before and after.
fn cut_preview(
    ui: &mut egui::Ui,
    doc: &Document,
    previews: &mut PreviewCache,
    keep: i64,
    cut: i64,
    simplify: bool,
) {
    let Some(source) = &doc.source else {
        ui.label("the input is not loaded");
        return;
    };
    match previews.plan(source, keep, cut, simplify) {
        Err(why) => {
            ui.label(why.as_str());
        }
        Ok(p) if p.result.empty => {
            ui.label(format!(
                "Cell {keep} lies inside cell {cut}: cutting would leave nothing"
            ));
        }
        Ok(p) => {
            ui.label(format!("Cell {keep}, with cell {cut} cut out of it:"));
            ui.label(theme::data(format!("- {}", p.old_card.trim_end())));
            ui.label(theme::data(format!("+ {}", p.new_card.trim_end())));
            ui.label(
                egui::RichText::new(format!(
                    "{} terms (the plain cut has {}){}",
                    p.result.terms_after,
                    p.result.terms_before,
                    if p.result.simplified {
                        ", by Boolean algebra only"
                    } else {
                        ""
                    }
                ))
                .weak(),
            );
            for w in &p.result.warnings {
                ui.label(egui::RichText::new(w).weak());
            }
        }
    }
}

impl ProblemsPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let m = &doc.model;
        // How they show in 3D.
        theme::section(ui, icons::EFFECTS, "Problem display", |ui| {
            crate::panels::view_settings::problem_options(ui, doc, out);
        });
        // The full check.
        theme::section(ui, icons::TARGET, "Check for gaps and overlaps", |ui| {
            let picked = self.region.ui_with(ui, doc, true);
            let running = doc.results.jobs.iter().any(|j| j.kind == JobKind::Check);
            if ui
                .add_enabled(picked.is_ok() && !running, egui::Button::new("Check"))
                .on_hover_text("Lines traced through the region along x, y and z: every gap and overlap they cross")
                .clicked()
            {
                if let Ok(p) = &picked {
                    out.push(Command::StartJob(
                        JobKind::Check,
                        p.region,
                        p.h,
                        p.scope.clone(),
                        p.within.clone(),
                    ));
                }
            }
            ui.checkbox(&mut self.plain_cuts, "Plain cuts (no simplification)")
                .on_hover_text(
                    "An overlap can be fixed by cutting one cell out of the other. By default \
                     the geometry written is shortened with the laws of Boolean algebra only \
                     (proven equal to the plain A #B, whatever the surfaces are).",
                );
            let Some(run) = &doc.results.check else {
                return;
            };
            let v = &run.volumes;
            let problems = v.problems();
            ui.label(
                egui::RichText::new(format!(
                    "{}: {} problem{} (h = {} cm, {:.1} s)",
                    run.scope,
                    problems.len(),
                    if problems.len() == 1 { "" } else { "s" },
                    crate::jobs::short(v.grid.h),
                    run.seconds
                ))
                .weak(),
            );
            for (k, &i) in problems.iter().take(500).enumerate() {
                let (l, vol) = (&v.labels[i], &v.volumes[i]);
                let volume = format!("{:.2e} cm³", vol.estimate);
                let sigma = format!("± {:.1e} cm³ (one standard deviation)", vol.sigma);
                if problem_row(
                    ui,
                    doc,
                    k,
                    l,
                    Some((volume, sigma)),
                    !self.plain_cuts,
                    &mut self.previews,
                    out,
                ) {
                    out.extend(go(doc, l, Some(vol.bbox), vol.bbox.center()));
                }
            }
            if problems.len() > 500 {
                ui.label(
                    egui::RichText::new(format!("… and {} smaller ones", problems.len() - 500))
                        .weak(),
                );
            }
        });

        // Met while looking.
        let seen = &doc.slice.seen;
        theme::section(ui, icons::SLICE, "Seen in the slice", |ui| {
            if seen.is_empty() {
                ui.label(egui::RichText::new("none so far").weak());
                return;
            }
            for (k, (l, p)) in seen.iter().enumerate() {
                if problem_row(
                    ui,
                    doc,
                    k,
                    l,
                    None,
                    !self.plain_cuts,
                    &mut self.previews,
                    out,
                ) {
                    out.extend(go(doc, l, None, *p));
                }
            }
        });

        // From the compiler.
        let diags = m.diagnostics();
        let title = format!(
            "Input ({} diagnostic{})",
            diags.len(),
            if diags.len() == 1 { "" } else { "s" }
        );
        theme::section(ui, icons::INFO, &title, |ui| {
            if diags.is_empty() {
                ui.label(egui::RichText::new("llevant read every card it needs").weak());
                return;
            }
            for d in diags.iter().take(1000) {
                ui.horizontal_wrapped(|ui| {
                    if let Some(line) = d.line {
                        ui.label(theme::data(format!("line {line}")));
                    }
                    match d.subject {
                        Subject::Cell(id) if m.cell_index(id).is_some() => {
                            link(ui, doc, &Query::cell(id), format!("cell {id}"), out);
                        }
                        Subject::Material(id) => {
                            ui.label(theme::data(format!("m{id}")));
                        }
                        Subject::Surface(id) => {
                            ui.label(theme::data(format!("surface {id}")));
                        }
                        _ => {}
                    }
                    ui.label(&d.message);
                });
            }
        });
    }
}
