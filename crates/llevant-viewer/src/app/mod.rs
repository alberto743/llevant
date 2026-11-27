//! The viewer application: a model loaded in the background into a
//! [`Document`], shown in one of four workspaces ([`frame`], decision 0012):
//! the menu bar and the workspaces' tabs on top, the workspace's panels on
//! the left, the inspector on the right, the status bar at the bottom, and
//! the tool row, the 3D view and the slice in the middle.
//!
//! Each frame, panels, tools and navigation read the document and collect
//! [`Command`]s; they run once the interface is laid out.
//!
//! [`ViewerApp`]'s methods are spread over this module (making it, its
//! accessors, the frame) and `commands.rs` (running the commands),
//! `models.rs` (opening, reloading, completing), `editing.rs` (edits to the
//! input, saving) and `drag.rs` (live surface drags).

mod commands;
mod devtools;
mod drag;
mod editing;
pub mod frame;
mod input_edits;
pub(crate) mod loading;
mod menu;
mod models;
mod setup;
mod start;
mod storm;

pub use setup::wgpu_options;
pub(crate) use setup::WINDOW_LOST;

use std::path::PathBuf;
use std::sync::Arc;

use llevant_gpu::Gpu;
use llevant_scene::labels::Labels;
use llevant_scene::query::Query;

use crate::command::Command;
use crate::context;
use crate::document::{Document, HoverKey, Loaded};
use crate::jobs::JobKind;
use crate::mesh::{cards_text, InputMeshes};
use crate::navigation::navigate_3d;
use crate::panels::palette::Palette;
use crate::panels::{keys, status, toolbar};
use crate::scene::Scene;
use crate::settings::Settings;
use crate::state::{BackendChoice, ViewState};
use crate::tasks::Task;
use crate::tools::Toolbox;
use crate::viewport::{Projection, ViewKind, ViewPort};
use crate::{overlay, theme};
use devtools::DevHooks;
pub use devtools::DevOptions;

/// What is read of the input beside the model: its cards, and its FMESH
/// cards and the numbers it uses.
type SourceRead = (
    Option<Arc<llevant_scene::source::Source>>,
    Option<Arc<InputMeshes>>,
    Option<Arc<Labels>>,
);

/// Writing the input to a file: its modification time after.
type SaveTask = Task<Result<std::time::SystemTime, String>>;

pub struct ViewerApp {
    /// The window's device: the views' images are made on it.
    gpu: Arc<Gpu>,
    render_state: egui_wgpu::RenderState,
    path: PathBuf,
    /// Where "Save view" writes.
    state_path: PathBuf,
    /// The view to open the model at.
    initial: ViewState,
    backend: BackendChoice,
    budget_ms: f64,
    /// The model being loaded (its progress the start screen's bar and
    /// Cancel).
    loading: Option<Task<Result<loading::Load, String>>>,
    /// The load under way reloads the model shown, which stays shown until
    /// the new one lands.
    reloading: bool,
    /// The cards of the input edited here, and the text as edited.
    edits: input_edits::InputEdits,
    /// What to select once the edit under way lands (the cells it edited).
    select_after: Option<Query>,
    /// The inspector's page to open once it lands (a new card's).
    open_after: Option<crate::panels::inspector::Subject>,
    /// The load under way compiles an edit of the input.
    edit_under_way: bool,
    /// The edited input being written to a file: after how many edits, and
    /// the file (another one: Save as).
    saving: Option<(usize, PathBuf, SaveTask)>,
    /// The octrees of the whole model being built (it was shown staged),
    /// for the scene they are being prepared at.
    completing: Option<(Scene, Task<Result<Loaded, String>>)>,
    /// What is remembered between runs; `None` (tests, embedding) forgets.
    settings: Option<Settings>,
    /// The input's modification time when loaded, and whether it changed
    /// since (checked every few seconds).
    watched: Option<(std::time::SystemTime, std::time::Instant)>,
    input_changed: bool,
    /// The input's cards (for the inspector), read beside the model.
    source: Option<Arc<llevant_scene::source::Source>>,
    /// The input's FMESH cards and the numbers it uses (read beside its
    /// cards).
    input_meshes: Option<Arc<InputMeshes>>,
    source_loading: Option<Task<SourceRead>>,
    /// The cells' labels (`$` comments), read with the cards; given to each
    /// document's rules ([`ViewerApp::give_labels`]).
    labels: Option<Arc<Labels>>,
    /// A universe being opened on its own, within the open model.
    opening: Option<(i64, Task<Result<Loaded, String>>)>,
    error: Option<String>,
    doc: Option<Document>,
    /// The models the open universe came from, the whole model first.
    parents: Vec<Document>,
    tools: Toolbox,
    theme: egui::ThemePreference,
    /// The keys window is open.
    show_keys: bool,
    /// The workspace shown, how each is laid out, the view maximised.
    frame: frame::Frame,
    /// The panels of the sides.
    panels: frame::Panels,
    /// The workspace of the frame before: leaving Tally stops drawing a mesh.
    last_workspace: frame::Workspace,
    palette: Palette,
    fps: f64,
    dev: DevHooks,
    /// To wake the window when work done elsewhere lands.
    egui_ctx: egui::Context,
    /// A surface being dragged, previewed live.
    dragging: Option<drag::Dragging>,
}

