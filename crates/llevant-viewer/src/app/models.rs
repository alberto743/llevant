//! Opening, reloading and completing models: the background loads, the
//! universes opened on their own, and the file watched for changes.

use super::*;

impl ViewerApp {
    /// Check now and then (every 2 s) whether the input changed on disk.
    pub(super) fn watch(&mut self) {
        let Some((loaded, checked)) = &mut self.watched else {
            return;
        };
        if checked.elapsed().as_secs_f64() < 2.0 || self.input_changed {
            return;
        }
        *checked = std::time::Instant::now();
        let now = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        if now.is_some_and(|t| t > *loaded) {
            self.input_changed = true;
        }
    }

    pub(super) fn start_loading(&mut self, ctx: &egui::Context, base: Option<Arc<llevant::Model>>) {
        self.input_changed = false;
        // Octrees of the model shown are built no more: it is replaced.
        self.completing = None;
        self.watched = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok()
            .map(|t| (t, std::time::Instant::now()));
        let (path, adapter) = (self.path.clone(), self.render_state.adapter.clone());
        let window = self.gpu.clone();
        let scene = self.initial.scene.clone();
        let open = self.initial.open.clone();
        // A model opened from nothing is shown before its GPU copy is made
        // (seconds whatever its size); a reload shows the model before
        // meanwhile.
        let how = self.how(self.reloading);
        let wake = Some(ctx.clone());
        // A load under way is dropped: cancelled.
        self.loading = Some(Task::run("load", wake.clone(), move |p| {
            loading::load(&path, &open, &adapter, &window, &scene, p, how, base)
        }));
        let path = self.path.clone();
        self.source_loading = Some(Task::run("source", wake, move |_| {
            let start = std::time::Instant::now();
            let source = llevant_scene::source::Source::load(&path)
                .ok()
                .map(Arc::new);
            loading::log_step("source", start);
            let meshes = source
                .as_ref()
                .map(|s| Arc::new(InputMeshes::of(&s.tree())));
            loading::log_step("FMESH cards", start);
            let labels = source.as_ref().map(|s| Arc::new(Labels::of(&s.tree())));
            loading::log_step("labels", start);
            (source, meshes, labels)
        }));
        // Those of the input as it was are not given to the new models.
        self.labels = None;
        self.error = None;
        if self.reloading {
            // The cards shown stay until the new ones are read.
            return;
        }
        self.source = None;
        self.input_meshes = None;
        self.doc = None;
        self.parents.clear();
    }

    /// Open universe `u` of the open model on its own, in the background;
    /// the model stays open to go back to.
    pub(super) fn start_opening(&mut self, u: i64, ctx: &egui::Context) {
        let how = self.how(true);
        let Some(doc) = &mut self.doc else { return };
        if self.opening.is_some() {
            return;
        }
        let (model, adapter) = (doc.model.clone(), self.render_state.adapter.clone());
        let window = self.gpu.clone();
        let scene = opened_scene(&doc.scene);
        let labels = self.labels.clone();
        let task = Task::run("open", Some(ctx.clone()), move |_| {
            loading::open_universe(&model, u, labels, &adapter, &window, &scene, how)
        });
        doc.messages.progress(format!("opening universe {u}…"));
        self.opening = Some((u, task));
    }

    /// The labels, once read, given to every document's rules (each index
    /// takes them once: this is a check per document and frame).
    pub(super) fn give_labels(&mut self) {
        let Some(l) = &self.labels else { return };
        for d in self.parents.iter_mut().chain(&mut self.doc) {
            if d.index.labels().is_none() {
                d.give_labels(l);
            }
        }
    }

    pub(super) fn poll_source(&mut self) {
        let Some(task) = &self.source_loading else {
            return;
        };
        let Some(s) = task.try_take() else { return };
        self.source_loading = None;
        (self.source, self.input_meshes, self.labels) = s;
        for d in self.parents.iter_mut().chain(&mut self.doc) {
            d.source = self.source.clone();
            d.input_meshes = self.input_meshes.clone();
        }
    }

    pub(super) fn poll_opening(&mut self) {
        let Some((u, task)) = &self.opening else {
            return;
        };
        let Some(r) = task.try_take() else { return };
        let u = *u;
        self.opening = None;
        match r {
            Ok(l) => {
                let Some(parent) = self.doc.take() else {
                    return;
                };
                let mut state = parent.view_state(self.backend, self.budget_ms);
                state.camera = None;
                state.scene = loading::known_to(opened_scene(&parent.scene), &l.model);
                let mut doc = Document::open(l, &state, self.software(), &self.egui_ctx);
                doc.source = self.source.clone();
                doc.input_meshes = self.input_meshes.clone();
                if doc
                    .messages
                    .last()
                    .is_some_and(|m| m.level == crate::messages::Level::Info)
                {
                    doc.messages.info(format!(
                        "universe {u} on its own: {} cells; Back returns to the model",
                        doc.model.cells().len()
                    ));
                }
                self.parents.push(parent);
                self.doc = Some(doc);
                self.link_whole();
            }
            Err(e) => {
                if let Some(d) = &mut self.doc {
                    d.messages.warn(format!("could not open universe {u}: {e}"));
                }
            }
        }
    }

