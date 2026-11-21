//! An open model and everything about looking at it: the scene (what is
//! shown, with its undo history), what is selected, the point the inspector
//! describes, what is under the pointer, and the two views. Tools and
//! panels read it and send [`Command`]s; [`Document::execute`] runs them
//! and brings the views in line, touching only what changed: the selection
//! and the hovered cell are drawn over what the views traced, without
//! tracing again.

use std::collections::HashMap;
use std::sync::Arc;

use llevant::{Aabb, Model, Vec3, Visibility};
use llevant_gpu::{Gpu, GpuModel};
use llevant_render::colours::ColourBy;
use llevant_render::field::Overlay;
use llevant_scene::field::{
    self, BinValues, Field, FileSummary, Format, Input, TallyChoice, TallyFile,
};
use llevant_scene::index::{CellSet, ModelIndex};
use llevant_scene::materials::Looks;
use llevant_scene::query::Query;

use crate::app::loading::{self, GpuLater, GpuPair};
use crate::camera::Orbit;
use crate::command::{Command, Edit, History};
use crate::derive::{Derived, Deriver};
use crate::jobs::{Alike, CheckRun, Job, JobKind, Outcome, VolumeRun};
use crate::pick::{pick_point, Picked};
use crate::scene::{Changes, Scene};
use crate::selection::{selection_box, SelectMode, Selection};
use crate::slice::{PlaneView, SliceView};
use crate::state::{BackendChoice, FieldSource, FieldState, SavedView, ViewState};
use crate::tasks::{Latest, Task};
use crate::view3d::backend::{CpuBackend, GpuBackend, Shown, TraceBackend};
use crate::view3d::{Prepared3d, View3d};

/// Where the pointer was over which view when what is under it was last
/// picked: picked again when any of it changes.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverKey {
    pub kind: crate::viewport::ViewKind,
    pub pos: egui::Pos2,
    pub camera: Option<crate::state::CameraState>,
    pub plane: Option<PlaneView>,
}

impl HoverKey {
    pub fn camera(c: crate::state::CameraState) -> HoverKey {
        HoverKey {
            kind: crate::viewport::ViewKind::ThreeD,
            pos: egui::Pos2::ZERO,
            camera: Some(c),
            plane: None,
        }
    }

    pub fn plane(p: PlaneView) -> HoverKey {
        HoverKey {
            kind: crate::viewport::ViewKind::Slice,
            pos: egui::Pos2::ZERO,
            camera: None,
            plane: Some(p),
        }
    }
}

/// A model loaded (`app::loading`), with its GPU copy when there is one.
pub struct Loaded {
    pub model: Arc<Model>,
    /// Who holds what in it.
    pub index: Arc<ModelIndex>,
    /// The universe opened on its own (`Model::universe`), or `None` for
    /// the whole model.
    pub universe: Option<i64>,
    /// What the views frame.
    pub region: Aabb,
    /// The model on a device of its own (see `view3d::backend`), or why
    /// not (`loading::GPU_NOT_YET` when it is to be made later).
    pub gpu_model: Result<(Arc<Gpu>, Arc<GpuModel>), String>,
    /// How to make the GPU copy, when it was not made with the model: the
    /// document makes it in the background when its 3D view wants the GPU,
    /// tracing on the CPU meanwhile (`Document::poll_gpu`).
    pub gpu_later: Option<GpuLater>,
    /// The scene's visibility made ready for the backends.
    pub shown: Shown,
    /// The scene's materials' looks.
    pub looks: Arc<Looks>,
    /// What the scene shows, for the panels' eyes.
    pub seen: Arc<crate::seen::Seen>,
    /// What was wrong with the scene (its visibility or looks; the model is
    /// shown solid, or with the materials' own looks).
    pub problems: Vec<String>,
    /// The 3D view's pipelines and palette.
    pub view3d: Prepared3d,
    pub seconds: f64,
}

pub struct Document {
    // The model and what was read beside it.
    /// The universe opened on its own, or `None` for the whole model.
    pub universe: Option<i64>,
    pub model: Arc<Model>,
    /// Who holds what in the model.
    pub index: Arc<ModelIndex>,
    /// For a universe opened on its own: the whole model and its index
    /// (surfaces and materials are shared with cells outside it).
    pub whole: Option<(Arc<Model>, Arc<ModelIndex>)>,
    /// The input's cards, once read (the application reads them beside the
    /// model).
    pub source: Option<Arc<llevant_scene::source::Source>>,
    /// What the materials look like (the scene's `materials`).
    pub looks: Arc<Looks>,
    pub region: Aabb,
    /// Universe levels of the model ([`Model::levels`]): the levels the
    /// scene can show.
    pub levels: usize,
    /// The model on a device of its own (see `view3d::backend`).
    pub gpu_model: Result<(Arc<Gpu>, Arc<GpuModel>), String>,
    // What is shown, and the views showing it.
    /// What is shown; change it with [`Edit`]s.
    pub scene: Scene,
    /// The cells the scene shows in 3D (the 3D view catches up when
    /// `shown` does).
    pub vis: Visibility,
    /// What the scene shows of each cell and material, and why not
    /// ([`crate::seen`]); made with the 3D view's visibility, so it catches
    /// up with `vis` when the 3D view does.
    pub seen: Arc<crate::seen::Seen>,
    /// The visibility the 3D view shows, made ready for its backends.
    shown: Shown,
    /// Makes what the views need of each new scene, off this thread.
    deriver: Deriver,
    /// The window, to wake when work done elsewhere lands.
    pub(crate) ctx: egui::Context,
    pub history: History,
    /// The cells selected.
    pub selection: Selection,
    /// The point the inspector describes (the last one clicked).
    pub focus: Option<Picked>,
    /// The surface picked with Alt+click, marked in both views.
    pub surface_pick: Option<SurfacePick>,
    /// The overlap cut under way, shown in the inspector.
    pub cut_flow: Option<crate::cutflow::CutFlow>,
    /// What the pointer is over.
    pub pointer: Pointer,
    /// What the tools are editing.
    pub editing: Editing,
    /// Where each of the scene's pins is.
    pub pin_info: Vec<Picked>,
    /// The input's FMESH cards and the tally and TR numbers it uses, once
    /// read (the application reads them beside the model).
    pub input_meshes: Option<Arc<crate::mesh::InputMeshes>>,
    pub orbit: Orbit,
    /// The 3D view's height on screen (points), last drawn: handles keep
    /// their size on screen by it.
    pub view3d_height: f32,
    /// The ruler laid, and how the Ruler tool lays the next one.
    pub ruler: Option<crate::ruler::Ruler>,
    /// Surfaces marked on the slice (a cell's, while its geometry is
    /// edited).
    pub marks: crate::marks::Marks,
    pub ruler_mode: crate::ruler::RulerMode,
    /// The places the camera rested at (Alt+←, Alt+→).
    pub camera_history: crate::camera::CameraHistory,
    pub view3d: View3d,
    pub slice: SliceView,
    /// What happened, for the status bar and its log.
    pub messages: crate::messages::Messages,
    /// Draw the slice's outline and handles in the 3D view (off at first:
    /// it is drawn then only while selected or moved, see
    /// [`Document::slice_shown_in_3d`]).
    pub slice_in_3d: bool,
    /// Cut the 3D view at the slice plane (the side facing the eye).
    pub cut_at_slice: bool,
    /// The cells the rule `shown` gives, and of what ([`Document::shown_cells`]).
    shown_cache: std::sync::Mutex<Option<(ShownKey, Arc<CellSet>)>>,
    /// Works out the cells `shown` gives under cuts (seconds on the largest
    /// models), and what it was last asked for.
    shown_job: Latest<(ShownKey, Arc<CellSet>)>,
    shown_asked: std::sync::Mutex<Option<ShownKey>>,
    /// Draw the cuts (outlines and handles) in the 3D view. Off, they still
    /// cut, and only the cut selected (from the Section menu) is drawn.
    pub show_cuts: bool,
    /// Draw the problems found (the check's, as boxes; those the slice
    /// met, as markers) over the 3D view, through what hides them.
    pub problems_on_top: bool,
    /// Views saved by name (in the view file).
    pub views: Vec<SavedView>,
    /// The input file (for exports next to it), when known.
    pub input: Option<std::path::PathBuf>,
    /// The analyses: running, and their last results.
    pub results: Results,
    /// Where the last reload changed the geometry: global boxes of every
    /// instance of the cells changed or removed, before and after (a check
    /// over them asks "did that edit open a gap?").
    pub changed_boxes: Option<Arc<Vec<Aabb>>>,
    /// Whether the input's cards can be edited now (else why not); set by
    /// the application each frame.
    pub input_editable: std::result::Result<(), &'static str>,
    /// The field drawn over the cells (the scene's, with its scale and
    /// opacity), if shown.
    pub field: Option<Overlay>,
    /// The fields read and being read.
    fields: Fields,
    /// A surface being dragged (set by the application), for the
    /// inspector.
    pub drag: Option<DragInfo>,
    /// The GPU copy of the model when it is made after the model is shown.
    gpu: GpuPrep,
}