impl ViewerApp {
    /// For eframe's app creator: opens `path` in the background at the view
    /// `state`; "Save view" writes to `state_path` (default
    /// `<path>.view.json`).
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        path: PathBuf,
        state: ViewState,
        state_path: Option<PathBuf>,
    ) -> Result<ViewerApp, String> {
        let rs = cc
            .wgpu_render_state
            .clone()
            .ok_or("the viewer needs eframe's wgpu renderer")?;
        // egui panics on a lost device; say why first (and `run` turns
        // the panic into an error).
        rs.device.set_device_lost_callback(|reason, message| {
            eprintln!("llevant-viewer: GPU window device lost ({reason:?}): {message}");
            WINDOW_LOST.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        theme::apply(&cc.egui_ctx);
        // Dark by default; light and the system's are a click away (the
        // theme button, top right).
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        let gpu = Arc::new(Gpu::from_parts(
            rs.device.clone(),
            rs.queue.clone(),
            rs.adapter.get_info(),
        ));
        let state_path = state_path.unwrap_or_else(|| default_state_path(&path));
        let mut app = ViewerApp {
            settings: None,
            watched: None,
            input_changed: false,
            gpu,
            render_state: rs,
            path,
            state_path,
            backend: state.backend,
            budget_ms: state.budget_ms,
            initial: state,
            loading: None,
            reloading: false,
            edits: Default::default(),
            select_after: None,
            open_after: None,
            edit_under_way: false,
            saving: None,
            completing: None,
            source: None,
            input_meshes: None,
            source_loading: None,
            labels: None,
            opening: None,
            error: None,
            doc: None,
            parents: Vec::new(),
            tools: Toolbox::standard(),
            theme: egui::ThemePreference::Dark,
            show_keys: false,
            frame: frame::Frame::default(),
            panels: frame::Panels::default(),
            last_workspace: frame::Workspace::Explore,
            palette: Palette::default(),
            fps: 0.0,
            dev: DevHooks::new(&DevOptions::default()),
            egui_ctx: cc.egui_ctx.clone(),
            dragging: None,
        };
        if !app.path.as_os_str().is_empty() {
            app.start_loading(&cc.egui_ctx, None);
        }
        Ok(app)
    }

    /// With the hooks for tests and benchmarks of `options`.
    pub fn with_dev(mut self, options: &DevOptions) -> ViewerApp {
        self.dev = DevHooks::new(options);
        self
    }

    /// For eframe's app creator: the start screen, no model open yet.
    pub fn empty(cc: &eframe::CreationContext<'_>) -> Result<ViewerApp, String> {
        ViewerApp::new(cc, PathBuf::new(), ViewState::new(), None)
    }

    /// Remember recent files, the theme and the workspaces' layout in `s`
    /// (saved on changes and on exit); applies the theme and layout now.
    pub fn use_settings(&mut self, s: Settings, ctx: &egui::Context) {
        self.theme = s.theme();
        ctx.set_theme(self.theme);
        if let Some(f) = &s.frame {
            self.frame = f.clone().normalized();
            self.last_workspace = self.frame.workspace;
        }
        if !self.path.as_os_str().is_empty() {
            let mut s = s;
            s.opened(&self.path);
            s.save();
            self.settings = Some(s);
        } else {
            self.settings = Some(s);
        }
    }

    /// Open `path` (closing what is open), at its saved view if it has one.
    pub fn open_file(&mut self, path: PathBuf, ctx: &egui::Context) {
        let state_path = default_state_path(&path);
        let mut state = if state_path.exists() {
            ViewState::load(&state_path).unwrap_or_default()
        } else {
            ViewState::new()
        };
        state.backend = self.backend;
        state.budget_ms = self.budget_ms;
        self.initial = state;
        self.path = path;
        self.state_path = state_path;
        // The edits were of the input closed.
        self.edits = Default::default();
        if let Some(s) = &mut self.settings {
            s.opened(&self.path);
            s.save();
        }
        self.start_loading(ctx, None);
    }

    /// Read the model again (it changed on disk), at the view as it is now.
    /// Only the universes that changed are built again; with only the whole
    /// model open, it stays shown (and its undo history, selection and
    /// tools' state stay) until the new one replaces it.
    pub fn reload(&mut self, ctx: &egui::Context) {
        if self.path.as_os_str().is_empty() || self.loading.is_some() {
            return;
        }
        // The file as it is: edits made here and not saved are dropped.
        self.edits = Default::default();
        let base = self.parents.first().or(self.doc.as_ref());
        let base = base
            .filter(|d| d.universe.is_none())
            .map(|d| d.model.clone());
        if self.doc.is_some() {
            self.initial = self.view_state();
        }
        self.reloading =
            self.parents.is_empty() && self.doc.as_ref().is_some_and(|d| d.universe.is_none());
        if let Some(d) = self.doc.as_mut().filter(|_| self.reloading) {
            d.messages
                .progress("reloading: building only what changed…");
        }
        self.start_loading(ctx, base);
    }

    /// Run `c` as if a panel had asked for it (tests, embedding).
    pub fn send(&mut self, c: Command, ctx: &egui::Context) {
        self.execute(vec![c], ctx);
    }

    /// The edits of the input made here (tests).
    pub fn input_edits(&self) -> (usize, bool) {
        (self.edits.len(), self.edits.unsaved())
    }

    /// Whether a model is being loaded (or reloaded).
    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Whether the octrees of the model shown are still being built.
    pub fn is_completing(&self) -> bool {
        self.completing.is_some()
    }

    /// Whether the input changed on disk since it was loaded.
    pub fn input_changed(&self) -> bool {
        self.input_changed
    }

    /// Look at the input on disk now rather than at the watcher's next
    /// check (every 2 s): for tests, which would otherwise wait for it.
    #[doc(hidden)]
    pub fn check_input_now(&mut self) {
        if let Some((_, checked)) = &mut self.watched {
            if let Some(t) = checked.checked_sub(std::time::Duration::from_secs(2)) {
                *checked = t;
            }
        }
        self.watch();
    }

    /// The open model, to change (tests).
    pub fn document_mut(&mut self) -> Option<&mut Document> {
        self.doc.as_mut()
    }

    /// The model file being shown or loaded (empty for none).
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The tools, and which is active.
    pub fn tools(&self) -> &Toolbox {
        &self.tools
    }

    /// The workspaces and how their sides are arranged.
    pub fn frame(&self) -> &frame::Frame {
        &self.frame
    }

    /// The open model, once loaded.
    pub fn document(&self) -> Option<&Document> {
        self.doc.as_ref()
    }

    /// Why the model could not be opened, if it could not.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The view as it is now (as "Save view" writes it).
    pub fn view_state(&self) -> ViewState {
        match &self.doc {
            Some(d) => ViewState {
                open: self.open_universes(),
                ..d.view_state(self.backend, self.budget_ms)
            },
            None => self.initial.clone(),
        }
    }
}

/// The 3D view in `ui`'s space: the active tool, then navigation, then the
/// image and the overlays.
fn view3d_ui(
    ui: &mut egui::Ui,
    doc: &mut Document,
    tools: &mut Toolbox,
    rs: &egui_wgpu::RenderState,
    out: &mut Vec<Command>,
) {
    let size = ui.available_size().max(egui::vec2(16.0, 16.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "3D view"));
    doc.view3d_height = rect.height();
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = (
        ((rect.width() * ppp) as usize).clamp(8, 8192),
        ((rect.height() * ppp) as usize).clamp(8, 8192),
    );
    let cut = doc.cut3d();
    let mut vp = ViewPort {
        kind: ViewKind::ThreeD,
        response,
        projection: Projection::Camera(doc.orbit.camera(w, h, cut.clone())),
    };
    tools.input(&vp, doc, out);
    navigate_3d(ui, &vp, doc);
    let camera = doc.orbit.camera(w, h, cut);
    vp.projection = Projection::Camera(camera.clone());
    timed("3D hover", || {
        hover(ui, &vp, doc, HoverKey::camera(doc.orbit.state), out)
    });
    menu(&vp, doc, out);

    // A geometry being edited: its surfaces and the cell as edited.
    let focus = doc.focus.as_ref().map(|f| f.point);
    let (surfaces, shell) = doc.marks.for_3d(&doc.model, focus, &camera, ui.ctx());
    doc.view3d.set_edit_marks(surfaces, shell);
    let shown = timed("3D frame", || doc.view3d.frame(rs, camera));
    if let Some(n) = doc.view3d.take_notice() {
        doc.messages.warn(n);
    }
    let painter = ui.painter_at(rect);
    match shown {
        Some(tex) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(tex, rect, uv, egui::Color32::WHITE);
        }
        None => {
            let [r, g, b] = theme::palette(ui.ctx()).view;
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(r, g, b));
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "tracing…",
                egui::FontId::proportional(16.0),
                egui::Color32::GRAY,
            );
        }
    }
    overlay::draw(&vp, doc, &painter);
    tools.overlay(&vp, doc, &painter);
    crate::gizmo::ui(ui, rect, &mut doc.orbit);
    if doc.view3d.busy() {
        ui.ctx().request_repaint();
    }
}