    /// Give each universe opened on its own the whole model (the first of
    /// the stack), to tell what its edits reach outside it.
    pub(super) fn link_whole(&mut self) {
        let whole = self
            .parents
            .first()
            .map(|d| (d.model.clone(), d.index.clone()));
        for d in self.parents.iter_mut().chain(self.doc.as_mut()) {
            d.whole = whole.clone().filter(|_| d.universe.is_some());
        }
    }

    /// Back to the model the open universe came from.
    pub(super) fn back(&mut self) {
        if let Some(parent) = self.parents.pop() {
            self.doc = Some(parent);
        }
    }

    /// The universes opened on their own, outermost first (the breadcrumb).
    pub fn open_universes(&self) -> Vec<i64> {
        self.parents
            .iter()
            .chain(&self.doc)
            .filter_map(|d| d.universe)
            .collect()
    }

    /// How a load in the background goes: the GPU copy made with the model
    /// when `shown_meanwhile` (another model is shown until it lands) and
    /// the 3D view traces on the GPU.
    pub(super) fn how(&self, shown_meanwhile: bool) -> loading::How {
        loading::How {
            fail_upload: self.dev.fail_gpu_upload(),
            stage_always: self.dev.stage_always(),
            gpu_now: shown_meanwhile && self.backend.wants_gpu(self.software()),
        }
    }

    /// The window's adapter is a software one (lavapipe): `auto` traces on
    /// the CPU.
    pub(super) fn software(&self) -> bool {
        self.gpu.info.device_type == wgpu::DeviceType::Cpu
    }

