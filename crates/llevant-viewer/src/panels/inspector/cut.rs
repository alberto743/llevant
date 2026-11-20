// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! An overlap cut, step by step, as the inspector shows it (see
//! [`crate::cutflow`]):
//!
//! 1. the cell with `#B` highlighted in yellow, to apply or cancel;
//! 2. what was written, `#B` expanded, the expansion in yellow;
//! 3. whether the check of what changed found the overlap solved (if not,
//!    revert), and, if it did, the button to simplify the geometry;
//! 4. the simplification as a unified diff, in red and green, to accept or
//!    discard.
//!
//! Nothing is written without a button.

use llevant_scene::source::{CutExpand, Simplified};

use super::diff;
use crate::command::Command;
use crate::cutflow::{self, CutFlow, Phase};
use crate::document::Document;
use crate::{icons, theme};

pub fn has_content(doc: &Document) -> bool {
    doc.cut_flow.is_some()
}

pub fn section(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let Some(f) = doc.cut_flow.as_ref() else {
        return;
    };
    // The input changed under it: closed.
    if !f.is_of(doc.source.as_ref()) {
        out.push(Command::CancelCutFlow);
        return;
    }
    let title = format!("Cut #{} from cell {}", f.cut, f.keep);
    theme::section(ui, icons::CLIP, &title, |ui| match &f.phase {
        Phase::Planning(slot) => cutflow::read(slot, |r| match r {
            None => {
                spinner(ui, "Planning the cut…");
                close_button(ui, "Cancel", out);
            }
            Some(Err(why)) => {
                ui.label(why.as_str());
                close_button(ui, "Close", out);
            }
            Some(Ok(plan)) if plan.empty => {
                ui.label(format!(
                    "Cell {} lies inside cell {}: cutting would leave nothing",
                    plan.keep, plan.cut
                ));
                close_button(ui, "Close", out);
            }
            Some(Ok(plan)) => proposed(ui, plan, out),
        }),
        Phase::Applying(plan) => {
            geometry(ui, plan, true);
            spinner(ui, "Writing the cut…");
        }
        Phase::Verifying { plan, check_before } => verifying(ui, doc, f, plan, *check_before, out),
        Phase::Simplifying { slot, .. } => cutflow::read(slot, |r| match r {
            None => {
                spinner(
                    ui,
                    &format!(
                        "Simplifying the geometry (stops after {} s)…",
                        cutflow::SIMPLIFY_LIMIT.as_secs()
                    ),
                );
                close_button(ui, "Cancel", out);
            }
            Some(Err(why)) => {
                ui.label(why.as_str());
                close_button(ui, "Close", out);
            }
            Some(Ok(s)) => review(ui, s, out),
        }),
    });
    ui.separator();
}

fn spinner(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(text);
    });
}

fn close_button(ui: &mut egui::Ui, text: &str, out: &mut Vec<Command>) {
    if ui.button(text).clicked() {
        out.push(Command::CancelCutFlow);
    }
}

fn notes(ui: &mut egui::Ui, warnings: &[String]) {
    for w in warnings {
        ui.label(egui::RichText::new(w).weak());
    }
}

/// The geometry of the cell as written (`written`) or as it would be with
/// `#B` only, the part the cut adds highlighted.
fn geometry(ui: &mut egui::Ui, plan: &CutExpand, written: bool) {
    ui.label(
        theme::data(format!(
            "cell {}{}",
            plan.keep,
            if written { "" } else { ", with #B added" }
        ))
        .weak()
        .small(),
    );
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            if written {
                diff::marked(ui, &plan.geometry, plan.mark.clone());
            } else {
                diff::marked(ui, &plan.plain_geometry, plan.plain_mark.clone());
            }
        });
}

/// Step 1: `#B` added, highlighted; Apply or Cancel.
fn proposed(ui: &mut egui::Ui, plan: &CutExpand, out: &mut Vec<Command>) {
    geometry(ui, plan, false);
    ui.label(
        egui::RichText::new(if plan.plain {
            "Apply writes #B as it is (it cannot be expanded)."
        } else {
            "Apply writes #B as what it stands for (the complement of cell B), then checks \
             whether the overlap is solved; Revert takes it back."
        })
        .weak()
        .small(),
    );
    notes(ui, &plan.warnings);
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            out.push(Command::CutApply(plan.clone()));
        }
        if ui.button("Cancel").clicked() {
            out.push(Command::CancelCutFlow);
        }
    });
}

/// Where the overlap stands, from the check of what the cut changed.
enum Verdict {
    /// The check is running.
    Running,
    /// It did not run (the model was not complete).
    NotChecked,
    /// No overlap of the two cells was found; other cells overlapping the
    /// one cut are named.
    Solved(Vec<i64>),
    /// The two still overlap (and other cells overlapping the one cut).
    Remains(Vec<i64>),
}