/// The slice in `ui`'s space (it pans and zooms itself), then the active
/// tool and the overlays.
fn slice_ui(ui: &mut egui::Ui, doc: &mut Document, tools: &mut Toolbox, out: &mut Vec<Command>) {
    // The left button belongs to the tools; the middle and right pan.
    let ev = timed("slice", || doc.slice.ui_with(ui, false));
    doc.pointer.hovered = ev.hovered;
    let vp = ViewPort {
        kind: ViewKind::Slice,
        response: ev.response,
        projection: Projection::Plane(ev.plane),
    };
    tools.input(&vp, doc, out);
    timed("slice hover", || {
        hover(ui, &vp, doc, HoverKey::plane(ev.plane), out)
    });
    menu(&vp, doc, out);
    let painter = ui.painter_at(vp.rect());
    overlay::draw(&vp, doc, &painter);
    tools.overlay(&vp, doc, &painter);
}

/// What is under the pointer over `vp`, picked again when the pointer or
/// the view moves (not while a button is down).
fn hover(ui: &egui::Ui, vp: &ViewPort, doc: &mut Document, key: HoverKey, out: &mut Vec<Command>) {
    let Some(pos) = vp.response.hover_pos() else {
        if doc
            .pointer
            .hover_key
            .as_ref()
            .is_some_and(|k| k.kind == vp.kind)
        {
            doc.pointer.hover_key = None;
            out.push(Command::Hover(None));
        }
        return;
    };
    if ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let key = HoverKey { pos, ..key };
    if doc.pointer.hover_key.as_ref() == Some(&key) {
        return;
    }
    doc.pointer.hover_key = Some(key.clone());
    // Picked on the document's picker (it can take a while on large
    // models); the result is shown if the pointer is still there.
    let pick = vp.picker(pos);
    let (m, vis) = (doc.model.clone(), doc.vis.clone());
    doc.pointer.picker.send(move || (key, pick(&m, &vis)));
}

