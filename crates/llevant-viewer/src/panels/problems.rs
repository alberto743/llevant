//! What is wrong with the model, IDE-style: what the compiler reported
//! (with input lines), the gaps and overlaps the slice has run into while
//! looking around, and a full check of a region for them as a job (as
//! `llevant check`). Each problem goes to where it is: the view framed on
//! it, the slice through it, its cells selected. On top, how they show in
//! 3D: gaps seen through, and the problems found drawn over everything
//! (`overlay`), the same switches as the 3D view's Display.

use llevant::{Label, Status, Subject};
use llevant_scene::query::Query;

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
fn problem_row(
    ui: &mut egui::Ui,
    doc: &Document,
    k: usize,
    l: &Label,
    volume: Option<(String, String)>,
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
            // An overlap of two cells: either can be cut out of the other;
            // the inspector shows what the cut would change.
            if l.status == Status::Overlap && l.involved.len() == 2 {
                let (a, b) = (m.cell_id(l.involved[0]), m.cell_id(l.involved[1]));
                ui.horizontal_wrapped(|ui| {
                    for (keep, cut) in [(a, b), (b, a)] {
                        let open = doc
                            .cut_flow
                            .as_ref()
                            .is_some_and(|f| f.is(keep, cut) && f.is_of(doc.source.as_ref()));
                        let r = ui
                            .selectable_label(open, format!("{} #{cut} from {keep}", icons::CLIP))
                            .on_hover_text("Click to preview the cut in the inspector");
                        if r.clicked() {
                            out.push(if open {
                                Command::CancelCutFlow
                            } else {
                                Command::PreviewCut { keep, cut }
                            });
                        }
                    }
                });
            }
        });
    go
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
                if problem_row(ui, doc, k, l, Some((volume, sigma)), out) {
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
                if problem_row(ui, doc, k, l, None, out) {
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