/// The GPU copy of a document's model made in the background
/// ([`Loaded::gpu_later`]), and who the 3D view should be traced by once it
/// lands.
struct GpuPrep {
    /// How to make it, until it is started.
    later: Option<GpuLater>,
    task: Option<Task<Result<GpuPair, String>>>,
    /// Whether the user was told it is taking long.
    slow_said: bool,
    /// The copy landed: the 3D view switches to it once the deriver has
    /// uploaded the visibility for it.
    switch: bool,
    /// What the 3D view is to be traced by (`software`: the window's
    /// adapter is a CPU one).
    choice: BackendChoice,
    software: bool,
}

/// After this long making the GPU copy, the user is told the 3D view
/// traces on the CPU meanwhile.
const GPU_SLOW_SECONDS: f64 = 10.0;

/// A surface being dragged, as the inspector shows it.
#[derive(Clone, Debug, Default)]
pub struct DragInfo {
    pub surface: i64,
    /// The coefficients asked for last.
    pub coeffs: Vec<f64>,
    /// The draft is ready (begun) and following.
    pub live: bool,
    /// Why the preview does not follow (a surface that cannot be dragged
    /// live, coefficients that change its shape).
    pub problem: Option<String>,
    /// Time of the last step (ms).
    pub step_ms: f64,
    /// Steps shown.
    pub steps: usize,
    /// Being written into the input and compiled.
    pub committing: bool,
    /// The scene's visibility changed: to be widened by the drag and shown.
    pub reshow: bool,
}

/// A surface picked in a view (Alt+click): the pick, with every surface
/// through its point ([`Picked::detailed`]), and which of them is meant.
/// Alt+clicking the same place again moves to the next one.
#[derive(Clone, Debug)]
pub struct SurfacePick {
    pub picked: Picked,
    pub at: usize,
}

impl SurfacePick {
    /// The compiled surface meant.
    pub fn shape(&self) -> Option<u32> {
        self.picked
            .through
            .get(self.at)
            .copied()
            .or(self.picked.surface.as_ref().map(|s| s.shape))
    }

    /// What to mark for it in both views.
    pub fn marks(&self, m: &llevant::Model) -> Option<crate::marks::MarkRequest> {
        crate::marks::MarkRequest::surface(m, &self.picked, self.shape()?)
    }
}

/// What the pointer is over, in either view.
pub struct Pointer {
    /// What is under it (picked on `picker`).
    pub hover: Option<Picked>,
    /// Where it was picked.
    pub hover_key: Option<HoverKey>,
    /// Picks what is under the pointer, where it was ([`HoverKey`]).
    pub picker: Latest<(HoverKey, Option<Picked>)>,
    /// The model point under it in the slice.
    pub hovered: Option<Vec3>,
    /// What the open context menu is about.
    pub context_target: Option<Picked>,
    /// What the drag in progress orbits about (the point under the cursor
    /// where it started).
    pub drag_pivot: Option<Vec3>,
}

/// What the handles in the views are on ([`crate::tools`]): the cut and the
/// mesh selected, and whether the next drag in the slice draws a mesh.
#[derive(Default)]
pub struct Editing {
    /// The cut selected (an index of the scene's cuts): its rings shown, and
    /// it shown in detail in the 3D view's Section menu.
    pub cut_selected: Option<usize>,
    /// The slice plane, as shown in 3D, selected: its rings shown.
    pub slice_selected: bool,
    /// The slice's position (or angle) being dragged or typed in its
    /// header: the plane shown in 3D meanwhile.
    pub slice_moving: bool,
    /// The mesh selected (an index of the scene's meshes): its handles shown
    /// in Tally, and it shown in detail in the FMESH panel.
    pub mesh_selected: Option<usize>,
    /// The next drag in the slice (in Tally) draws a new mesh: a cylinder
    /// (`Some(true)`) or a box (`Some(false)`).
    pub drawing_mesh: Option<bool>,
}

/// The analyses ([`crate::jobs`]): those running, and the last result of
/// each kind.
#[derive(Default)]
pub struct Results {
    pub jobs: Vec<Job>,
    pub volumes: Option<Arc<VolumeRun>>,
    pub check: Option<Arc<CheckRun>>,
    /// How many checks have finished (`check` is the last one).
    pub checks: u64,
    pub alike: Option<Arc<Alike>>,
}

/// The fields read, by where they came from, and the one being read.
#[derive(Default)]
struct Fields {
    /// As read, scale fitted.
    data: HashMap<FieldSource, Overlay>,
    /// The sources read, in the order they were (the Fields panel lists
    /// them to switch between).
    order: Vec<FieldSource>,
    /// The value and error of each cell bin of the MCTAL tallies read.
    bins: HashMap<FieldSource, Arc<BinValues>>,
    /// A field being read; with `true`, made the scene's field when read
    /// (an edit), else only drawn (the scene already names it).
    loading: Option<FieldRead>,
    /// The field whose reading failed last (not tried again by itself),
    /// and why.
    failed: Option<(FieldSource, String)>,
    /// Tally files indexed, so that another tally or bin of one reads
    /// only that.
    tallies: TallyFiles,
}

/// A field being read: where from, whether it becomes the scene's when
/// read, and the reading.
type FieldRead = (
    FieldSource,
    bool,
    Task<Result<(Overlay, Option<BinValues>), String>>,
);

/// What the cells `shown` gives depend on: the model, what the scene
/// shows, what the 3D view cuts away and the region it frames.
#[derive(Clone, PartialEq)]
struct ShownKey {
    model: usize,
    seen: usize,
    cut: llevant::Cutaway,
    region: Aabb,
}

/// The cells `shown` gives ([`Document::shown_cells`]): those `seen` draws
/// with some part `cut` leaves within `region`.
fn shown_cells(
    m: &Model,
    seen: &crate::seen::Seen,
    cut: &llevant::Cutaway,
    region: Aabb,
) -> CellSet {
    let drawn = seen.drawn(m);
    if cut.is_empty() {
        return drawn;
    }
    let cells: Vec<u32> = drawn.iter().collect();
    let kept = m.cells_kept(&cells, region, cut);
    CellSet::from_cells(
        m.cells().len(),
        cells.iter().zip(kept).filter(|(_, k)| *k).map(|(&c, _)| c),
    )
}

impl Document {
    /// What the 3D view cuts away: the scene's clip plane and cuts, and
    /// with [`Document::cut_at_slice`] the side of the slice plane the eye
    /// is on.
    pub fn cut3d(&self) -> llevant::Cutaway {
        let mut cut = self.scene.cutaway();
        if self.cut_at_slice {
            let f = self.slice.frame();
            let (_, off) = f.coords(self.orbit.eye());
            let normal = if off >= 0.0 { f.n } else { f.n.map(|x| -x) };
            cut = cut.union(llevant::Cutaway::plane(llevant::Clip {
                point: f.origin,
                normal,
            }));
        }
        cut
    }

    fn shown_key(&self) -> ShownKey {
        ShownKey {
            model: Arc::as_ptr(&self.model) as usize,
            seen: Arc::as_ptr(&self.seen) as usize,
            cut: self.cut3d(),
            region: self.region,
        }
    }