/// The context menu of a right-click on `vp`: what is under the click.
fn menu(vp: &ViewPort, doc: &mut Document, out: &mut Vec<Command>) {
    if vp.response.secondary_clicked() {
        doc.pointer.context_target = vp
            .response
            .interact_pointer_pos()
            .and_then(|pos| vp.pick(&doc.model, &doc.vis, pos))
            .map(|p| p.with_through(&doc.model));
    }
    let doc: &Document = doc;
    vp.response.context_menu(|ui| {
        context::view_menu(ui, doc, doc.pointer.context_target.as_ref(), out);
    });
}

impl eframe::App for ViewerApp {
    fn on_exit(&mut self) {
        let f = self.frame.clone();
        if let Some(s) = &mut self.settings {
            s.frame = Some(f);
            s.save();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let own = std::time::Instant::now();
        let dt = ui.input(|i| i.unstable_dt) as f64;
        if dt > 0.0 {
            self.fps = 0.9 * self.fps + 0.1 / dt;
        }
        if ui.input(|i| i.viewport().close_requested()) && !self.may_drop_edits("Quit") {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.poll_loading();
        self.poll_drag();
        self.poll_saving();
        self.poll_completing();
        let editable = self.edit_blocked().map_or(Ok(()), Err);
        if let Some(d) = &mut self.doc {
            d.input_editable = editable;
        }
        self.poll_source();
        self.poll_opening();
        self.give_labels();
        if let Some(d) = &mut self.doc {
            if d.input.is_none() {
                d.input = Some(self.path.clone());
            }
            if d.poll_jobs() {
                // Progress bars move.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        let mut spans = Spans::new(own);
        spans.mark("polls");
        let mut out = Vec::new();
        let storm = self.dev.storm_frame(ui.ctx(), self.doc.as_ref());
        out.extend(storm.commands);
        if let Some(t) = storm.palette {
            self.palette.set_text(&t);
        }
        if let Some(r) = &storm.find_rule {
            self.panels.find.set_rule(r);
        }
        toolbar::shortcuts(ui.ctx(), &mut self.tools, &mut out);
        if let Some(d) = &self.doc {
            context::shortcuts(ui.ctx(), d, &mut out);
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.show_keys = !self.show_keys;
        }
        keys::window(ui.ctx(), &mut self.show_keys);
        if let Some(d) = &self.doc {
            if d.ruler.is_some() || self.tools.active().name() == "Ruler" {
                crate::panels::ruler::window(ui.ctx(), d, &mut out);
            }
        }
        self.frame_keys(ui.ctx(), &mut out);

        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            self.palette.toggle();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
            out.push(Command::SaveView);
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            out.push(Command::OpenDialog);
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R)) {
            out.push(Command::Reload);
        }
        self.watch();
        if self.input_changed
            && self.loading.is_none()
            && !self.edits.unsaved()
            && self.settings.as_ref().is_some_and(|s| s.auto_reload)
        {
            out.push(Command::Reload);
        }
        self.palette.ui(ui.ctx(), self.doc.as_ref(), &mut out);

        spans.mark("windows");
        let theme_before = self.theme;
        egui::Panel::top("menu").show(ui, |ui| {
            let recent = self.settings.as_ref().map_or(&[][..], |s| &s.recent[..]);
            let doc = self.doc.as_ref();
            let (layout, theme, keys) = (&self.frame, &mut self.theme, &mut self.show_keys);
            menu::ui(ui, doc, layout, recent, &mut out, |ui, out| {
                if doc.is_some() {
                    ui.separator();
                    frame::workspace_tabs(ui, layout, out);
                }
                toolbar::bar_right(ui, doc, theme, keys, out);
            });
        });
        if self.theme != theme_before {
            if let Some(s) = &mut self.settings {
                s.set_theme(self.theme);
                s.save();
            }
        }
        if !self.edits.is_empty() {
            self.edits_banner(ui, &mut out);
        }
        if self.input_changed && self.loading.is_none() {
            egui::Panel::top("changed").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} changed on disk.",
                            self.path
                                .file_name()
                                .map_or(String::new(), |n| n.to_string_lossy().into())
                        ))
                        .color(theme::SELECTION),
                    );
                    let tip = if self.edits.unsaved() {
                        "Ctrl+R: read it again, keeping the view (the edits made here and not saved are dropped)"
                    } else {
                        "Ctrl+R: read it again, keeping the view"
                    };
                    if ui
                        .button("Reload")
                        .on_hover_text(tip)
                        .clicked()
                    {
                        out.push(Command::Reload);
                    }
                    if ui.button("Not now").clicked() {
                        self.input_changed = false;
                        self.watched = None;
                    }
                    if let Some(s) = &mut self.settings {
                        if ui
                            .button("Always reload")
                            .on_hover_text(
                                "Reload whenever the input changes on disk, from now on: \
                                 only what changed is built again",
                            )
                            .clicked()
                        {
                            s.auto_reload = true;
                            s.save();
                            out.push(Command::Reload);
                        }
                    }
                });
            });
        }
        let hint = self.tools.hint(self.doc.as_ref());
        let busy = status::Busy {
            working: self.loading.is_some()
                || self.saving.is_some()
                || self.opening.is_some()
                || self
                    .doc
                    .as_ref()
                    .is_some_and(|d| d.field_loading().is_some()),
            building: self
                .completing
                .as_ref()
                .map(|(_, t)| t.started().elapsed().as_secs_f64()),
        };
        egui::Panel::bottom("status").show(ui, |ui| {
            status::ui(
                ui,
                self.fps,
                self.doc.as_ref(),
                hint,
                self.backend,
                self.budget_ms,
                busy,
                &mut out,
            );
        });
        if let Some(doc) = &mut self.doc {
            let view = theme::palette(ui.ctx()).view;
            doc.view3d.set_background(view);
            doc.slice.background = view;
        }
        self.dev
            .step(ui.ctx(), self.doc.as_mut(), self.error.is_some(), self.fps);

        let fill = theme::palette(ui.ctx()).base;
        // A model file dropped on the window opens.
        let dropped = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|p| !p.as_os_str().is_empty())
        });
        if let Some(p) = dropped {
            // Results go over the open model; anything else is a model.
            if self.doc.is_some() && is_field_file(&p) {
                out.push(Command::LoadField(crate::state::FieldSource::new(p)));
                out.push(Command::ShowPanel(frame::Panel::Fields));
            } else {
                out.push(Command::OpenFile(p));
            }
        }
        let recent = self
            .settings
            .as_ref()
            .map_or(Vec::new(), |s| s.recent.clone());
        let (error, path) = (&self.error, &self.path);
        let load = self
            .loading
            .as_ref()
            .map(|t| (t.progress().clone(), t.started()));
        spans.mark("bars");
        if self.doc.is_some() {
            if let Some(e) = error {
                let e = e.clone();
                egui::Panel::top("error").show(ui, |ui| {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                });
            }
            self.workspace_ui(ui, &mut out);
        } else {
            egui::CentralPanel::no_frame()
                .frame(egui::Frame::NONE.fill(fill))
                .show(ui, |ui| match &load {
                    Some((p, started)) => start::loading(ui, path, p, *started),
                    _ => start::screen(ui, error.as_deref(), &recent, &mut out),
                });
        }
        spans.mark("workspace");
        let ctx = ui.ctx().clone();
        let ran = log::log_enabled!(target: FRAMES, log::Level::Debug).then(|| {
            out.iter()
                .map(|c| format!("{c:?}").chars().take(80).collect::<String>())
                .collect::<Vec<_>>()
        });
        self.execute(out, &ctx);
        self.sync_tool_and_workspace();
        if let Some(d) = &mut self.doc {
            d.camera_history
                .observe(d.orbit.state, std::time::Instant::now());
            if d.camera_history.settling() {
                ctx.request_repaint_after(std::time::Duration::from_millis(650));
            }
        }
        spans.mark("commands");
        // The frame-time guard: the UI thread's own work (not painting)
        // should stay far from a frame.
        let own = own.elapsed();
        if own > FRAME_BUDGET {
            log::debug!(
                target: FRAMES,
                "{:.1} ms of own work in a frame ({}); commands: {:?}",
                1e3 * own.as_secs_f64(),
                spans.text(),
                ran.unwrap_or_default()
            );
        }
        self.dev.frame_end(&ctx, own, self.doc.as_ref());
    }
}

