// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! The overlap cut previewed from the Problems panel: the card of the cell
//! before and after, in red and green as every edit is (`diff`), with Apply
//! and Cancel. Nothing is written until Apply.

use super::diff;
use crate::command::Command;
use crate::cutpreview::CutPreview;
use crate::document::Document;
use crate::{icons, theme};

/// The preview open and made from the input as it is.
fn current(doc: &Document) -> Option<&CutPreview> {
    doc.cut_preview
        .as_ref()
        .filter(|p| p.is_of(doc.source.as_ref()))
}

pub fn has_content(doc: &Document) -> bool {
    current(doc).is_some()
}

pub fn section(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let Some(p) = current(doc) else {
        return;
    };
    let (keep, cut, simplify) = (p.keep, p.cut, p.simplify);
    let title = format!("Cut #{cut} from cell {keep}");
    let can_apply = theme::section(ui, icons::CLIP, &title, |ui| {
        p.with_plan(|plan| match plan {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Planning the cut…");
                });
                false
            }
            Some(Err(why)) => {
                ui.label(why.as_str());
                false
            }
            Some(Ok(p)) if p.result.empty => {
                ui.label(format!(
                    "Cell {keep} lies inside cell {cut}: cutting would leave nothing"
                ));
                false
            }
            Some(Ok(p)) => {
                diff::cards(
                    ui,
                    &[(
                        format!("cell {keep}"),
                        p.old_card.clone(),
                        p.new_card.clone(),
                    )],
                    0,
                );
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
                diff::after_apply(ui);
                true
            }
        })
    });
    ui.horizontal(|ui| {
        let apply = ui.add_enabled(can_apply.unwrap_or(false), egui::Button::new("Apply"));
        if apply.clicked() {
            out.push(Command::CutCell {
                keep,
                cut,
                simplify,
            });
        }
        if ui.button("Cancel").clicked() {
            out.push(Command::CancelCutPreview);
        }
    });
    ui.separator();
}