    /// The cells the rule `shown` gives: those the 3D view draws
    /// ([`crate::seen::Seen::drawn`]) with some part its cuts leave
    /// ([`Model::cells_kept`]: conservative, a cell only just reaching past
    /// a cut, or below a lattice, may count). `None` while they are worked
    /// out on a thread of their own (under cuts: seconds on the largest
    /// models); the window is woken when they are ready. Made again only
    /// when what they depend on changes.
    pub fn shown_cells(&self) -> Option<Arc<CellSet>> {
        let key = self.shown_key();
        let mut cache = self.shown_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = self.shown_job.latest() {
            *cache = Some(done);
        }
        if let Some((k, set)) = cache.as_ref() {
            if *k == key {
                return Some(set.clone());
            }
        }
        if key.cut.is_empty() {
            let set = Arc::new(self.seen.drawn(&self.model));
            *cache = Some((key, set.clone()));
            return Some(set);
        }
        let mut asked = self.shown_asked.lock().unwrap_or_else(|e| e.into_inner());
        if asked.as_ref() != Some(&key) {
            *asked = Some(key.clone());
            let (m, seen) = (self.model.clone(), self.seen.clone());
            self.shown_job.send(move || {
                let set = shown_cells(&m, &seen, &key.cut, key.region);
                (key, Arc::new(set))
            });
        }
        None
    }

    /// [`Document::shown_cells`], waiting for them if they are being worked
    /// out (or working them out here).
    pub fn shown_cells_now(&self) -> Arc<CellSet> {
        if let Some(set) = self.shown_cells() {
            return set;
        }
        let key = self.shown_key();
        let set = Arc::new(shown_cells(&self.model, &self.seen, &key.cut, key.region));
        *self.shown_cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((key, set.clone()));
        set
    }

    /// The cells of `q`, `shown` being what the 3D view shows now (waiting
    /// for it if need be: [`Document::shown_cells_now`]).
    pub fn resolve(&self, q: &Query) -> llevant::Result<CellSet> {
        if q.uses_shown() {
            q.resolve_with(&self.model, &self.index, Some(&self.shown_cells_now()))
        } else {
            q.resolve(&self.model, &self.index)
        }
    }

    /// The cells of `q` if they can be had now: `None` while what `shown`
    /// gives is being worked out ([`Document::shown_cells`]).
    pub fn try_resolve(&self, q: &Query) -> Option<llevant::Result<CellSet>> {
        if q.uses_shown() {
            let shown = self.shown_cells()?;
            Some(q.resolve_with(&self.model, &self.index, Some(&shown)))
        } else {
            Some(q.resolve(&self.model, &self.index))
        }
    }

    /// `q` as the cells it gives now when it has a `shown` term (what is
    /// shown changes as the view does; a selection or a hiding rule made
    /// of it should not), `q` itself otherwise.
    pub fn freeze(&self, q: Query) -> Result<Query, String> {
        if !q.uses_shown() {
            return Ok(q);
        }
        let set = self.resolve(&q).map_err(|e| e.to_string())?;
        Ok(Query::Cells(
            set.iter().map(|c| self.model.cell_id(c)).collect(),
        ))
    }

    /// `c` with the rules it carries frozen ([`Document::freeze`]): those
    /// it selects, hides, isolates, ghosts or saves by.
    fn freeze_command(&self, c: Command) -> Result<Command, String> {
        Ok(match c {
            Command::Select(q, mode) => Command::Select(self.freeze(q)?, mode),
            Command::Edit(e) => Command::Edit(match e {
                Edit::Hide(q) => Edit::Hide(self.freeze(q)?),
                Edit::Unhide(q) => Edit::Unhide(self.freeze(q)?),
                Edit::Reveal(q, widen) => Edit::Reveal(self.freeze(q)?, widen),
                Edit::Isolate(Some(q)) => Edit::Isolate(Some(self.freeze(q)?)),
                Edit::Ghost(q) => Edit::Ghost(self.freeze(q)?),
                Edit::Unghost(q) => Edit::Unghost(self.freeze(q)?),
                Edit::SaveRule(r) => match r.rule.parse::<Query>() {
                    // Kept by its name (the rule as typed, by default).
                    Ok(q) if q.uses_shown() => Edit::SaveRule(crate::scene::SavedRule {
                        rule: self.freeze(q)?.to_string(),
                        ..r
                    }),
                    _ => Edit::SaveRule(r),
                },
                e => e,
            }),
            c => c,
        })
    }

    /// Whether the 3D view draws the slice plane and its handles: when
    /// asked to, and while it is selected or being moved from the slice's
    /// header.
    pub fn slice_shown_in_3d(&self) -> bool {
        self.slice_in_3d || self.editing.slice_selected || self.editing.slice_moving
    }

    /// The visibility the 3D view shows, as made for its backends.
    pub fn shown(&self) -> &Shown {
        &self.shown
    }

    /// Show `shown` (the document's own, widened by a drag) in the 3D
    /// view, the document's own staying what it shows otherwise.
    pub fn show_dragged(&mut self, shown: &Shown) {
        if let Err(e) = self.view3d.show(shown) {
            self.messages.warn(e.to_string());
        }
    }

    /// A step of a surface being dragged: the 3D view patched, the slice
    /// tracing the draft's model when one came with it.
    pub fn draft_step(&mut self, step: &crate::view3d::backend::DraftStep) {
        if let Err(e) = self.view3d.draft(step) {
            self.messages.warn(e.to_string());
        }
        if let Some(m) = &step.model {
            self.slice.set_model(m.clone());
        }
    }

    /// The drag ended without an edit: `undo` (its revert patch) written,
    /// the model and visibility of the document shown again.
    pub fn end_drag(&mut self, undo: Option<Arc<llevant::draft::DraftPatch>>) {
        if let Some(patch) = undo {
            let step = crate::view3d::backend::DraftStep {
                patch,
                model: Some(self.model.clone()),
            };
            if let Err(e) = self.view3d.draft(&step) {
                self.messages.warn(e.to_string());
            }
        }
        self.drag = None;
        self.slice.set_model(self.model.clone());
        let shown = self.shown.clone();
        self.show_dragged(&shown);
    }

    /// The document of a loaded model at the view `state`; problems with
    /// the state (unknown numbers, bad material specs) are reported in
    /// `messages` (a warning) and left out.
    pub fn open(l: Loaded, state: &ViewState, software: bool, ctx: &egui::Context) -> Document {
        // The loading thread made the visibility and looks.
        let mut problems = l.problems;
        let scene = state.scene.clone();
        let vis = l.shown.vis.clone();
        let looks = l.looks;
        // Until the GPU copy is made, on the CPU, and nothing to say.
        let choice = match l.gpu_later {
            Some(_) => BackendChoice::Cpu,
            None => state.backend,
        };
        let (backend, warning) = backend_for(&l.model, &l.gpu_model, &l.shown, choice, software);
        problems.extend(warning);
        let by: ColourBy = scene.colour_by.into();
        let view3d = View3d::with(
            l.view3d,
            l.model.clone(),
            looks.clone(),
            backend,
            &vis,
            state.budget_ms / 1e3,
        );
        let orbit = match state.camera {
            Some(c) => {
                let mut o = Orbit::fit(&l.region, 0.0, 0.0);
                o.state = c;
                o
            }
            None => Orbit::fit(&l.region, 35.0, 25.0),
        };
        let mut slice = SliceView::new(l.model.clone(), looks.clone(), l.region, &state.slice, by);
        slice.level = vis.level;
        let mut messages = crate::messages::Messages::default();
        if problems.is_empty() {
            messages.info(format!(
                "loaded {} cells in {:.1} s",
                l.model.cells().len(),
                l.seconds
            ));
        } else {
            messages.warn(problems.join("; "));
        }
        let wake = ctx.clone();
        let deriver = Deriver::new(
            l.model.clone(),
            l.index.clone(),
            l.gpu_model.clone().ok(),
            scene.clone(),
            looks.clone(),
            move || wake.request_repaint(),
        );
        let mut d = Document {
            shown: l.shown,
            drag: None,
            deriver,
            ctx: ctx.clone(),
            levels: l.model.levels(),
            pin_info: Vec::new(),
            universe: l.universe,
            selection: Selection::new(l.model.cells().len()),
            model: l.model,
            index: l.index,
            whole: None,
            source: None,
            looks,
            region: l.region,
            gpu_model: l.gpu_model,
            scene,
            vis,
            seen: l.seen,
            history: History::default(),
            focus: None,
            surface_pick: None,
            cut_flow: None,
            pointer: Pointer {
                hover: None,
                hover_key: None,
                picker: Latest::new("pick", Some(ctx.clone())),
                hovered: None,
                context_target: None,
                drag_pivot: None,
            },
            editing: Editing::default(),
            results: Results::default(),
            changed_boxes: None,
            input_editable: Err("the input's cards are being read"),
            fields: Fields::default(),
            input_meshes: None,
            orbit,
            camera_history: Default::default(),
            ruler: None,
            marks: Default::default(),
            ruler_mode: Default::default(),
            view3d_height: 600.0,
            view3d,
            slice,
            messages,
            slice_in_3d: false,
            cut_at_slice: false,
            shown_cache: Default::default(),
            shown_job: Latest::new("shown", Some(ctx.clone())),
            shown_asked: Default::default(),
            show_cuts: true,
            problems_on_top: false,
            views: state.views.clone(),
            input: None,
            field: None,
            gpu: GpuPrep {
                later: l.gpu_later,
                task: None,
                slow_said: false,
                switch: false,
                choice: state.backend,
                software,
            },
        };
        d.apply_effects();
        d.locate_pins();
        d.start_gpu();
        d
    }