/// Where a frame's own work went: the time to each mark from the one
/// before.
struct Spans {
    last: std::time::Instant,
    spans: Vec<(&'static str, f64)>,
}

impl Spans {
    fn new(start: std::time::Instant) -> Spans {
        Spans {
            last: start,
            spans: Vec::new(),
        }
    }

    fn mark(&mut self, name: &'static str) {
        let now = std::time::Instant::now();
        self.spans
            .push((name, 1e3 * (now - self.last).as_secs_f64()));
        self.last = now;
    }

    fn text(&self) -> String {
        let v: Vec<String> = self
            .spans
            .iter()
            .map(|(n, ms)| format!("{n} {ms:.1}"))
            .collect();
        v.join(", ")
    }
}

/// `f`, logged with the frames over budget if it takes more than 2 ms.
fn timed<R>(name: &str, f: impl FnOnce() -> R) -> R {
    let start = std::time::Instant::now();
    let r = f();
    let ms = 1e3 * start.elapsed().as_secs_f64();
    if ms > 2.0 {
        log::debug!(target: FRAMES, "{name}: {ms:.1} ms");
    }
    r
}

/// The log target of frames over [`FRAME_BUDGET`]
/// (`RUST_LOG=llevant_viewer::frames=debug`).
const FRAMES: &str = "llevant_viewer::frames";

/// UI work in a frame beyond which it is logged: half a frame at 60 Hz.
const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);