fn verdict(doc: &Document, f: &CutFlow, check_before: usize) -> Verdict {
    use crate::jobs::JobKind;
    if doc.results.jobs.iter().any(|j| j.kind == JobKind::Check) {
        return Verdict::Running;
    }
    let Some(run) = doc.results.check.as_ref() else {
        return Verdict::NotChecked;
    };
    if cutflow::check_id(doc) == check_before {
        return Verdict::NotChecked;
    }
    let m = &doc.model;
    let (Some(a), Some(b)) = (m.cell_index(f.keep), m.cell_index(f.cut)) else {
        return Verdict::NotChecked;
    };
    let v = &run.volumes;
    let mut both = false;
    let mut others: Vec<i64> = Vec::new();
    for &i in &v.problems() {
        let l = &v.labels[i];
        if l.status != llevant::Status::Overlap || !l.involved.contains(&a) {
            continue;
        }
        if l.involved.contains(&b) {
            both = true;
        }
        for &c in &l.involved {
            let id = m.cell_id(c);
            if c != a && c != b && !others.contains(&id) {
                others.push(id);
            }
        }
    }
    others.sort_unstable();
    if both {
        Verdict::Remains(others)
    } else {
        Verdict::Solved(others)
    }
}

fn cells_list(ids: &[i64]) -> String {
    let mut s: Vec<String> = ids.iter().take(8).map(|c| c.to_string()).collect();
    if ids.len() > 8 {
        s.push(format!("+{} more", ids.len() - 8));
    }
    s.join(" ")
}

/// Steps 2 and 3: what was written, and whether it solved the overlap.
fn verifying(
    ui: &mut egui::Ui,
    doc: &Document,
    f: &CutFlow,
    plan: &CutExpand,
    check_before: usize,
    out: &mut Vec<Command>,
) {
    geometry(ui, plan, true);
    notes(ui, &plan.warnings);
    let verdict = verdict(doc, f, check_before);
    let green = egui::Color32::from_rgb(96, 200, 120);
    let revert = |ui: &mut egui::Ui, out: &mut Vec<Command>| {
        if ui
            .button("Revert")
            .on_hover_text("Take the cut back (the cell as it was) and close this")
            .clicked()
        {
            out.push(Command::RevertCut);
        }
    };
    match verdict {
        Verdict::Running => {
            spinner(ui, "Checking the overlap where the cell changed…");
            ui.horizontal(|ui| revert(ui, out));
        }
        Verdict::NotChecked => {
            ui.label(
                egui::RichText::new(
                    "The check did not run (the model is not complete): check from the \
                     Problems panel to see whether the overlap is solved.",
                )
                .weak(),
            );
            ui.horizontal(|ui| {
                simplify_button(ui, out);
                revert(ui, out);
                close_button(ui, "Keep it", out);
            });
        }
        Verdict::Solved(others) => {
            ui.colored_label(
                green,
                format!(
                    "Overlap solved: the check found no overlap of cells {} and {}",
                    f.keep, f.cut
                ),
            );
            if !others.is_empty() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "cell {} still overlaps cells {}",
                        f.keep,
                        cells_list(&others)
                    ),
                );
            }
            ui.horizontal(|ui| {
                simplify_button(ui, out);
                revert(ui, out);
                close_button(ui, "Keep it", out);
            });
        }
        Verdict::Remains(others) => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!(
                    "The overlap remains: the check still found cells {} and {} overlapping",
                    f.keep, f.cut
                ),
            );
            if !others.is_empty() {
                ui.label(
                    egui::RichText::new(format!("(and cells {})", cells_list(&others))).weak(),
                );
            }
            ui.horizontal(|ui| {
                revert(ui, out);
                close_button(ui, "Keep it", out);
            });
        }
    }
}

fn simplify_button(ui: &mut egui::Ui, out: &mut Vec<Command>) {
    if ui
        .button("Simplify")
        .on_hover_text(
            "Shorten the geometry with the laws of Boolean algebra; the result is proven \
             equal to it, and shown as a diff to accept or discard",
        )
        .clicked()
    {
        out.push(Command::CutSimplify);
    }
}

/// Step 4: the simplification as a diff; Accept or Discard.
fn review(ui: &mut egui::Ui, s: &Simplified, out: &mut Vec<Command>) {
    if !s.simplified {
        ui.label("The geometry is as short as the algebra gets: nothing to change.");
        notes(ui, &s.warnings);
        close_button(ui, "Close", out);
        return;
    }
    ui.label(
        egui::RichText::new(format!(
            "{} terms instead of {}; proven equal to the geometry as it is",
            s.terms_after, s.terms_before
        ))
        .weak(),
    );
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            diff::unified(
                ui,
                &format!("cell {} (as it is)", s.keep),
                &format!("cell {} (simplified)", s.keep),
                &diff::display_lines(&s.old_card),
                &diff::display_lines(&s.new_card),
            );
        });
    if !s.trace.is_empty() {
        ui.label(egui::RichText::new(s.trace.join(" · ")).weak().small());
    }
    notes(ui, &s.warnings);
    ui.horizontal(|ui| {
        if ui.button("Accept").clicked() {
            out.push(Command::CutAccept(s.clone()));
        }
        let discard = ui
            .button("Discard")
            .on_hover_text("Keep the geometry as it is");
        crate::app::frame::named(&discard, "Discard simplification");
        if discard.clicked() {
            out.push(Command::CancelCutFlow);
        }
    });
}