    /// Start making the GPU copy of the model in the background if the
    /// 3D view wants it and it was left for later.
    fn start_gpu(&mut self) {
        if !self.gpu.choice.wants_gpu(self.gpu.software) || self.gpu.task.is_some() {
            return;
        }
        let Some(later) = self.gpu.later.take() else {
            return;
        };
        let (model, region) = (self.model.clone(), self.region);
        let task = Task::run("gpu", Some(self.ctx.clone()), move |p| {
            loading::upload(&model, &later.adapter, &region, later.fail_upload, Some(p))
        });
        self.gpu.slow_said = false;
        self.gpu.task = Some(task);
    }

    /// Whether the GPU copy of the model is being made (the 3D view traces
    /// on the CPU meanwhile), and the step it is at.
    pub fn gpu_preparing(&self) -> Option<&'static str> {
        self.gpu.task.as_ref().map(|t| t.progress().stage())
    }

    /// Take the GPU copy of the model once made: the deriver uploads the
    /// visibility for it, then the 3D view switches to it (a drag of a
    /// surface ended first). Said if it takes long, or fails.
    fn poll_gpu(&mut self) {
        if let Some(task) = &self.gpu.task {
            let r = task.try_take().or_else(|| {
                task.ended()
                    .then(|| Err("the GPU renderer could not be prepared".to_string()))
            });
            match r {
                None => {
                    if !self.gpu.slow_said
                        && task.started().elapsed().as_secs_f64() > GPU_SLOW_SECONDS
                    {
                        self.gpu.slow_said = true;
                        self.messages.info(
                            "the GPU renderer is still being prepared (the graphics driver \
                             compiles it): the 3D view traces on the CPU meanwhile",
                        );
                    }
                }
                Some(Ok(pair)) => {
                    let seconds = task.started().elapsed().as_secs_f64();
                    log::debug!("GPU copy of the model made in {seconds:.2} s");
                    self.gpu.task = None;
                    self.gpu_model = Ok(pair.clone());
                    self.deriver.set_gpu(pair);
                    self.send_derive(false);
                    self.gpu.switch = true;
                }
                Some(Err(e)) => {
                    self.gpu.task = None;
                    self.gpu_model = Err(e);
                    if self.gpu.choice.wants_gpu(self.gpu.software) {
                        let (b, warning) = backend_for(
                            &self.model,
                            &self.gpu_model,
                            &self.shown,
                            self.gpu.choice,
                            self.gpu.software,
                        );
                        self.view3d.set_backend(b);
                        self.messages.warn(warning.unwrap_or_default());
                    }
                }
            }
        }
        // The visibility uploaded for the GPU: traced there from now.
        if self.gpu.switch && self.shown.gpu.is_some() && self.drag.is_none() {
            self.gpu.switch = false;
            if self.gpu.choice.wants_gpu(self.gpu.software) {
                let (choice, software) = (self.gpu.choice, self.gpu.software);
                self.set_backend(choice, software);
                if self.gpu.slow_said {
                    self.messages
                        .info("the GPU renderer is ready: the 3D view traces on the GPU");
                }
            }
        }
    }

    /// Show the model of `l` in place of this one, keeping the work on the
    /// document: the view (`state`, whose scene is the one `l` was prepared
    /// for), undo history, saved views and what the tools edit. With
    /// `same`, `l` is this model completed (its octrees built): the cells
    /// are the same, so the selection, the point inspected, the analyses
    /// and the fields read stay as they are. Otherwise (a recompile), the
    /// selection is the same rule on the new model, and what was computed
    /// on the old one is dropped.
    pub fn replace_model(
        &mut self,
        l: Loaded,
        state: &ViewState,
        same: bool,
        software: bool,
        ctx: &egui::Context,
    ) {
        let mut new = Document::open(l, state, software, ctx);
        let scene = std::mem::take(&mut self.scene);
        new.history = std::mem::take(&mut self.history);
        new.editing = std::mem::take(&mut self.editing);
        new.source = self.source.take();
        new.whole = self.whole.take();
        new.input_meshes = self.input_meshes.take();
        new.input = self.input.take();
        new.cut_flow = self.cut_flow.take();
        new.slice_in_3d = self.slice_in_3d;
        new.cut_at_slice = self.cut_at_slice;
        new.show_cuts = self.show_cuts;
        new.problems_on_top = self.problems_on_top;
        if same {
            new.selection = std::mem::replace(&mut self.selection, Selection::new(0));
            new.focus = self.focus.take();
            new.results = std::mem::take(&mut self.results);
            new.changed_boxes = self.changed_boxes.take();
            new.fields = std::mem::take(&mut self.fields);
            new.field = self.field.take();
        } else {
            new.selection = std::mem::replace(&mut self.selection, Selection::new(0));
            new.selection.rebind(&new.model, &new.index);
            // The point inspected, looked up again in the new model (after
            // an edit, the inspector stays on what it showed).
            new.focus = self
                .focus
                .take()
                .map(|f| crate::pick::pick_point(&new.model, f.point));
        }
        // The scene may have changed while `l` was made.
        if new.scene != scene {
            let before = std::mem::replace(&mut new.scene, scene);
            new.follow(&before);
        }
        *self = new;
    }

    /// Start reading the field of `source` on a thread; with `edit`, it
    /// becomes the scene's field once read.
    pub fn load_field(&mut self, source: FieldSource, edit: bool) {
        let (m, input, s) = (self.model.clone(), self.input.clone(), source.clone());
        let tallies = self.fields.tallies.clone();
        let task = Task::run("field", Some(self.ctx.clone()), move |_| {
            read_field(&m, input.as_deref(), &s, &tallies)
        });
        self.fields.failed = None;
        self.messages
            .progress(format!("reading {}", source.describe()));
        // A field being read before is dropped: cancelled.
        self.fields.loading = Some((source, edit, task));
    }

    /// Whether a field is being read.
    pub fn field_loading(&self) -> Option<&FieldSource> {
        self.fields.loading.as_ref().map(|l| &l.0)
    }

    /// Take the field read, if it is; whether one is still being read.
    /// The scene's field is read when it is shown and was not read yet
    /// (from the view file, once the application has said where the input
    /// is; or undone back to): never on this thread, as a tally file can
    /// take a while.
    fn poll_field(&mut self) -> bool {
        if let Some(f) = &self.scene.field {
            if f.shown
                && self.fields.loading.is_none()
                && !self.fields.data.contains_key(&f.source)
                && self.fields.failed.as_ref().map(|f| &f.0) != Some(&f.source)
            {
                self.load_field(f.source.clone(), false);
            }
        }
        let Some((source, edit, task)) = &self.fields.loading else {
            return false;
        };
        // Ended is read first: a value sent before it is then there to take.
        let ended = task.ended();
        let r = match task.try_take() {
            Some(r) => r,
            None if !ended => return true,
            None => Err("reading the field stopped".into()),
        };
        let (source, edit) = (source.clone(), *edit);
        self.fields.loading = None;
        match r {
            Ok((f, bins)) => {
                self.messages.info(format!("read {}", f.field.title()));
                self.fields.data.insert(source.clone(), f);
                self.fields.order.retain(|s| *s != source);
                self.fields.order.push(source.clone());
                if let Some(b) = bins {
                    self.fields.bins.insert(source.clone(), Arc::new(b));
                }
                if edit {
                    let mut state = FieldState::new(source);
                    if let Some(old) = &self.scene.field {
                        state.opacity = old.opacity;
                    }
                    self.edit(&Edit::Field(Some(Box::new(state))));
                } else {
                    self.send_derive(true);
                }
            }
            Err(e) => {
                self.messages.warn(format!("{}: {e}", source.describe()));
                self.fields.failed = Some((source.clone(), e));
                // A tally file that was indexed: the scene names it all
                // the same, so that the panel offers its tallies and bins
                // to choose others with.
                if edit && self.tally_summary(&source).is_some() {
                    let mut state = FieldState::new(source);
                    if let Some(old) = &self.scene.field {
                        state.opacity = old.opacity;
                    }
                    self.edit(&Edit::Field(Some(Box::new(state))));
                }
            }
        }
        false
    }

    /// The field of the scene's `state` as drawn: its data with the
    /// scene's scale and opacity; `None` if hidden or not read yet
    /// ([`Document::poll_field`] reads it, then draws it).
    fn shown_field(&self, state: &FieldState) -> Result<Option<Overlay>, String> {
        if !state.shown {
            return Ok(None);
        }
        let Some(read) = self.fields.data.get(&state.source) else {
            return Ok(None);
        };
        // The data is shared, only the look changes.
        let mut f = read.clone();
        if let Some(s) = state.scale {
            if !(s.lo < s.hi) || (s.log && !(s.lo > 0.0)) {
                return Err("field scale: give low < high (and low > 0 on a log scale)".into());
            }
            f.look.scale = llevant_render::scale::Scale {
                lo: s.lo,
                hi: s.hi,
                log: s.log,
                ..f.look.scale
            };
        } else if !state.style.exact_ends {
            f.look.scale = llevant_render::legend::rounded(&f.look.scale);
        }
        f.look.opacity = state.opacity.clamp(0.0, 1.0);
        f.look.under = if state.geometry_under {
            llevant_render::field::Under::Geometry
        } else {
            llevant_render::field::Under::Neutral
        };
        f.look.scale.ramp = state.style.ramp();
        f.look.scale.bands = state.style.bands.min(64);
        f.look.below = state.style.below;
        f.look.above = state.style.above;
        if let [Some(a), Some(b)] = state.threshold {
            if !(a <= b) {
                return Err("field threshold: give low ≤ high".into());
            }
        }
        f.look.threshold = state.threshold;
        Ok(Some(f))
    }

    /// The fields read, oldest first: each is drawn again at once.
    pub fn fields_read(&self) -> &[FieldSource] {
        &self.fields.order
    }

    /// The field of `source` as read (its look fitted to it), if read.
    pub fn field_data(&self, source: &FieldSource) -> Option<&Overlay> {
        self.fields.data.get(source)
    }

    /// The result and relative error of each cell bin of the MCTAL tally
    /// of `source`, if read (in the order of its `TallySummary::bins`).
    pub fn field_bins(&self, source: &FieldSource) -> Option<&Arc<BinValues>> {
        self.fields.bins.get(source)
    }

    /// The format of the tally file of `source`, once it has been read
    /// (`None` for a CSV file).
    pub fn tally_format(&self, source: &FieldSource) -> Option<Format> {
        self.tally_summary(source).map(|s| s.format)
    }

    /// What the tally file of `source` holds, once it has been indexed
    /// (`None` for a CSV file).
    pub fn tally_summary(&self, source: &FieldSource) -> Option<Arc<FileSummary>> {
        let (path, input) = tally_paths(source, self.input.as_deref());
        self.fields.tallies.summary(&path, input.as_deref())
    }

    /// Why the field of `source` could not be read, if it was the last to
    /// fail.
    pub fn field_error(&self, source: &FieldSource) -> Option<&str> {
        self.fields
            .failed
            .as_ref()
            .filter(|f| &f.0 == source)
            .map(|f| f.1.as_str())
    }

    /// The view as it is now, with the given tracing settings (and no
    /// universes opened: the application knows its stack).
    pub fn view_state(&self, backend: BackendChoice, budget_ms: f64) -> ViewState {
        ViewState {
            camera: Some(self.orbit.state),
            scene: self.scene.clone(),
            slice: self.slice.state(),
            backend,
            budget_ms,
            open: Vec::new(),
            views: self.views.clone(),
        }
    }

    /// Run a command. The application's own (tracing settings, saving,
    /// opening universes) are left to it.
    pub fn execute(&mut self, c: Command) {
        let c = match self.freeze_command(c) {
            Ok(c) => c,
            Err(e) => {
                self.messages.warn(e);
                return;
            }
        };
        match c {
            Command::Edit(e) => self.edit(&e),
            Command::Undo => {
                let before = self.scene.clone();
                let mut plane = self.slice.plane_state();
                if let Some(what) = self.history.undo(&mut self.scene, &mut plane) {
                    self.slice.set_plane_state(plane);
                    self.follow(&before);
                    self.messages.info(format!("undid {what}"));
                }
            }
            Command::Redo => {
                let before = self.scene.clone();
                let mut plane = self.slice.plane_state();
                if let Some(what) = self.history.redo(&mut self.scene, &mut plane) {
                    self.slice.set_plane_state(plane);
                    self.follow(&before);
                    self.messages.info(format!("redid {what}"));
                }
            }
            Command::Focus(p) => self.focus = p.map(|p| p.detailed(&self.model, &self.index)),
            Command::PickSurface(p) => self.pick_surface(p),
            Command::Pick(p, mode) => {
                self.surface_pick = None;
                match p.as_ref().and_then(Picked::cell) {
                    Some(c) => self.select(Query::cell(self.model.cell_id(c)), mode),
                    None if mode == SelectMode::Replace => self.set_selection(None),
                    None => {}
                }
                if p.is_none() && mode == SelectMode::Replace {
                    self.focus = None;
                }
            }
            Command::Select(q, mode) => self.select(q, mode),
            Command::ClearSelection => self.set_selection(None),
            Command::InvertSelection => {
                let q = match self.selection.query.clone() {
                    None => Query::All,
                    Some(Query::Not(q)) => *q,
                    Some(q) => q.not(),
                };
                self.set_selection(Some(q));
            }
            Command::SelectParent => {
                if let Some(q) = self.parent_of_selection() {
                    self.set_selection(Some(q));
                }
            }
            Command::SelectionBack => {
                if self.selection.back(&self.model, &self.index) {
                    self.selection_changed();
                }
            }
            Command::SelectionForward => {
                if self.selection.forward(&self.model, &self.index) {
                    self.selection_changed();
                }
            }
            Command::Hover(p) => {
                let cell = p.as_ref().and_then(Picked::cell);
                self.view3d.hover(cell);
                self.slice.hover(cell);
                self.pointer.hover = p;
            }
            Command::FrameSelection => self.frame_selection(),
            Command::FrameBox(b) => self.frame_box(&b),
            Command::StartJob(kind, region, h, scope, within) => {
                let (m, wake) = (self.model.clone(), Some(self.ctx.clone()));
                // The one before of this kind is dropped: cancelled.
                self.results.jobs.retain(|j| j.kind != kind);
                self.results.jobs.push(match kind {
                    JobKind::Fingerprints => Job::fingerprints(m, wake),
                    _ => Job::check(m, region, h, scope, within, wake),
                });
            }
            Command::StartVolumes(spec) => {
                let m = self.model.clone();
                self.results.jobs.retain(|j| j.kind != JobKind::Volumes);
                self.results
                    .jobs
                    .push(Job::volumes(m, spec, Some(self.ctx.clone())));
            }
            Command::CancelJob(k) => {
                if let Some(j) = self.results.jobs.get(k) {
                    j.progress().cancel();
                }
            }
            Command::Message(m) => self.messages.info(m),
            Command::Warn(m) => self.messages.warn(m),
            Command::DismissMessage => self.messages.dismiss(),
            Command::MarkSurfaces(r) => self.marks.set(&self.model, r),
            Command::LookAt(p) => self.orbit.pivot_at(p),
            Command::SelectCut(k) => {
                self.editing.cut_selected = k.filter(|&k| k < self.scene.cuts.len());
                self.editing.slice_selected = false;
            }
            Command::SelectSlice(on) => {
                self.editing.slice_selected = on;
                if on {
                    self.editing.cut_selected = None;
                }
            }
            Command::SelectMesh(k) => {
                self.editing.mesh_selected = k.filter(|&k| k < self.scene.meshes.len())
            }
            Command::DrawMesh(shape) => self.editing.drawing_mesh = shape,
            Command::SaveNamedView(name) => {
                self.views.push(SavedView {
                    name,
                    camera: self.orbit.state,
                    slice: self.slice.state(),
                    scene: self.scene.clone(),
                });
            }
            Command::GoToView(k) => {
                if let Some(v) = self.views.get(k).cloned() {
                    self.orbit.state = v.camera;
                    self.slice.set_state(&v.slice);
                    let scene = Scene {
                        pins: self.scene.pins.clone(),
                        ..v.scene
                    };
                    self.edit(&Edit::Scene(Box::new(scene), v.name));
                }
            }
            Command::RenameView(k, name) => {
                if let Some(v) = self.views.get_mut(k) {
                    v.name = name;
                }
            }
            Command::DeleteView(k) => {
                if k < self.views.len() {
                    self.views.remove(k);
                }
            }
            Command::SlicePlane(p) => {
                let before = self.slice.plane_state();
                if p != before {
                    self.history.plane(&self.scene, before);
                    self.slice.set_plane_state(p);
                }
            }
            Command::SliceThrough(p) => self.slice.go_to(p),
            Command::SliceRadial(phi) => {
                if self.slice.radial.is_none() {
                    let p = crate::panels::view_settings::default_axis_point(&self.region);
                    self.slice.radial = Some(crate::state::Radial::about_z(p, phi));
                    self.slice.fit();
                }
                if let Some(r) = &mut self.slice.radial {
                    r.phi = phi;
                }
            }
            Command::SliceAt(axis, at) => {
                if self.slice.radial.take().is_some() || self.slice.axis != axis {
                    self.slice.axis = axis;
                    self.slice.fit();
                }
                self.slice.at = at;
            }
            Command::FitCamera => {
                let s = self.orbit.state;
                self.orbit = Orbit::fit(&self.region, s.yaw_deg, s.pitch_deg);
                self.orbit.state.ortho = s.ortho;
            }
            Command::Ortho(on) => self.orbit.state.ortho = on,
            Command::ProblemsOnTop(on) => self.problems_on_top = on,
            Command::SetRuler(r) => self.ruler = r,
            Command::RulerMode(m) => self.ruler_mode = m,
            Command::CameraBack => {
                if let Some(s) = self.camera_history.back() {
                    self.orbit.state = s;
                }
            }
            Command::CameraForward => {
                if let Some(s) = self.camera_history.forward() {
                    self.orbit.state = s;
                }
            }
            Command::Backend(_)
            | Command::Budget(_)
            | Command::SaveView
            | Command::OpenUniverse(_)
            | Command::Back
            | Command::Inspect(_)
            | Command::Workspace(_)
            | Command::ShowPanel(_)
            | Command::Maximize(_)
            | Command::FoldLeft
            | Command::FoldRight
            | Command::ShowKeys
            | Command::CopyCli
            | Command::OpenFile(_)
            | Command::OpenDialog
            | Command::Reload
            | Command::EditCard(..)
            | Command::PreviewCut { .. }
            | Command::CutApply(_)
            | Command::CutSimplify
            | Command::CutAccept(_)
            | Command::RevertCut
            | Command::CancelCutFlow
            | Command::EditCells(..)
            | Command::Renumber(..)
            | Command::EditSelection
            | Command::AddCard { .. }
            | Command::DragSurface(..)
            | Command::EndDrag { .. }
            | Command::UndoInputEdit
            | Command::SaveInput
            | Command::SaveInputAs(_)
            | Command::DiscardInputEdits
            | Command::CancelLoading
            | Command::OpenFieldDialog
            | Command::OpenRunInputDialog
            | Command::UseTool(_)
            | Command::CopyMeshes(_)
            | Command::SaveMeshes(_)
            | Command::Palette => {}
            Command::LoadField(source) => self.load_field(source, true),
            Command::Export3d => {
                let r = crate::export::save_3d(self);
                self.messages.result(r)
            }
            Command::ExportSlice => {
                let r = crate::export::save_slice(self);
                self.messages.result(r)
            }
        }
    }

    fn select(&mut self, q: Query, mode: SelectMode) {
        match self.selection.apply(q, mode, &self.model, &self.index) {
            Ok(()) => self.selection_changed(),
            Err(e) => self.messages.warn(e),
        }
    }

    fn set_selection(&mut self, q: Option<Query>) {
        match self.selection.set(q, &self.model, &self.index) {
            Ok(()) => self.selection_changed(),
            Err(e) => self.messages.warn(e),
        }
    }

    /// Pick the surface at `p` (none: let go of the one picked). The same
    /// place picked again moves to the next surface through it.
    fn pick_surface(&mut self, p: Option<Picked>) {
        let Some(p) = p else {
            self.surface_pick = None;
            return;
        };
        let Some(shape) = p.surface.as_ref().map(|s| s.shape) else {
            self.messages
                .info("no surface there (in the slice, within a few points)");
            return;
        };
        let p = p.detailed(&self.model, &self.index);
        let again = self.surface_pick.as_ref().filter(|s| {
            s.picked.through.len() > 1
                && s.picked.through == p.through
                && s.picked.surface.as_ref().map(|x| x.shape) == Some(shape)
        });
        let at = match again {
            Some(s) => (s.at + 1) % p.through.len(),
            None => p.through.iter().position(|&x| x == shape).unwrap_or(0),
        };
        if p.through.len() > 1 {
            let names: Vec<String> = p
                .through
                .iter()
                .filter_map(|&x| self.model.surface_id(x))
                .map(crate::pick::surface_name)
                .collect();
            self.messages.info(format!(
                "surface {} of {} here ({}): Alt+click again for the next",
                at + 1,
                p.through.len(),
                names.join(", ")
            ));
        }
        self.surface_pick = Some(SurfacePick { picked: p, at });
    }

    /// The MCNP numbers of `cells` (indices into the whole model,
    /// [`Document::whole`]) that this universe opened on its own has not
    /// got; none for the whole model.
    pub fn outside(&self, cells: impl IntoIterator<Item = u32>) -> Vec<i64> {
        let Some((w, _)) = &self.whole else {
            return Vec::new();
        };
        cells
            .into_iter()
            .map(|c| w.cell_id(c))
            .filter(|&id| self.model.cell_index(id).is_none())
            .collect()
    }

    /// Show the selection in both views (no new trace).
    fn selection_changed(&mut self) {
        self.view3d.set_selection(&self.selection.cells);
        self.slice.set_selection(self.selection.cells.clone());
    }

    /// The cell one level above the deepest selected one on the focused
    /// point's path (or above its deepest cell when none is selected).
    fn parent_of_selection(&self) -> Option<Query> {
        let path = &self.focus.as_ref()?.label.path;
        let sel = &self.selection.cells;
        let level = path
            .iter()
            .rposition(|s| sel.contains(s.cell))
            .unwrap_or(path.len());
        let up = path.get(level.checked_sub(1)?)?;
        Some(Query::cell(self.model.cell_id(up.cell)))
    }

    /// The selected cells, as a set.
    pub fn selected(&self) -> &CellSet {
        &self.selection.cells
    }

    /// Take what work done elsewhere made (scenes derived, a field read,
    /// the results of jobs that finished, volumes so far of those still
    /// running); whether a job is still running (its bar moves). A
    /// cancelled volume job leaves its volumes so far. All of them wake
    /// the window when they send.
    pub fn poll_jobs(&mut self) -> bool {
        for d in self.deriver.poll() {
            self.commit(d);
        }
        self.poll_gpu();
        if let Some((key, p)) = self.pointer.picker.latest() {
            // Still there: shown.
            if self.pointer.hover_key.as_ref() == Some(&key) {
                self.execute(Command::Hover(p));
            }
        }
        self.poll_field();
        let plane = self.slice.plane();
        self.marks
            .update(&self.model, self.source.as_ref(), plane, &self.ctx);
        let mut done = Vec::new();
        for (k, j) in self.results.jobs.iter_mut().enumerate() {
            for o in j.poll() {
                match o {
                    Outcome::Partial(v) => {
                        j.sent_partial = true;
                        self.results.volumes = Some(Arc::new(v));
                    }
                    o => done.push((k, o)),
                }
            }
        }
        for (k, o) in done.into_iter().rev() {
            let j = self.results.jobs.remove(k);
            match o {
                Outcome::Volumes(v) => {
                    let short = v.short();
                    self.messages.info(format!(
                        "{} in {:.1} s{}",
                        j.title,
                        v.seconds,
                        if short > 0 {
                            format!(", {short} cells short of it")
                        } else {
                            String::new()
                        }
                    ));
                    self.results.volumes = Some(Arc::new(v));
                }
                Outcome::Partial(_) => unreachable!("kept above"),
                Outcome::Check(v) => {
                    let n = v.volumes.problems().len();
                    self.messages.info(format!(
                        "{}: {n} problem{} in {:.1} s",
                        j.title,
                        if n == 1 { "" } else { "s" },
                        v.seconds
                    ));
                    self.results.check = Some(Arc::new(v));
                    self.results.checks += 1;
                }
                Outcome::Fingerprints(a) => {
                    self.messages.info(format!(
                        "{} groups of universes alike ({:.1} s)",
                        a.groups.len(),
                        a.seconds
                    ));
                    self.results.alike = Some(Arc::new(a));
                }
                Outcome::Failed(e) => {
                    self.messages.warn(if j.sent_partial {
                        format!("{}: {e}; the volumes so far are kept", j.title)
                    } else {
                        format!("{}: {e}", j.title)
                    });
                }
            }
        }
        !self.results.jobs.is_empty()
    }

    /// Fit the 3D view to `b`, keeping the direction of view, and centre
    /// the slice on it.
    fn frame_box(&mut self, b: &Aabb) {
        let s = self.orbit.state;
        let radius = self.orbit.radius;
        self.orbit = Orbit::fit(b, s.yaw_deg, s.pitch_deg);
        self.orbit.state.ortho = s.ortho;
        self.orbit.radius = radius.max(self.orbit.radius);
        self.slice.go_to(b.center());
    }

    /// Fit the 3D view to the selection, keeping the direction of view, and
    /// centre the slice on it.
    fn frame_selection(&mut self) {
        let b = selection_box(
            &self.model,
            &self.index,
            &self.selection.cells,
            &self.region,
        );
        let Some(b) = b.or_else(|| {
            self.focus
                .as_ref()
                .map(|f| point_box(f.point, &self.region))
        }) else {
            return;
        };
        let s = self.orbit.state;
        let radius = self.orbit.radius;
        self.orbit = Orbit::fit(&b, s.yaw_deg, s.pitch_deg);
        self.orbit.state.ortho = s.ortho;
        // Rays still reach across the whole model.
        self.orbit.radius = radius;
        self.slice.go_to(b.center());
    }

    /// Trace the 3D view with `choice`.
    pub fn set_backend(&mut self, choice: BackendChoice, software: bool) {
        // The backend tracing a drag holds its patches.
        if self.drag.is_some() {
            self.messages.warn("end the drag of the surface first");
            return;
        }
        self.gpu.choice = choice;
        self.gpu.software = software;
        self.start_gpu();
        // Being made (or its visibility uploaded): on the CPU until then.
        let waiting = self.gpu.task.is_some() || self.gpu.switch;
        let choice = if waiting { BackendChoice::Cpu } else { choice };
        let (b, warning) = backend_for(&self.model, &self.gpu_model, &self.shown, choice, software);
        self.view3d.set_backend(b);
        if let Some(w) = warning {
            self.messages.warn(w);
        }
    }

    /// Apply an edit if the scene it makes can be shown (known numbers,
    /// valid material specs); otherwise warn why in `messages`.
    fn edit(&mut self, e: &Edit) {
        let mut next = self.scene.clone();
        e.apply(&mut next);
        let changes = Changes::between(&self.scene, &next);
        match self.check(&next, changes) {
            Ok(vis) => {
                self.history.edit(&mut self.scene, e);
                self.follow_cuts(e);
                if let Some(v) = vis {
                    self.vis = v;
                }
                self.send_derive(false);
            }
            Err(msg) => self.messages.warn(msg),
        }
    }

    /// Keep the selected cut and mesh in step with their lists after `e`:
    /// a new one is selected, a removed one is not.
    fn follow_cuts(&mut self, e: &Edit) {
        let after_removing = |selected: Option<usize>, k: usize| match selected {
            Some(s) if s == k => None,
            Some(s) if s > k => Some(s - 1),
            s => s,
        };
        let (cuts, meshes) = (self.scene.cuts.len(), self.scene.meshes.len());
        match e {
            Edit::AddCut(_) => {
                self.editing.cut_selected = cuts.checked_sub(1);
                self.editing.slice_selected = false;
            }
            Edit::RemoveCut(k) => {
                self.editing.cut_selected = after_removing(self.editing.cut_selected, *k)
            }
            Edit::AddMesh(_) => {
                self.editing.mesh_selected = meshes.checked_sub(1);
                self.editing.drawing_mesh = None;
            }
            Edit::RemoveMesh(k) => {
                self.editing.mesh_selected = after_removing(self.editing.mesh_selected, *k)
            }
            _ => {}
        }
        self.editing.cut_selected = self.editing.cut_selected.filter(|&k| k < cuts);
        self.editing.mesh_selected = self.editing.mesh_selected.filter(|&k| k < meshes);
    }

    /// The views after the scene changed from `before` (undo, redo).
    fn follow(&mut self, before: &Scene) {
        self.editing.cut_selected = self
            .editing
            .cut_selected
            .filter(|&k| k < self.scene.cuts.len());
        self.editing.mesh_selected = self
            .editing
            .mesh_selected
            .filter(|&k| k < self.scene.meshes.len());
        let changes = Changes::between(before, &self.scene);
        match self.check(&self.scene, changes) {
            Ok(vis) => {
                if let Some(v) = vis {
                    self.vis = v;
                }
                self.send_derive(false);
            }
            Err(msg) => self.messages.warn(msg),
        }
    }

    /// Whether `next` can be shown (known numbers, valid material specs, a
    /// valid field scale), with its visibility when that changed. Cheap:
    /// the work of showing it is the deriver's.
    fn check(&self, next: &Scene, ch: Changes) -> Result<Option<Visibility>, String> {
        if ch.looks {
            next.check_looks(&self.model)
                .map_err(|e| format!("materials: {e}"))?;
        }
        if let Some(f) = &next.field {
            self.shown_field(f).map_err(|e| format!("field: {e}"))?;
        }
        if ch.visibility {
            let v = next
                .visibility(&self.model, &self.index)
                .map_err(|e| e.to_string())?;
            return Ok(Some(v));
        }
        Ok(None)
    }

    /// Give rules the cells' labels, read from the input (once per model).
    /// A scene whose rules need them, which could not be shown until now, is
    /// shown as it says.
    pub fn give_labels(&mut self, labels: &Arc<llevant_scene::labels::Labels>) {
        if !self.index.set_labels(&self.model, labels.clone()) || !self.scene.hiding.uses_labels() {
            return;
        }
        match self.scene.visibility(&self.model, &self.index) {
            Ok(v) => {
                self.vis = v;
                self.send_derive(false);
            }
            Err(e) => self.messages.warn(format!("view state: {e}")),
        }
    }

    /// Have the deriver bring the views to the scene (`field_read`: the
    /// field's data changed though the scene did not).
    fn send_derive(&mut self, field_read: bool) {
        let field = match &self.scene.field {
            Some(f) => self.shown_field(f).unwrap_or(None),
            None => None,
        };
        self.deriver
            .send(self.scene.clone(), self.vis.clone(), field, field_read);
    }

    /// Whether the views are still catching up with the scene.
    pub fn deriving(&self) -> bool {
        self.deriver.busy()
    }

    /// Bring the views in line with a scene derived ([`Deriver`]): only
    /// swaps what was made.
    fn commit(&mut self, d: Derived) {
        if let Some(e) = d.error {
            self.messages.warn(e);
            return;
        }
        let ch = d.changes;
        if let Some(l) = d.looks {
            self.slice.set_looks(l.clone());
            self.looks = l;
        }
        if let Some((looks, by, words)) = d.palette {
            self.view3d.set_palette(looks, by, &words);
            self.slice.by = by;
        }
        if let Some(s) = d.shown {
            // While a surface is dragged the 3D view shows this widened by
            // the drag (the application makes it): it waits until then.
            if let Some(drag) = &mut self.drag {
                drag.reshow = true;
            } else if let Err(e) = self.view3d.show(&s) {
                self.messages.warn(e.to_string());
            }
            self.slice.level = s.vis.level;
            self.shown = s;
        }
        if let Some(s) = d.seen {
            self.seen = s;
        }
        if ch.effects {
            self.apply_effects();
        }
        if ch.pins {
            self.locate_pins();
        }
        if let Some((f, words)) = d.field {
            if let Err(e) = self.view3d.set_field_words(f.clone(), words.as_deref()) {
                self.messages.warn(format!("{e}: shown in the slice only"));
            }
            self.slice.set_field(f.clone());
            self.field = f;
        }
    }

    fn apply_effects(&mut self) {
        let fx = self.scene.effects;
        self.view3d.set_effect("outline", fx.outline);
        let outline = crate::view3d::passes::Outline {
            colour: fx
                .outline_colour
                .unwrap_or([0; 3])
                .map(|c| c as f32 / 255.0),
            ..Default::default()
        };
        self.view3d
            .passes()
            .set_params("outline", crate::view3d::passes::PostPass::params(&outline));
        self.view3d
            .set_effect("ambient_occlusion", fx.ambient_occlusion);
    }

    fn locate_pins(&mut self) {
        self.pin_info = self
            .scene
            .pins
            .iter()
            .map(|p| pick_point(&self.model, p.point))
            .collect();
    }
}