/// The scene a universe opened on its own starts with: the model's, without
/// its clip plane, pins, field and meshes (all in the model's frame and
/// numbering).
fn opened_scene(s: &Scene) -> Scene {
    Scene {
        clip: None,
        cuts: Vec::new(),
        pins: Vec::new(),
        field: None,
        meshes: Vec::new(),
        ..s.clone()
    }
}

/// Whether a file dropped on the window holds a field rather than a model:
/// a CSV file, or a tally file (meshtal, MCTAL or D1SUNED cuvmsh, by name
/// or by its first bytes; a meshinfo too, which the fields panel refuses,
/// saying to open its cuvmsh).
fn is_field_file(p: &std::path::Path) -> bool {
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("csv" | "msht" | "meshtal" | "mctal")) {
        return true;
    }
    matches!(llevant_scene::field::tally_format(p), Ok(Some(_)))
}

/// Write FMESH cards to a file picked in a dialog (beside the input,
/// `<input>.fmesh.i` by default); what happened.
fn save_cards(input: &std::path::Path, text: &str) -> Result<String, String> {
    let mut d = rfd::FileDialog::new().set_title("Save the FMESH cards");
    if let Some(dir) = input.parent().filter(|d| !d.as_os_str().is_empty()) {
        d = d.set_directory(dir);
    }
    if let Some(name) = input.file_stem() {
        d = d.set_file_name(format!("{}.fmesh.i", name.to_string_lossy()));
    }
    match d.save_file() {
        Some(p) => match std::fs::write(&p, text) {
            Ok(()) => Ok(format!("FMESH cards saved to {}", p.display())),
            Err(e) => Err(format!("{}: {e}", p.display())),
        },
        None => Ok("not saved".into()),
    }
}