    pub(super) fn poll_loading(&mut self) {
        let Some(task) = &self.loading else { return };
        let Some(r) = task.try_take() else { return };
        let cancelled = task.progress().cancelled();
        let seconds = task.started().elapsed().as_secs_f64();
        self.loading = None;
        let reloading = std::mem::take(&mut self.reloading);
        let editing = std::mem::take(&mut self.edit_under_way);
        if cancelled {
            if !reloading {
                // Back to the start screen.
                self.path = PathBuf::new();
            }
            self.error = None;
            return;
        }
        let r = match r {
            Ok(load)
                if reloading
                    && self.doc.is_some()
                    && (load.chain.len() == 1 && self.parents.is_empty()
                        || load.edit.is_some() && load.chain.len() <= self.parents.len() + 1) =>
            {
                let loading::Load {
                    mut chain,
                    pending,
                    changed,
                    changed_boxes,
                    edit,
                    meshes,
                    labels,
                } = load;
                if meshes.is_some() {
                    self.input_meshes = meshes;
                }
                if labels.is_some() {
                    self.labels = labels;
                }
                // An edit (made in the cards shown): the edit to take back.
                let what = edit.map(|e| match e {
                    loading::InputEdit::Made(c) => {
                        let mut w = format!("edited {}", c.what);
                        if let Some(first) = c.notes.first() {
                            w.push_str(&format!(
                                " ({} references left as written: {first}{})",
                                c.notes.len(),
                                if c.notes.len() > 1 { "…" } else { "" }
                            ));
                            for n in &c.notes {
                                log::warn!("left as written: {n}");
                            }
                        }
                        self.edits.push(c);
                        w
                    }
                    loading::InputEdit::TakenBack(_) => {
                        self.select_after = None;
                        let w = self.edits.last().map_or(String::new(), |c| {
                            format!("took back the edit of {}", c.what)
                        });
                        self.edits.pop();
                        w
                    }
                });
                let software = self.software();
                // A universe the edit took away: back to the one holding it.
                let gone: Vec<i64> = self
                    .parents
                    .iter()
                    .chain(&self.doc)
                    .skip(chain.len())
                    .filter_map(|d| d.universe)
                    .collect();
                while self.parents.len() + 1 > chain.len() {
                    self.back();
                }
                // Each model of the chain in place of the one it was made
                // from, its view kept; the one shown last.
                let (backend, budget) = (self.backend, self.budget_ms);
                let mut boxes = changed_boxes.into_iter();
                let n = chain.len();
                let docs = self.parents.iter_mut().chain(self.doc.as_mut());
                for (k, (d, l)) in docs.zip(chain.drain(..)).enumerate() {
                    let mut state = d.view_state(backend, budget);
                    // The scene the new model was prepared at (the others
                    // are not shown meanwhile: theirs is the same).
                    if k + 1 == n && gone.is_empty() {
                        state.scene = self.initial.scene.clone();
                    }
                    d.replace_model(l, &state, false, software, &self.egui_ctx);
                    d.changed_boxes = boxes.next().flatten();
                    d.source = self.source.clone();
                }
                self.link_whole();
                let doc = self.doc.as_mut().expect("shown");
                if let Some(q) = self.select_after.take() {
                    doc.execute(Command::Select(q, crate::selection::SelectMode::Replace));
                }
                if let Some(s) = self.open_after.take() {
                    self.panels.inspector.open(s);
                }
                if what.is_some() {
                    doc.source = self.source.clone();
                    doc.input_meshes = self.input_meshes.clone();
                    // A cut written: the check of what it changed is next.
                    let check = crate::cutflow::check_id(doc);
                    if let Some(f) = &mut doc.cut_flow {
                        f.written(doc.source.as_ref(), check);
                    }
                }
                let mut message = reloaded_message(doc, changed.as_ref(), seconds);
                if let Some(w) = what {
                    message = message.replacen("reloaded", &w, 1);
                }
                if let Some(u) = gone.first() {
                    message.push_str(&format!(
                        "; universe {u} is no longer in the model: back to the one holding it"
                    ));
                }
                // Did the edit open a gap? Checked at once where it can have,
                // when the model is complete (else, from the Problems panel).
                if pending.is_none() {
                    if let Ok(p) = crate::panels::region::changed(doc, CHECK_CHANGED_LINES) {
                        doc.execute(Command::StartJob(
                            JobKind::Check,
                            p.region,
                            p.h,
                            p.scope,
                            p.within,
                        ));
                        message.push_str("; checking what changed for gaps and overlaps");
                    }
                }
                doc.messages.info(message);
                self.start_completing(pending);
                return;
            }
            Ok(load) => Ok(load),
            Err(e) if reloading && self.doc.is_some() => {
                self.select_after = None;
                self.open_after = None;
                if editing {
                    self.panels.inspector.edit_failed(&e.to_string());
                }
                if let Some(d) = &mut self.doc {
                    if editing {
                        d.cut_flow = None;
                    }
                    d.messages.warn(if editing {
                        format!("could not edit: {e}")
                    } else {
                        format!("could not reload: {e}")
                    });
                }
                return;
            }
            Err(e) => Err(e),
        };
        match r {
            Ok(load) => {
                // The view state is the last one's; the models it came from
                // open at their default view.
                let n = load.chain.len();
                for (k, l) in load.chain.into_iter().enumerate() {
                    let mut state = if k + 1 == n {
                        self.initial.clone()
                    } else {
                        ViewState::default()
                    };
                    state.backend = self.backend;
                    state.budget_ms = self.budget_ms;
                    let mut doc = Document::open(l, &state, self.software(), &self.egui_ctx);
                    doc.source = self.source.clone();
                    doc.input_meshes = self.input_meshes.clone();
                    if let Some(parent) = self.doc.replace(doc) {
                        self.parents.push(parent);
                    }
                }
                self.link_whole();
                // Once shown: its octrees next.
                self.start_completing(load.pending);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Build the octrees of `pending` (the whole model, shown staged) in
    /// the background, at the scene shown now.
    pub(super) fn start_completing(&mut self, pending: Option<llevant_scene::store::Staged>) {
        let Some(staged) = pending else { return };
        let Some(scene) = self
            .parents
            .first()
            .or(self.doc.as_ref())
            .map(|d| d.scene.clone())
        else {
            return;
        };
        let (adapter, window) = (self.render_state.adapter.clone(), self.gpu.clone());
        let how = self.how(true);
        let s = scene.clone();
        let labels = self.labels.clone();
        let task = Task::run("octrees", Some(self.egui_ctx.clone()), move |p| {
            loading::complete(staged, labels, &adapter, &window, &s, p, how)
        });
        self.completing = Some((scene, task));
    }

    /// The whole model completed: shown in place of the staged one.
    pub(super) fn poll_completing(&mut self) {
        let Some((_, task)) = &self.completing else {
            return;
        };
        let Some(r) = task.try_take() else { return };
        let seconds = task.started().elapsed().as_secs_f64();
        let (scene, _) = self.completing.take().expect("polled");
        let (backend, budget, software) = (self.backend, self.budget_ms, self.software());
        let ctx = self.egui_ctx.clone();
        let Some(doc) = self.parents.first_mut().or(self.doc.as_mut()) else {
            return;
        };
        if doc.universe.is_some() {
            return;
        }
        match r {
            Ok(l) => {
                let mut state = doc.view_state(backend, budget);
                state.scene = scene;
                doc.replace_model(l, &state, true, software, &ctx);
                doc.messages
                    .info(format!("search structures built in {seconds:.1} s"));
                self.link_whole();
            }
            Err(e) => doc.messages.warn(format!(
                "search structures not built ({e}): views stay slower"
            )),
        }
    }
}