/// Tally files indexed, by path and the input they were read with, with
/// their summaries; and the MCNP inputs parsed for them (shared with the
/// threads reading fields). An index is a few hundred kB however large the
/// file; it is made again when the file changes. An input is parsed once
/// for all the tally files of its runs.
#[derive(Clone, Default)]
pub struct TallyFiles(Arc<std::sync::Mutex<TallyCache>>);

type TallyKey = (std::path::PathBuf, Option<std::path::PathBuf>);

#[derive(Default)]
struct TallyCache {
    files: HashMap<TallyKey, (Arc<TallyFile>, Arc<FileSummary>)>,
    inputs: HashMap<std::path::PathBuf, Arc<Input>>,
}

impl TallyFiles {
    /// The field of `choice` from the tally file at `path` for model `m`,
    /// indexed with what `input` says of its tallies (indexing it first if
    /// needed).
    fn field(
        &self,
        m: &Model,
        path: &std::path::Path,
        input: Option<&std::path::Path>,
        choice: &TallyChoice,
    ) -> llevant::Result<(Field, Option<BinValues>)> {
        let key = (path.to_path_buf(), input.map(|i| i.to_path_buf()));
        let known = self.lock().files.get(&key).map(|f| f.0.clone());
        let file = match known.filter(|f| f.is_current()) {
            Some(f) => f,
            None => {
                // Not holding the lock while parsing and indexing: other
                // files can be read meanwhile.
                let parsed = match input {
                    Some(i) => Some(match self.input(i) {
                        Some(p) => p,
                        None => {
                            let p = Arc::new(field::read_input(i)?);
                            self.lock().inputs.insert(i.to_path_buf(), p.clone());
                            p
                        }
                    }),
                    None => None,
                };
                let f = Arc::new(TallyFile::open(path, parsed.as_deref())?);
                let summary = Arc::new(f.summary());
                self.lock().files.insert(key, (f.clone(), summary));
                f
            }
        };
        field::tally_field_and_bins(m, &file, choice)
    }

