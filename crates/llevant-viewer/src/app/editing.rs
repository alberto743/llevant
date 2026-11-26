//! Editing the input from the viewer: cards edited, renumbered or added,
//! surfaces dragged, edits taken back, and the input saved.

use super::*;

impl ViewerApp {
    /// Why the input's cards cannot be edited now, if they cannot.
    pub(super) fn edit_blocked(&self) -> Option<&'static str> {
        if self.loading.is_some() {
            Some("wait for the model being loaded")
        } else if self.saving.is_some() {
            Some("wait for the input being saved")
        } else if self.opening.is_some() {
            Some("wait for the universe being opened")
        } else if self.source.is_none() || self.source_loading.is_some() {
            Some("the input's cards are being read")
        } else if self.source.as_ref().is_some_and(|s| s.not_utf8()) {
            Some("the input is not UTF-8: saving it would change its other bytes")
        } else if self.dragging.as_ref().is_some_and(|d| !d.committing) {
            Some("apply or cancel the surface being moved first")
        } else {
            None
        }
    }

    /// Write card `kind` `id` as `text` and compile the input again against
    /// the model shown (only what changed is built), keeping the view; the
    /// edit is not saved until [`Command::SaveInput`].
    pub(super) fn edit_card(
        &mut self,
        kind: llevant_scene::source::CardKind,
        id: i64,
        text: String,
        ctx: &egui::Context,
    ) {
        let what = format!("{} {id}", kind.name());
        self.start_change(ctx, what, None, move |source| {
            Ok(input_edits::Change::one(id, source.edit(kind, id, &text)?))
        });
    }

    /// The cut proposed written (`#cut` expanded), as one edit (one Undo);
    /// both cells then selected. The cut stays open in the inspector, which
    /// says whether the check of what changed found the overlap solved.
    pub(super) fn cut_apply(
        &mut self,
        plan: llevant_scene::source::CutExpand,
        ctx: &egui::Context,
    ) {
        let what = format!("cell {} cut by #{}", plan.keep, plan.cut);
        let select = Query::Cells(vec![plan.keep, plan.cut]);
        if let Some(d) = self.doc.as_mut() {
            // Before the cut: where the cells meet, and who overlapped B.
            let meet = crate::cutflow::where_they_meet(&d.model, plan.keep, plan.cut);
            let before = crate::cutflow::before(d, plan.cut);
            if let Some(f) = d.cut_flow.as_mut() {
                f.phase = crate::cutflow::Phase::Applying(plan.clone());
                f.meet = meet;
                f.before = before;
            }
        }
        let said = what.clone();
        let started = self.start_change(ctx, what, Some(select), move |source| {
            let mut change = input_edits::Change::one(plan.keep, source.apply_expand(&plan)?);
            change.what = said;
            Ok(change)
        });
        // Not started (an edit is under way, or refused): the cut is closed,
        // and what was said is in the messages. Only the edit this started
        // may take the cut to the next step.
        if !started {
            if let Some(d) = &mut self.doc {
                d.cut_flow = None;
            }
        }
    }

    /// The simplification shown written, as one edit (one Undo); the cut is
    /// closed. Not started (a save is under way): the simplification stays
    /// shown, to be accepted when it can.
    pub(super) fn cut_accept(&mut self, s: llevant_scene::source::Simplified, ctx: &egui::Context) {
        let what = format!("cell {} simplified", s.keep);
        let select = Query::Cells(vec![s.keep]);
        let said = what.clone();
        let started = self.start_change(ctx, what, Some(select), move |source| {
            let mut change = input_edits::Change::one(s.keep, source.apply_simplified(&s)?);
            change.what = said;
            Ok(change)
        });
        if started {
            if let Some(d) = &mut self.doc {
                d.cut_flow = None;
            }
        }
    }

    /// Cells `ids` each with `changes` made, as one edit (one Undo); the
    /// cells then selected (a rule they matched may no longer).
    pub(super) fn edit_cells(
        &mut self,
        ids: Vec<i64>,
        changes: Vec<llevant_scene::source::CellChange>,
        ctx: &egui::Context,
    ) {
        let what = match ids.as_slice() {
            [one] => format!("cell {one}"),
            _ => format!("{} cells", ids.len()),
        };
        let select = Query::Cells(ids.clone());
        let w = what.clone();
        self.start_change(ctx, what, Some(select), move |source| {
            let cards = source.cells_with(&ids, &changes)?;
            Ok(input_edits::Change {
                what: w,
                cards: source.edit_many(&cards)?,
                notes: Vec::new(),
            })
        });
    }

    /// Numbers of `kind` changed, `(old, new)`, with every card that uses
    /// them, as one edit; renumbered cells then selected.
    pub(super) fn renumber(
        &mut self,
        kind: llevant_scene::source::IdKind,
        pairs: Vec<(i64, i64)>,
        ctx: &egui::Context,
    ) {
        use llevant_scene::source::IdKind;
        let name = match kind {
            IdKind::Cell => "cells",
            IdKind::Surface => "surfaces",
            IdKind::Material => "materials",
            IdKind::Transform => "transforms",
            IdKind::Universe => "universes",
            IdKind::Tally => "tallies",
        };
        // Universes: no card defines one; the model knows which are used.
        if kind == IdKind::Universe {
            if let Some(doc) = &self.doc {
                let used: std::collections::HashSet<i64> =
                    doc.model.units().iter().map(|u| u.universe).collect();
                if let Some(&(_, t)) = pairs.iter().find(|(a, b)| a != b && used.contains(b)) {
                    self.refuse_edit(&format!("universe {t} is in use"));
                    return;
                }
            }
        }
        let moved = pairs.iter().filter(|(a, b)| a != b).count();
        let what = format!("the numbers of {moved} {name}");
        let select =
            (kind == IdKind::Cell).then(|| Query::Cells(pairs.iter().map(|p| p.1).collect()));
        let w = what.clone();
        self.start_change(ctx, what, select, move |source| {
            let r = source.renumber(kind, &pairs)?;
            if r.cards.is_empty() {
                return Err(format!("no {name} to renumber"));
            }
            Ok(input_edits::Change {
                what: w,
                cards: r.cards,
                notes: r.unfollowed,
            })
        });
    }

    /// Add card `text` (`kind` `id`) at the end of its block, a cell cut
    /// out of cell `carve` with it, as one edit; its page then shown.
    pub(super) fn add_card(
        &mut self,
        kind: llevant_scene::source::CardKind,
        id: i64,
        text: String,
        carve: Option<i64>,
        ctx: &egui::Context,
    ) {
        use crate::panels::inspector::Subject;
        use llevant_scene::source::CardKind;
        let what = format!("new {} {id}", kind.name());
        let select = (kind == CardKind::Cell).then(|| Query::Cells(vec![id]));
        let w = what.clone();
        self.start_change(ctx, what, select, move |source| {
            let mut cards = source.add_card(kind, &text)?;
            if let Some(p) = carve {
                let cut = source
                    .cell_cut(p, id)
                    .and_then(|t| source.edit_many(&[(CardKind::Cell, p, t)]));
                match cut {
                    Ok(e) => cards.extend(e),
                    Err(e) => {
                        let _ = source.restore_all(&cards);
                        return Err(e);
                    }
                }
            }
            Ok(input_edits::Change {
                what: w,
                cards,
                notes: Vec::new(),
            })
        });
        self.open_after = Some(match kind {
            CardKind::Cell => Subject::Cell(id),
            CardKind::Surface => Subject::Surface(id),
            CardKind::Material => Subject::Material(id),
            _ => Subject::Picked,
        });
    }

    /// Say an edit cannot be made (on its form and in the status).
    pub(super) fn refuse_edit(&mut self, why: &str) {
        self.panels.inspector.edit_failed(why);
        if let Some(d) = &mut self.doc {
            d.messages.warn(why);
        }
    }

    /// Make the change `make` gives in the input and compile it against the
    /// model shown, keeping the view; `select` selected once it lands.
    /// Whether it was started (not when an edit is under way, or the
    /// input is not there to edit: what was refused is said in the messages).
    pub(super) fn start_change(
        &mut self,
        ctx: &egui::Context,
        what: String,
        select: Option<Query>,
        make: impl FnOnce(&llevant_scene::source::Source) -> Result<input_edits::Change, String>
            + Send
            + 'static,
    ) -> bool {
        if let Some(why) = self.edit_blocked() {
            self.refuse_edit(&format!("cannot edit now: {why}"));
            return false;
        }
        let (Some(_), Some(source)) = (&self.doc, self.source.clone()) else {
            return false;
        };
        self.select_after = select;
        self.start_edit(ctx, source, move |source| {
            Ok(loading::InputEdit::Made(make(source)?))
        });
        if let Some(d) = &mut self.doc {
            d.messages
                .progress(format!("editing {what}: building only what changed…"));
        }
        true
    }

    /// Move surface `id` to `coeffs`, previewed live: the drag begun if
    /// none is under way.
    pub(super) fn drag_surface(&mut self, id: i64, coeffs: Vec<f64>, ctx: &egui::Context) {
        if let Some(dr) = &mut self.dragging {
            if dr.surface == id && !dr.committing && !dr.stopping {
                dr.set(coeffs.clone());
                if let Some(info) = self.doc.as_mut().and_then(|d| d.drag.as_mut()) {
                    info.coeffs = coeffs;
                }
            }
            return;
        }
        if let Some(why) = self.edit_blocked() {
            if let Some(d) = &mut self.doc {
                d.messages.warn(format!("cannot edit now: {why}"));
            }
            return;
        }
        let (Some(doc), Some(source)) = (&mut self.doc, self.source.clone()) else {
            return;
        };
        let Some((_, start)) = source.surface_coeffs(id) else {
            return;
        };
        // The GPU copy is patched only when it is what traces the view.
        let gpu = match (&doc.gpu_model, doc.view3d.backend_name()) {
            (Ok((g, gm)), "GPU") => Some((g.clone(), gm.clone())),
            _ => None,
        };
        let mut dr = drag::Dragging::start(
            id,
            start,
            doc.model.clone(),
            source,
            doc.shown().clone(),
            gpu,
            ctx.clone(),
        );
        dr.set(coeffs.clone());
        doc.drag = Some(crate::document::DragInfo {
            surface: id,
            coeffs,
            ..Default::default()
        });
        self.dragging = Some(dr);
    }

    /// End the drag under way: with `commit`, its card written with the
    /// coefficients reached and compiled again as any edit (the preview
    /// shown until the new model lands); else the preview taken back.
    pub(super) fn end_drag(&mut self, commit: bool, ctx: &egui::Context) {
        let Some(dr) = &mut self.dragging else { return };
        if dr.committing || dr.stopping {
            return;
        }
        if !commit || dr.coeffs == dr.start {
            dr.stop(true);
            return;
        }
        let (id, coeffs) = (dr.surface, dr.coeffs.clone());
        // Committing: no longer a drag that blocks edits.
        dr.committing = true;
        let text = match &self.source {
            Some(s) => s.surface_with_coeffs(id, &coeffs),
            None => Err("the input's cards are not read".into()),
        };
        match text {
            Ok(text) => {
                self.edit_card(llevant_scene::source::CardKind::Surface, id, text, ctx);
            }
            Err(e) => {
                if let Some(d) = &mut self.doc {
                    d.messages.warn(format!("could not edit: {e}"));
                }
            }
        }
        let started = self.edit_under_way && self.loading.is_some();
        let dr = self.dragging.as_mut().expect("dragging");
        if started {
            dr.committing = true;
            if let Some(info) = self.doc.as_mut().and_then(|d| d.drag.as_mut()) {
                info.committing = true;
            }
        } else {
            dr.committing = false;
            dr.stop(true);
        }
    }

    /// What the drag's worker sent, shown; the drag dropped once the model
    /// it previewed is replaced (its edit landed, or another model).
    pub(super) fn poll_drag(&mut self) {
        let Some(dr) = &mut self.dragging else { return };
        // The document dragged in: the one shown, or one left for a
        // universe opened meanwhile (the drag then taken back).
        let shown_here = self.doc.as_ref().is_some_and(|d| d.drag.is_some());
        let Some(doc) = dragged(&mut self.doc, &mut self.parents) else {
            // Replaced: by the edit's model (a new GPU copy), or another.
            self.dragging = None;
            return;
        };
        if dr.committing && !self.edit_under_way {
            // The edit failed or was cancelled: the model stays.
            dr.committing = false;
            dr.stop(true);
        } else if !shown_here && !dr.stopping {
            dr.stop(true);
        }
        while let Ok(m) = dr.rx.try_recv() {
            match m {
                drag::FromWorker::Ready(s) => {
                    doc.show_dragged(&s);
                    if let Some(i) = &mut doc.drag {
                        i.live = true;
                    }
                }
                drag::FromWorker::Step(step, ms) => {
                    doc.draft_step(&step);
                    if let Some(i) = &mut doc.drag {
                        i.step_ms = ms;
                        i.steps += 1;
                        i.problem = None;
                    }
                }
                drag::FromWorker::Refused(e) => {
                    if let Some(i) = &mut doc.drag {
                        i.problem = Some(e);
                    }
                }
                drag::FromWorker::Unavailable(e) => {
                    if let Some(i) = &mut doc.drag {
                        i.problem = Some(format!("no live preview: {e}"));
                    }
                }
                drag::FromWorker::Stopped(undo) => {
                    doc.end_drag(undo);
                    self.dragging = None;
                    return;
                }
            }
        }
        if let Some(i) = doc.drag.as_mut().filter(|i| i.reshow) {
            i.reshow = false;
            dr.show(doc.shown().clone());
        }
    }

    /// Take back the last card edit (compiled again, as an edit).
    pub(super) fn undo_input_edit(&mut self, ctx: &egui::Context) {
        if self.edit_blocked().is_some() {
            return;
        }
        let (Some(_), Some(c), Some(source)) = (&self.doc, self.edits.last(), self.source.clone())
        else {
            return;
        };
        let what = c.what.clone();
        let change = c.clone();
        self.select_after = None;
        self.start_edit(ctx, source, move |source| {
            source.restore_all(&change.cards)?;
            Ok(loading::InputEdit::TakenBack(change))
        });
        if let Some(d) = &mut self.doc {
            d.messages
                .progress(format!("taking back the edit of {what}…"));
        }
    }

    /// Make `edit` in `source` and compile it against `base` in the
    /// background, the model shown staying until it lands (the edit taken
    /// back if it does not).
    pub(super) fn start_edit(
        &mut self,
        ctx: &egui::Context,
        source: Arc<llevant_scene::source::Source>,
        edit: impl FnOnce(&llevant_scene::source::Source) -> Result<loading::InputEdit, String>
            + Send
            + 'static,
    ) {
        self.initial = self.view_state();
        self.reloading = true;
        self.edit_under_way = true;
        self.completing = None;
        let adapter = self.render_state.adapter.clone();
        let window = self.gpu.clone();
        // Compiled against the whole model; each universe opened on its own
        // cut out again, at the scene it shows.
        let opened = loading::Opened {
            models: self
                .parents
                .iter()
                .chain(&self.doc)
                .map(|d| (d.universe, d.model.clone(), d.scene.clone()))
                .collect(),
        };
        let how = loading::How {
            stage_always: false,
            ..self.how(true)
        };
        let labels = self.labels.clone();
        self.loading = Some(Task::run("edit", Some(ctx.clone()), move |p| {
            let edit = edit(&source)?;
            let undo = edit.clone();
            loading::load_edit(edit, &source, labels, opened, &adapter, &window, p, how)
                .inspect_err(|_| undo.revert(&source))
        }));
    }

    /// Write the input as edited to its file, in the background; with
    /// `to`, to that file instead, which is then the input (Save as).
    pub(super) fn save_input(&mut self, to: Option<PathBuf>, ctx: &egui::Context) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.saving.is_some() || self.edit_blocked().is_some() {
            return;
        }
        if to.is_none() && !self.edits.unsaved() {
            return;
        }
        let path = to.unwrap_or_else(|| self.path.clone());
        let to = path.clone();
        let task = Task::run("save input", Some(ctx.clone()), move |_| {
            write_input(&source, &to).map_err(|e| format!("{}: {e}", to.display()))
        });
        self.saving = Some((self.edits.len(), path, task));
    }

    /// A file to save the input as, picked in a dialog (beside it).
    pub(super) fn pick_save_as(&self) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new().set_title("Save the input as");
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            d = d.set_directory(dir);
        }
        if let Some(name) = self.path.file_name() {
            d = d.set_file_name(name.to_string_lossy());
        }
        d.save_file()
    }

    pub(super) fn poll_saving(&mut self) {
        let Some((n, _, task)) = &self.saving else {
            return;
        };
        let Some(r) = task.try_take() else { return };
        let n = *n;
        let (_, path, _) = self.saving.take().expect("saving");
        let message: Result<String, String> = match r {
            Ok(modified) => {
                if path != self.path {
                    // Saved as: that file is the input now.
                    self.state_path = default_state_path(&path);
                    self.path = path;
                    if let Some(s) = &mut self.settings {
                        s.opened(&self.path);
                        s.save();
                    }
                }
                self.edits.saved_at(n);
                // The watcher's own write: not a change on disk.
                self.watched = Some((modified, std::time::Instant::now()));
                self.input_changed = false;
                Ok(format!("saved {}", self.path.display()))
            }
            Err(e) => Err(format!("could not save the input: {e}")),
        };
        if let Some(d) = &mut self.doc {
            d.messages.result(message);
        }
    }

    /// Whether edits not saved may be dropped by `action`: none, or the
    /// user says so.
    pub(super) fn may_drop_edits(&self, action: &str) -> bool {
        !self.edits.unsaved()
            || rfd::MessageDialog::new()
                .set_title(action)
                .set_description(format!(
                    "{} card edits of {} are not saved. {action} and drop them?",
                    self.edits.len(),
                    self.path.display()
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }

    /// The banner of the cards edited here: save, take back, drop, copy.
    pub(super) fn edits_banner(&self, ui: &mut egui::Ui, out: &mut Vec<Command>) {
        let busy = self
            .edit_blocked()
            .filter(|_| self.loading.is_some() || self.saving.is_some());
        egui::Panel::top("edits").show(ui, |ui| {
            ui.horizontal(|ui| {
                let n = self.edits.len();
                let state = if self.saving.is_some() {
                    "saving…"
                } else if self.edits.unsaved() {
                    "not saved"
                } else {
                    "saved"
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{n} card edit{} of {}, {state}.",
                        if n == 1 { "" } else { "s" },
                        self.path
                            .file_name()
                            .map_or(String::new(), |n| n.to_string_lossy().into())
                    ))
                    .color(theme::SELECTION),
                );
                let idle = busy.is_none();
                if ui
                    .add_enabled(idle && self.edits.unsaved(), egui::Button::new("Save"))
                    .on_hover_text("Write the input as edited to its file")
                    .clicked()
                {
                    out.push(Command::SaveInput);
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Save as…"))
                    .on_hover_text("Write the input as edited to another file, which is then the one open")
                    .clicked()
                {
                    out.push(Command::SaveInputAs(None));
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Undo edit"))
                    .on_hover_text("Take back the last card edit")
                    .clicked()
                {
                    out.push(Command::UndoInputEdit);
                }
                if ui
                    .add_enabled(idle && self.edits.unsaved(), egui::Button::new("Discard"))
                    .on_hover_text("Drop the edits not saved: read the file again")
                    .clicked()
                {
                    out.push(Command::DiscardInputEdits);
                }
                if ui
                    .button(crate::icons::with(crate::icons::COPY, "Copy changed cards"))
                    .on_hover_text(
                        "Each card edited, before and after, to carry into the sources the input is assembled from",
                    )
                    .clicked()
                {
                    ui.ctx().copy_text(self.edits.report());
                }
            });
        });
    }
}
