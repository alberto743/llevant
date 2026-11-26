// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! An overlap cut, step by step, as the inspector shows it (see
//! [`crate::cutflow`]):
//!
//! 1. Proposed: the cell with `#B` highlighted in yellow, to apply or cancel;
//! 2. Written: `#B` expanded, the expansion in yellow;
//! 3. Verified: whether the check of what changed found the overlap solved,
//!    remaining, or a new one of B with a cell that writes `#A` (if not
//!    solved, revert), and, if solved, the button to simplify the geometry;
//! 4. Simplified: a unified diff, in red and green;
//! 5. accepted or discarded (or the whole cut reverted).
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
                ui.horizontal(|ui| {
                    close_button(ui, "Cancel", out);
                    revert_button(ui, out);
                });
            }
            Some(Err(why)) => {
                ui.label(why.as_str());
                ui.horizontal(|ui| {
                    close_button(ui, "Close", out);
                    revert_button(ui, out);
                });
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

fn revert_button(ui: &mut egui::Ui, out: &mut Vec<Command>) {
    if ui
        .button("Revert")
        .on_hover_text("Take the cut back (the cell as it was) and close this")
        .clicked()
    {
        out.push(Command::RevertCut);
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
            if written {
                String::new()
            } else {
                format!(", with #{} added", plan.cut)
            }
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
            format!(
                "Apply writes #{} as it is (it cannot be expanded).",
                plan.cut
            )
        } else {
            format!(
                "Apply writes #{c} as what it stands for (the complement of cell {c}), then \
                 checks whether the overlap is solved; Revert takes it back.",
                c = plan.cut
            )
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
#[derive(Clone)]
enum Verdict {
    /// The check is running.
    Running,
    /// It did not run (the model was not complete).
    NotChecked,
    /// No overlap of the two cells, and none of B with a cell that writes
    /// `#A`, was found; other cells overlapping A are named.
    Solved(Vec<i64>),
    /// The two still overlap (and other cells overlapping A).
    Remains(Vec<i64>),
    /// Cells that write `#A` took in what was cut, and now overlap B (and
    /// other cells overlapping A).
    New { cells: Vec<i64>, others: Vec<i64> },
}

fn verdict(doc: &Document, f: &CutFlow, plan: &CutExpand, check_before: u64) -> Verdict {
    use crate::jobs::JobKind;
    if doc.results.jobs.iter().any(|j| j.kind == JobKind::Check) {
        return Verdict::Running;
    }
    let Some(run) = doc.results.check.as_ref() else {
        return Verdict::NotChecked;
    };
    // A check started before the cut was written looked at the model
    // without it, even if it finished after.
    if run.started <= check_before {
        return Verdict::NotChecked;
    }
    let m = &doc.model;
    let (Some(a), Some(b)) = (m.cell_index(f.keep), m.cell_index(f.cut)) else {
        return Verdict::NotChecked;
    };
    // A check that did not look where the two cells can overlap says nothing
    // of the cut (one of a region elsewhere, started from the panels).
    let problem = m.bounds(llevant::Scope::Problem);
    let together = m
        .cell(a)
        .bbox
        .intersect(&m.cell(b).bbox)
        .intersect(&problem);
    if !together.is_empty() && !run.covers(&together) {
        return Verdict::NotChecked;
    }
    let v = &run.volumes;
    let groups: Vec<&[u32]> = v
        .problems()
        .iter()
        .filter(|&&i| v.labels[i].status == llevant::Status::Overlap)
        .map(|&i| v.labels[i].involved.as_slice())
        .collect();
    let users: Vec<u32> = plan.users.iter().filter_map(|&n| m.cell_index(n)).collect();
    let j = judge(&groups, a, b, &users, |c| m.cell_id(c));
    if j.remains {
        Verdict::Remains(j.others)
    } else if !j.new.is_empty() {
        Verdict::New {
            cells: j.new,
            others: j.others,
        }
    } else {
        Verdict::Solved(j.others)
    }
}

/// What the overlaps found say about a cut of `b` out of `a`.
#[derive(Debug, PartialEq)]
struct Judgement {
    /// `a` and `b` still overlap.
    remains: bool,
    /// The cells among `users` (those that write `#a`) overlapping `b`.
    new: Vec<i64>,
    /// The other cells overlapping `a`.
    others: Vec<i64>,
}

/// The overlaps (`overlaps`, each the cells involved) judged for the cut of
/// `b` out of `a`, where `users` write `#a`; cells named by `id`.
fn judge(overlaps: &[&[u32]], a: u32, b: u32, users: &[u32], id: impl Fn(u32) -> i64) -> Judgement {
    let mut j = Judgement {
        remains: false,
        new: Vec::new(),
        others: Vec::new(),
    };
    let add = |v: &mut Vec<i64>, i: i64| {
        if !v.contains(&i) {
            v.push(i);
        }
    };
    for involved in overlaps {
        if involved.contains(&b) {
            for c in involved.iter().filter(|c| users.contains(c)) {
                add(&mut j.new, id(*c));
            }
        }
        if !involved.contains(&a) {
            continue;
        }
        j.remains |= involved.contains(&b);
        for &c in *involved {
            if c != a && c != b {
                add(&mut j.others, id(c));
            }
        }
    }
    j.new.sort_unstable();
    j.others.sort_unstable();
    j
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
    check_before: u64,
    out: &mut Vec<Command>,
) {
    geometry(ui, plan, true);
    notes(ui, &plan.warnings);
    let verdict = verdict(doc, f, plan, check_before);
    conclusion(ui, verdict, f.keep, f.cut, out);
}

/// The verdict, and what can be done about it.
fn conclusion(ui: &mut egui::Ui, verdict: Verdict, keep: i64, cut: i64, out: &mut Vec<Command>) {
    let green = egui::Color32::from_rgb(96, 200, 120);
    let revert = revert_button;
    match verdict {
        Verdict::Running => {
            spinner(ui, "Checking the overlap where the cell changed…");
            ui.horizontal(|ui| revert(ui, out));
        }
        Verdict::NotChecked => {
            ui.label(
                egui::RichText::new(
                    "No check of the place where the two cells meet has run since the cut \
                     (the model may not be complete, the check was of another region, or it \
                     started before the cut): check from the Problems panel to see whether \
                     the overlap is solved.",
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
                    "Overlap solved: the check found no overlap of cells {keep} and {cut} \
                     where cell {keep} changed"
                ),
            );
            if !others.is_empty() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("cell {} still overlaps cells {}", keep, cells_list(&others)),
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
                    keep, cut
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
        Verdict::New { cells, others } => {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!(
                    "New overlap: cell{s} {list} now overlap{v} cell {cut} (they write #{keep}, \
                     so they took in what was cut out of cell {keep})",
                    s = if cells.len() == 1 { "" } else { "s" },
                    v = if cells.len() == 1 { "s" } else { "" },
                    list = cells_list(&cells),
                ),
            );
            if !others.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "(cell {keep} still overlaps cells {})",
                        cells_list(&others)
                    ))
                    .weak(),
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
        ui.horizontal(|ui| {
            close_button(ui, "Close", out);
            revert_button(ui, out);
        });
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
        revert_button(ui, out);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn the_overlap_is_judged_from_the_cells_involved() {
        let id = |c: u32| i64::from(c) * 10;
        let j = |remains, new: Vec<i64>, others: Vec<i64>| Judgement {
            remains,
            new,
            others,
        };
        // Cells a=1, b=2: the pair still overlaps, and cell 3 as well.
        assert_eq!(
            judge(&[&[1, 2], &[1, 3], &[4, 5]], 1, 2, &[], id),
            j(true, vec![], vec![30])
        );
        // Only a third cell overlaps a: solved, with that cell named.
        assert_eq!(
            judge(&[&[1, 3], &[2, 3]], 1, 2, &[], id),
            j(false, vec![], vec![30])
        );
        // Cell 3 writes #1 and now overlaps b: a new overlap.
        assert_eq!(
            judge(&[&[2, 3], &[4, 5]], 1, 2, &[3, 4], id),
            j(false, vec![30], vec![])
        );
        // Nothing involves a or b with a user.
        assert_eq!(judge(&[&[2, 3]], 1, 2, &[], id), j(false, vec![], vec![]));
        assert_eq!(judge(&[], 1, 2, &[3], id), j(false, vec![], vec![]));
    }

    #[test]
    fn a_new_overlap_offers_only_revert_and_keep() {
        let mut h = shown(Verdict::New {
            cells: vec![5],
            others: vec![],
        });
        h.run();
        h.get_by_label_contains("New overlap: cell 5 now overlaps cell 3");
        assert!(h.query_by_label("Simplify").is_none());
        h.get_by_label("Keep it");
        h.get_by_label("Revert").click();
        h.run();
        assert!(matches!(h.state().as_slice(), [Command::RevertCut]));
    }

    fn shown(verdict: Verdict) -> egui_kittest::Harness<'static, Vec<Command>> {
        egui_kittest::Harness::new_ui_state(
            move |ui, out: &mut Vec<Command>| conclusion(ui, verdict.clone(), 2, 3, out),
            Vec::new(),
        )
    }

    #[test]
    fn when_the_overlap_remains_only_revert_and_keep_are_offered() {
        let mut h = shown(Verdict::Remains(vec![7]));
        h.run();
        h.get_by_label_contains("The overlap remains");
        h.get_by_label_contains("cells 7");
        assert!(
            h.query_by_label("Simplify").is_none(),
            "nothing to simplify"
        );
        h.get_by_label("Keep it");
        h.get_by_label("Revert").click();
        h.run();
        assert!(matches!(h.state().as_slice(), [Command::RevertCut]));
    }

    #[test]
    fn when_the_overlap_is_solved_simplify_is_offered_as_well() {
        let mut h = shown(Verdict::Solved(vec![]));
        h.run();
        h.get_by_label_contains("Overlap solved");
        h.get_by_label("Revert");
        h.get_by_label("Simplify").click();
        h.run();
        assert!(matches!(h.state().as_slice(), [Command::CutSimplify]));
    }
}