    /// The input at `path`, if parsed (the lock is let go before parsing
    /// one: a guard in a `match` lives through its arms).
    fn input(&self, path: &std::path::Path) -> Option<Arc<Input>> {
        self.lock().inputs.get(path).cloned()
    }

    /// What the tally file at `path` read with `input` holds, if indexed.
    fn summary(
        &self,
        path: &std::path::Path,
        input: Option<&std::path::Path>,
    ) -> Option<Arc<FileSummary>> {
        let key = (path.to_path_buf(), input.map(|i| i.to_path_buf()));
        self.lock().files.get(&key).map(|f| f.1.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TallyCache> {
        self.0.lock().expect("not poisoned")
    }
}

/// The tally file of `source` and the input it is read with: its own run
/// input, or else the model's `input` (relative paths as [`resolve`]).
fn tally_paths(
    source: &FieldSource,
    input: Option<&std::path::Path>,
) -> (std::path::PathBuf, Option<std::path::PathBuf>) {
    let run = match &source.input {
        Some(i) => Some(resolve(i, input)),
        None => input.map(|i| i.to_path_buf()),
    };
    (resolve(&source.path, input), run)
}

/// `path`, or next to the input when it is relative and not found.
fn resolve(path: &std::path::Path, input: Option<&std::path::Path>) -> std::path::PathBuf {
    if path.is_relative() && !path.exists() {
        if let Some(dir) = input.and_then(|i| i.parent()) {
            return dir.join(path);
        }
    }
    path.to_path_buf()
}

/// Read the field of `source` for model `m`: a tally file, meshtal or
/// MCTAL (what the files leave out from the model's `input`; indexed once
/// in `tallies`), or a CSV file. A relative path that is not found is
/// looked for next to the input. With it, for an MCTAL tally, the value
/// and error of each of its cell bins.
pub fn read_field(
    m: &Model,
    input: Option<&std::path::Path>,
    source: &FieldSource,
    tallies: &TallyFiles,
) -> Result<(Overlay, Option<BinValues>), String> {
    let (path, run) = tally_paths(source, input);
    let read = || {
        let choice = source.choice()?;
        if field::tally_format(&path)?.is_some() {
            tallies.field(m, &path, run.as_deref(), &choice)
        } else {
            field::read_csv(m, &path).map(|f| (f, None))
        }
    };
    let (f, bins) = read().map_err(|e: llevant::Error| e.to_string())?;
    // Fitting the scale sorts the values: here, not on the UI thread.
    Ok((Overlay::new(Arc::new(f)), bins))
}

/// A small box around `p` (a twentieth of `region`), to frame a point.
fn point_box(p: Vec3, region: &Aabb) -> Aabb {
    let r = 0.05
        * (0..3)
            .map(|k| region.hi[k] - region.lo[k])
            .fold(0.0, f64::max)
            .max(1e-3);
    Aabb {
        lo: p.map(|x| x - r),
        hi: p.map(|x| x + r),
    }
}

/// Who traces the 3D view, and a warning if it is not who was asked for.
/// `software`: the window's adapter is a CPU one (lavapipe), so `Auto`
/// traces on the CPU.
fn backend_for(
    model: &Arc<Model>,
    gpu_model: &Result<(Arc<Gpu>, Arc<GpuModel>), String>,
    shown: &Shown,
    choice: BackendChoice,
    software: bool,
) -> (Box<dyn TraceBackend>, Option<String>) {
    let want_gpu = choice.wants_gpu(software);
    let cpu = || CpuBackend::with(model.clone(), shown);
    match (gpu_model, want_gpu) {
        (Ok((gpu, gm)), true) => {
            let mut g = GpuBackend::new(gpu.clone(), gm.clone(), model.clone());
            if let Err(e) = g.set_visibility(shown) {
                return (Box::new(cpu()), Some(e.to_string()));
            }
            (Box::new(g), None)
        }
        (Err(e), true) => (
            Box::new(cpu()),
            Some(format!("GPU renderer unavailable, tracing on the CPU: {e}")),
        ),
        (_, false) => (Box::new(cpu()), None),
    }
}