/// Write the input as `source` has it to `path`, card by card, whole or not
/// at all (`Source::save_to`); the file's modification time after.
fn write_input(
    source: &llevant_scene::source::Source,
    path: &std::path::Path,
) -> std::io::Result<std::time::SystemTime> {
    source.save_to(path)?;
    std::fs::metadata(path)?.modified()
}

/// `<model>.view.json`, where "Save view" writes by default.
fn default_state_path(model: &std::path::Path) -> PathBuf {
    let mut p = model.as_os_str().to_owned();
    p.push(".view.json");
    PathBuf::from(p)
}

/// Cells changed by a reload beyond this many are not selected.
const SELECT_CHANGED_MAX: usize = 20_000;

/// Lines of the check of what a reload changed, started with it.
const CHECK_CHANGED_LINES: f64 = 1e5;

/// The status line after a reload; selects the cells whose geometry changed
/// (new, moved or reshaped) so that the views show them.
fn reloaded_message(
    doc: &mut Document,
    changed: Option<&llevant::Changed>,
    seconds: f64,
) -> String {
    let Some(c) = changed else {
        return format!("reloaded in {seconds:.1} s");
    };
    let removed = match c.cells_removed.len() {
        0 => String::new(),
        n => format!(", {n} removed"),
    };
    let rebuilt = c.universes_rebuilt.len();
    let n = c.cells_changed.len();
    if n == 0 {
        return format!(
            "reloaded in {seconds:.1} s: no cell's geometry changed{removed} \
             ({rebuilt} universes rebuilt)"
        );
    }
    let selected = n <= SELECT_CHANGED_MAX
        && doc
            .selection
            .set(
                Some(llevant_scene::query::Query::Cells(c.cells_changed.clone())),
                &doc.model,
                &doc.index,
            )
            .is_ok();
    format!(
        "reloaded in {seconds:.1} s: {n} cells changed{}{removed} ({rebuilt} universes rebuilt)",
        if selected { " (selected)" } else { "" }
    )
}

/// The document a surface is being dragged in: the one shown, or one of
/// those it was opened from.
fn dragged<'a>(
    doc: &'a mut Option<Document>,
    parents: &'a mut [Document],
) -> Option<&'a mut Document> {
    doc.iter_mut()
        .chain(parents.iter_mut())
        .find(|d| d.drag.is_some())
}
