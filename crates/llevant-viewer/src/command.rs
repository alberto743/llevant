//! What tools and panels ask for. They read the document and return
//! [`Command`]s; the application runs them once the frame's interface is laid
//! out (`Document::execute`), so nothing changes under a panel mid-frame.
//!
//! [`Edit`]s change the [`Scene`] and are undone and redone as whole
//! scenes ([`History`]). The other commands change what is looked at
//! (selection, slice, camera) and are not undone; the selection has its
//! own back and forward. The camera and the slice plane are moved directly
//! by the views, as navigation.

use std::time::{Duration, Instant};

use llevant::Vec3;
use llevant_scene::hiding::ProblemScope;
use llevant_scene::query::Query;

use crate::app::frame::{Panel, View, Workspace};
use crate::jobs::JobKind;
use crate::mesh::MeshState;
use crate::pick::Picked;
use crate::scene::{Pin, Scene};
use crate::selection::SelectMode;
use crate::state::{
    AxisName, BackendChoice, ClipState, Colouring, CutState, Effects, FieldState, PlaneState,
    ScaleState,
};

/// Which list of hidden numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hidden {
    Cells,
    Universes,
    Materials,
}

/// A change to the scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Colouring(Colouring),
    /// Looks of one material (`None`: classified from its card again).
    Material(i64, Option<String>),
    HideVoid(bool),
    Hidden(Hidden, Vec<i64>),
    /// Hide the cells of a rule (with what they hold).
    Hide(Query),
    /// Show again what a rule hid: the rule removed, or its cells taken
    /// out of a list of hidden cells.
    Unhide(Query),
    /// Show a rule's cells again, as an eye of a panel does: the rule (or
    /// its numbers) taken out of what is hidden (void cells too, for
    /// [`Query::Void`]), and with `true` added to the isolated rule.
    Reveal(Query, bool),
    /// Show only a rule's cells (with what they hold and what holds them),
    /// or stop isolating.
    Isolate(Option<Query>),
    /// The universe level shown (MCNP's plotter `LEVEL n`), or the
    /// deepest cells.
    Level(Option<usize>),
    /// Draw a rule's cells as ghosts: shells over what is behind them.
    Ghost(Query),
    /// No longer ghosts: the rule removed, or its cells taken out of the
    /// ghosts' lists of cells.
    Unghost(Query),
    /// No ghosts.
    NoGhosts,
    /// See through gaps (and, on the CPU, overlaps) in 3D, as veils, or
    /// draw them solid.
    SeeThrough(bool),
    /// While isolating, whose gaps and problems are drawn.
    IsolatedProblems(ProblemScope),
    /// Nothing hidden but void cells (as before).
    ShowAll,
    /// The whole scene, from a saved view (named).
    Scene(Box<Scene>, String),
    AddCut(CutState),
    /// A cut changed (dragging one is one step of the history).
    SetCut(usize, CutState),
    RemoveCut(usize),
    /// Turn cut `k` on or off (off: kept, cutting nothing, not drawn).
    CutOn(usize, bool),
    /// Keep a rule by name (the Find panel's Saved).
    SaveRule(crate::scene::SavedRule),
    RemoveRule(usize),
    Clip(Option<ClipState>),
    Effects(Effects),
    AddPin(Pin),
    RemovePin(usize),
    PinNote(usize, String),
    ClearPins,
    /// The field drawn over the cells, or none.
    Field(Option<Box<FieldState>>),
    /// The field's colour scale (`None`: fitted to its values); dragging
    /// its ends is one step.
    FieldScale(Option<ScaleState>),
    /// How much the field covers the cells; dragging is one step.
    FieldOpacity(f32),
    /// The geometry's colours under the field rather than the grey.
    FieldGeometryUnder(bool),
    /// Only the field's values within these (each end included; none, no
    /// bound) drawn; dragging is one step.
    FieldThreshold([Option<f64>; 2]),
    /// The field's colour map, bands and legend.
    FieldStyle(Box<crate::state::FieldStyle>),
    AddMesh(Box<MeshState>),
    /// A mesh changed (dragging a handle is one step).
    SetMesh(usize, Box<MeshState>),
    RemoveMesh(usize),
}

impl Edit {
    /// Apply to `s`. Out-of-range pins are ignored.
    pub fn apply(&self, s: &mut Scene) {
        match self {
            Edit::Colouring(c) => s.colour_by = *c,
            Edit::Material(id, Some(spec)) => {
                s.materials.insert(*id, spec.clone());
            }
            Edit::Material(id, None) => {
                s.materials.remove(id);
            }
            Edit::HideVoid(on) => s.hiding.void = *on,
            Edit::Hidden(Hidden::Cells, v) => s.hiding.cells = v.clone(),
            Edit::Hidden(Hidden::Universes, v) => s.hiding.universes = v.clone(),
            Edit::Hidden(Hidden::Materials, v) => s.hiding.materials = v.clone(),
            Edit::Hide(q) => {
                // Cells join one list of hidden cells; other rules stack.
                match q {
                    Query::Cells(ids) => {
                        for id in ids {
                            if !s.hiding.cells.contains(id) {
                                s.hiding.cells.push(*id);
                            }
                        }
                    }
                    q if !s.hiding.rules.contains(q) => s.hiding.rules.push(q.clone()),
                    _ => {}
                }
            }
            Edit::Unhide(q) => {
                if let Query::Cells(ids) = q {
                    s.hiding.cells.retain(|c| !ids.contains(c));
                }
                s.hiding.rules.retain(|r| r != q);
                if s.hiding.isolate.as_ref() == Some(q) {
                    s.hiding.isolate = None;
                }
            }
            Edit::Reveal(q, widen) => {
                let h = &mut s.hiding;
                match q {
                    Query::Cells(ids) => h.cells.retain(|c| !ids.contains(c)),
                    Query::Universes(ids) => h.universes.retain(|u| !ids.contains(u)),
                    Query::Materials(ids) => h.materials.retain(|m| !ids.contains(m)),
                    Query::Void => h.void = false,
                    _ => {}
                }
                h.rules.retain(|r| r != q);
                if *widen {
                    h.isolate = h.isolate.take().map(|i| i.plus(q.clone()));
                }
            }
            Edit::Isolate(q) => s.hiding.isolate = q.clone(),
            Edit::Level(l) => s.hiding.level = *l,
            Edit::Ghost(q) => {
                if !s.hiding.ghost.contains(q) {
                    s.hiding.ghost.push(q.clone());
                }
            }
            Edit::Unghost(q) => {
                s.hiding.ghost.retain(|r| r != q);
                if let Query::Cells(ids) = q {
                    for r in &mut s.hiding.ghost {
                        if let Query::Cells(c) = r {
                            c.retain(|x| !ids.contains(x));
                        }
                    }
                    s.hiding
                        .ghost
                        .retain(|r| !matches!(r, Query::Cells(c) if c.is_empty()));
                }
            }
            Edit::NoGhosts => s.hiding.ghost.clear(),
            Edit::SeeThrough(on) => s.hiding.see_through = *on,
            Edit::IsolatedProblems(p) => s.hiding.problems = *p,
            Edit::ShowAll => s.hiding = s.hiding.shown_all(),
            Edit::Scene(scene, _) => *s = (**scene).clone(),
            Edit::AddCut(c) => s.cuts.push(*c),
            Edit::SetCut(k, c) => {
                if let Some(x) = s.cuts.get_mut(*k) {
                    *x = *c;
                }
            }
            Edit::RemoveCut(k) => {
                if *k < s.cuts.len() {
                    s.cuts.remove(*k);
                    s.cuts_off.retain(|&j| j != *k);
                    for j in &mut s.cuts_off {
                        if *j > *k {
                            *j -= 1;
                        }
                    }
                }
            }
            Edit::CutOn(k, on) => {
                s.cuts_off.retain(|j| j != k);
                if !*on && *k < s.cuts.len() {
                    s.cuts_off.push(*k);
                    s.cuts_off.sort_unstable();
                }
            }
            Edit::SaveRule(r) => {
                s.rules.retain(|x| x.name != r.name);
                s.rules.push(r.clone());
            }
            Edit::RemoveRule(k) => {
                if *k < s.rules.len() {
                    s.rules.remove(*k);
                }
            }
            Edit::Clip(c) => s.clip = *c,
            Edit::Effects(e) => s.effects = *e,
            Edit::AddPin(p) => s.pins.push(p.clone()),
            Edit::RemovePin(k) => {
                if *k < s.pins.len() {
                    s.pins.remove(*k);
                }
            }
            Edit::PinNote(k, note) => {
                if let Some(p) = s.pins.get_mut(*k) {
                    p.note = note.clone();
                }
            }
            Edit::ClearPins => s.pins.clear(),
            Edit::Field(f) => s.field = f.as_deref().cloned(),
            Edit::FieldScale(x) => {
                if let Some(f) = &mut s.field {
                    f.scale = *x;
                }
            }
            Edit::FieldOpacity(a) => {
                if let Some(f) = &mut s.field {
                    f.opacity = a.clamp(0.0, 1.0);
                }
            }
            Edit::FieldThreshold(t) => {
                if let Some(f) = &mut s.field {
                    f.threshold = *t;
                }
            }
            Edit::FieldGeometryUnder(on) => {
                if let Some(f) = &mut s.field {
                    f.geometry_under = *on;
                }
            }
            Edit::FieldStyle(st) => {
                if let Some(f) = &mut s.field {
                    f.style = (**st).clone();
                }
            }
            Edit::AddMesh(m) => s.meshes.push((**m).clone()),
            Edit::SetMesh(k, m) => {
                if let Some(x) = s.meshes.get_mut(*k) {
                    *x = (**m).clone();
                }
            }
            Edit::RemoveMesh(k) => {
                if *k < s.meshes.len() {
                    s.meshes.remove(*k);
                }
            }
        }
    }

    /// Edits of the same kind in quick succession (a slider being dragged)
    /// become one step of the history.
    fn merge_key(&self) -> Option<&'static str> {
        match self {
            Edit::Clip(_) => Some("clip"),
            Edit::SetCut(..) => Some("cut"),
            Edit::PinNote(..) => Some("note"),
            Edit::FieldScale(_) => Some("field scale"),
            Edit::FieldOpacity(_) => Some("field opacity"),
            Edit::FieldThreshold(_) => Some("field threshold"),
            Edit::FieldStyle(_) => Some("field style"),
            Edit::SetMesh(..) => Some("mesh"),
            _ => None,
        }
    }

    /// For the history menu and the status bar.
    pub fn describe(&self) -> String {
        // A rule of thousands of cells cut to its start.
        let q = |q: &Query| crate::context::elide(&q.to_string(), 60).0;
        match self {
            Edit::Colouring(c) => format!("colour by {}", crate::state::colouring_name(*c)),
            Edit::Material(id, _) => format!("look of material {id}"),
            Edit::HideVoid(on) => format!("{} void cells", if *on { "hide" } else { "show" }),
            Edit::Hidden(Hidden::Cells, _) => "hidden cells".into(),
            Edit::Hidden(Hidden::Universes, _) => "hidden universes".into(),
            Edit::Hidden(Hidden::Materials, _) => "hidden materials".into(),
            Edit::Hide(r) => format!("hide {}", q(r)),
            Edit::Unhide(r) => format!("show {}", q(r)),
            Edit::Reveal(r, false) => format!("show {}", q(r)),
            Edit::Reveal(r, true) => format!("show {} too (isolating)", q(r)),
            Edit::Isolate(Some(r)) => format!("isolate {}", q(r)),
            Edit::Isolate(None) => "stop isolating".into(),
            Edit::Level(Some(l)) => format!("show level {l}"),
            Edit::Level(None) => "show every level".into(),
            Edit::Ghost(r) => format!("ghost {}", q(r)),
            Edit::Unghost(r) => format!("no ghost {}", q(r)),
            Edit::NoGhosts => "no ghosts".into(),
            Edit::SeeThrough(true) => "see through gaps".into(),
            Edit::SeeThrough(false) => "solid gaps".into(),
            Edit::IsolatedProblems(ProblemScope::Selection) => {
                "problems of the cells isolated only".into()
            }
            Edit::IsolatedProblems(ProblemScope::All) => "every problem".into(),
            Edit::ShowAll => "show all".into(),
            Edit::Scene(_, name) => format!("view {name}"),
            Edit::AddCut(c) => format!("add {}", c.describe()),
            Edit::SetCut(_, c) => c.describe(),
            Edit::RemoveCut(k) => format!("remove cut {}", k + 1),
            Edit::CutOn(k, on) => format!("cut {} {}", k + 1, if *on { "on" } else { "off" }),
            Edit::SaveRule(r) => format!("save rule {}", r.name),
            Edit::RemoveRule(_) => "forget a saved rule".into(),
            Edit::Clip(None) => "no clip plane".into(),
            Edit::Clip(Some(_)) => "clip plane".into(),
            Edit::Effects(_) => "effects".into(),
            Edit::AddPin(_) => "add pin".into(),
            Edit::RemovePin(k) => format!("remove pin {}", k + 1),
            Edit::PinNote(k, _) => format!("note of pin {}", k + 1),
            Edit::ClearPins => "remove all pins".into(),
            Edit::Field(Some(f)) => format!("field {}", f.source.describe()),
            Edit::Field(None) => "no field".into(),
            Edit::FieldScale(None) => "field scale fitted".into(),
            Edit::FieldScale(Some(_)) => "field scale".into(),
            Edit::FieldOpacity(_) => "field opacity".into(),
            Edit::FieldThreshold([None, None]) => "no field threshold".into(),
            Edit::FieldThreshold(_) => "field threshold".into(),
            Edit::FieldGeometryUnder(true) => "geometry's colours under the field".into(),
            Edit::FieldGeometryUnder(false) => "grey under the field".into(),
            Edit::FieldStyle(_) => "field style".into(),
            Edit::AddMesh(m) => format!("add FMESH{}", m.id),
            Edit::SetMesh(_, m) => format!("FMESH{}", m.id),
            Edit::RemoveMesh(k) => format!("remove mesh {}", k + 1),
        }
    }
}

/// Everything tools and panels can ask for.
#[derive(Clone, Debug)]
pub enum Command {
    Edit(Edit),
    Undo,
    Redo,
    /// What the inspector shows (`None` clears it); the selection is left
    /// as it is.
    Focus(Option<Picked>),
    /// A point clicked with the select tool: its deepest cell joins the
    /// selection as `mode` says; the inspected point is left as it is
    /// (clicking nothing with `Replace` clears both).
    Pick(Option<Picked>, SelectMode),
    /// The surface at a point clicked with Alt (`None`: let go of it),
    /// opened in the inspector and marked in both views; the selection is
    /// left as it is.
    PickSurface(Option<Picked>),
    /// Select the cells of a rule.
    Select(Query, SelectMode),
    /// Select nothing.
    ClearSelection,
    /// Every cell not selected.
    InvertSelection,
    /// The cell one level up the focused point's path from the deepest
    /// selected one (its envelope).
    SelectParent,
    SelectionBack,
    SelectionForward,
    /// The cell under the pointer in either view (outlined, described in
    /// the status bar).
    Hover(Option<Picked>),
    /// Frame the selection in 3D (and centre the slice on it).
    FrameSelection,
    /// Frame a box in 3D (and centre the slice on it).
    FrameBox(llevant::Aabb),
    /// Start work in the background: a kind, its region and grid spacing
    /// (checks), and what the region is.
    /// A job over a region at a spacing, what it covers; a check with boxes
    /// traces only inside them.
    StartJob(
        JobKind,
        llevant::Aabb,
        f64,
        String,
        Option<std::sync::Arc<Vec<llevant::Aabb>>>,
    ),
    /// Measure volumes in the background.
    StartVolumes(crate::jobs::VolumeSpec),
    CancelJob(usize),
    /// Say something in the status bar (news, shown for a few seconds).
    Message(String),
    /// Warn in the status bar (an error: shown until dismissed).
    Warn(String),
    /// Stop showing the status bar's message.
    DismissMessage,
    /// Mark these surfaces on the slice, as a cell sees them (while its
    /// geometry is edited), or nothing.
    MarkSurfaces(Option<crate::marks::MarkRequest>),
    /// Turn the 3D view to look at a point (the eye stays; it becomes the view's centre).
    LookAt(Vec3),
    /// The cut whose handles are shown (rings included) and that the 3D
    /// view's Section menu shows in detail; `None` also deselects the slice
    /// plane.
    SelectCut(Option<usize>),
    /// The slice plane shown in 3D selected (its rings shown) or not.
    SelectSlice(bool),
    /// Put the slice on a plane, as a step of the history (the slice
    /// plane's handles in 3D).
    SlicePlane(PlaneState),
    /// Move the slice plane through a point.
    SliceThrough(Vec3),
    /// The slice on the plane `axis = at`.
    SliceAt(AxisName, f64),
    /// The slice through the radial axis (z by default) at an angle
    /// (degrees).
    SliceRadial(f64),
    /// Frame the visible model in 3D, keeping the direction of view.
    FitCamera,
    /// See the 3D view straight on (orthographic) or through a pinhole.
    Ortho(bool),
    /// Draw the problems found over the 3D view, through what hides them
    /// ([`crate::document::Document::problems_on_top`]).
    ProblemsOnTop(bool),
    /// The ruler laid (`None`: cleared).
    SetRuler(Option<crate::ruler::Ruler>),
    /// How the Ruler tool lays it.
    RulerMode(crate::ruler::RulerMode),
    /// The place the camera rested at before (after).
    CameraBack,
    CameraForward,
    Backend(BackendChoice),
    /// Milliseconds per preview image.
    Budget(f64),
    /// Write the view file.
    SaveView,
    /// Open a universe of the model on its own (cut to what it holds),
    /// keeping the model open to go back to.
    OpenUniverse(i64),
    /// Back to the model the open universe came from.
    Back,
    /// Open a page of the inspector (and unfold it).
    Inspect(crate::panels::inspector::Subject),
    /// Switch to a workspace.
    Workspace(Workspace),
    /// Bring a panel into sight: its workspace, its tab, its side unfolded.
    ShowPanel(Panel),
    /// Maximise a view, or (`None`) show both.
    Maximize(Option<View>),
    /// Fold or unfold the left side of the workspace shown.
    FoldLeft,
    /// Fold or unfold the right side (the inspector).
    FoldRight,
    /// Save the view as it is now, by name.
    SaveNamedView(String),
    /// Go back to a saved view (its scene is an edit).
    GoToView(usize),
    RenameView(usize, String),
    DeleteView(usize),
    /// The keys window.
    ShowKeys,
    /// The 3D view, the slice, as PNG next to the input.
    Export3d,
    ExportSlice,
    /// The 3D view as a `llevant render` command, to the clipboard.
    CopyCli,
    /// The command palette.
    Palette,
    /// Open another model, or pick one in a dialog.
    OpenFile(std::path::PathBuf),
    OpenDialog,
    /// Read the model again, keeping the view.
    Reload,
    /// Write a card of the input (its kind, number and new text) and
    /// compile again (what changed only), the view kept; not saved.
    EditCard(llevant_scene::source::CardKind, i64, String),
    /// Start fixing an overlap by cutting cell `cut` out of cell `keep`
    /// (`#cut` added to its geometry), shown in the inspector (planned in
    /// the background); nothing is written until [`Command::CutApply`].
    PreviewCut {
        keep: i64,
        cut: i64,
    },
    /// Write the cut proposed, `#cut` expanded to what it stands for, as one
    /// edit (compiled as [`Command::EditCard`]), then check whether the
    /// overlap is solved.
    CutApply(llevant_scene::source::CutExpand),
    /// Simplify the geometry of the cell cut, in the background (the result
    /// is proven equal to it); shown as a diff to accept or discard.
    CutSimplify,
    /// Write the simplification shown, as a second edit.
    CutAccept(llevant_scene::source::Simplified),
    /// Take the cut back (one Undo) and close it.
    RevertCut,
    /// Close the cut (and stop the work under way); what is written stays.
    CancelCutFlow,
    /// Cells changed together (their material, density, parameters), as
    /// one edit taken back by one Undo; the cells then selected.
    EditCells(Vec<i64>, Vec<llevant_scene::source::CellChange>),
    /// Numbers of a kind changed, `(old, new)`, with every card that uses
    /// them, as one edit.
    Renumber(llevant_scene::source::IdKind, Vec<(i64, i64)>),
    /// Open the form that edits the cells selected together (in the
    /// inspector).
    EditSelection,
    /// Add a card numbered `id` at the end of its block, as one edit; a
    /// cell cut out of cell `carve` (`#id` added to its geometry) with it.
    AddCard {
        kind: llevant_scene::source::CardKind,
        id: i64,
        text: String,
        carve: Option<i64>,
    },
    /// Move surface `id` to these coefficients, previewed live (a
    /// [`llevant::draft::Draft`]); written into the input by [`Command::EndDrag`].
    DragSurface(i64, Vec<f64>),
    /// End the drag of a surface: with `commit`, its card written with the
    /// coefficients reached and compiled again (as [`Command::EditCard`]),
    /// else the preview taken back.
    EndDrag {
        commit: bool,
    },
    /// Take back the last card edit.
    UndoInputEdit,
    /// Write the edited input to its file.
    SaveInput,
    /// Write the edited input to another file (`None`: picked in a
    /// dialog), which is then the input.
    SaveInputAs(Option<std::path::PathBuf>),
    /// Forget the edits not saved: read the file again.
    DiscardInputEdits,
    CancelLoading,
    /// Read a field (in the background) and draw it over the cells.
    LoadField(crate::state::FieldSource),
    /// Pick a field's file in a dialog.
    OpenFieldDialog,
    /// Pick the MCNP input of the run of the field's tally file in a dialog.
    OpenRunInputDialog,
    /// The mesh whose handles are shown in Tally and that the FMESH panel
    /// shows in detail.
    SelectMesh(Option<usize>),
    /// The next drag in the slice (in Tally) makes a new FMESH: a cylinder
    /// (`Some(true)`) or a box; `None`: it does not.
    DrawMesh(Option<bool>),
    /// Make a tool the active one, by name.
    UseTool(&'static str),
    /// The cards of these meshes (of the scene), to the clipboard, or to a
    /// file picked in a dialog.
    CopyMeshes(Vec<usize>),
    SaveMeshes(Vec<usize>),
}

impl From<Edit> for Command {
    fn from(e: Edit) -> Command {
        Command::Edit(e)
    }
}

/// A step: the scene before it (or after it, once undone) and, for a change
/// of the slice plane, the plane then; what it was.
struct Step {
    scene: Scene,
    plane: Option<PlaneState>,
    what: String,
}

/// Steps to undo, and after each undo, to redo. Changes of the slice plane
/// are steps among the scene's edits.
#[derive(Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    last: Option<(&'static str, Instant)>,
}

/// Edits of the same kind closer than this merge into one step.
const MERGE: Duration = Duration::from_millis(800);
/// Steps kept.
const DEPTH: usize = 200;

impl History {
    /// Apply `e` to `scene`, remembering the scene before (unless `e`
    /// continues the previous edit, or changes nothing).
    pub fn edit(&mut self, scene: &mut Scene, e: &Edit) {
        let before = scene.clone();
        e.apply(scene);
        if *scene == before {
            return;
        }
        let step = Step {
            scene: before,
            plane: None,
            what: e.describe(),
        };
        self.record(step, e.merge_key());
    }

    /// Remember that the slice was on `before` (the scene is as it is), unless
    /// this continues a drag of the plane.
    pub fn plane(&mut self, scene: &Scene, before: PlaneState) {
        let step = Step {
            scene: scene.clone(),
            plane: Some(before),
            what: "slice plane".into(),
        };
        self.record(step, Some("slice"));
    }

    fn record(&mut self, step: Step, key: Option<&'static str>) {
        let now = Instant::now();
        let merges =
            matches!((key, self.last), (Some(k), Some((l, t))) if k == l && now - t < MERGE);
        if !merges {
            self.undo.push(step);
            if self.undo.len() > DEPTH {
                self.undo.remove(0);
            }
        }
        self.last = key.map(|k| (k, now));
        self.redo.clear();
    }

    /// Go back one step; what was undone, if anything.
    pub fn undo(&mut self, scene: &mut Scene, plane: &mut PlaneState) -> Option<String> {
        let step = self.undo.pop()?;
        let what = step.what.clone();
        self.redo.push(swap(step, scene, plane));
        self.last = None;
        Some(what)
    }

    /// Redo the last step undone; what was redone, if anything.
    pub fn redo(&mut self, scene: &mut Scene, plane: &mut PlaneState) -> Option<String> {
        let step = self.redo.pop()?;
        let what = step.what.clone();
        self.undo.push(swap(step, scene, plane));
        self.last = None;
        Some(what)
    }

    /// What undo would undo.
    pub fn next_undo(&self) -> Option<&str> {
        self.undo.last().map(|s| s.what.as_str())
    }

    /// What redo would redo.
    pub fn next_redo(&self) -> Option<&str> {
        self.redo.last().map(|s| s.what.as_str())
    }
}

/// `step` put in place of the scene and plane now, which become the step.
fn swap(step: Step, scene: &mut Scene, plane: &mut PlaneState) -> Step {
    let now_plane = step.plane.map(|p| std::mem::replace(plane, p));
    Step {
        scene: std::mem::replace(scene, step.scene),
        plane: now_plane,
        what: step.what,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(x: f64) -> Pin {
        Pin {
            point: [x, 0.0, 0.0],
            note: String::new(),
        }
    }

    #[test]
    fn cuts_turned_off_follow_removals() {
        let mut s = Scene::default();
        let plane = |x: f64| CutState::Plane {
            point: [x, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        for x in [0.0, 1.0, 2.0] {
            Edit::AddCut(plane(x)).apply(&mut s);
        }
        Edit::CutOn(2, false).apply(&mut s);
        Edit::CutOn(0, false).apply(&mut s);
        Edit::CutOn(0, false).apply(&mut s);
        assert_eq!(s.cuts_off, vec![0, 2]);
        assert_eq!(s.cuts_on().map(|(k, _)| k).collect::<Vec<_>>(), vec![1]);
        // Removing the first: the third (off) is now the second.
        Edit::RemoveCut(0).apply(&mut s);
        assert_eq!(s.cuts_off, vec![1]);
        assert_eq!(s.cuts_on().next(), Some((0, &plane(1.0))));
        Edit::CutOn(1, true).apply(&mut s);
        assert!(s.cuts_off.is_empty());
        // Off past the end: nothing.
        Edit::CutOn(7, false).apply(&mut s);
        assert!(s.cuts_off.is_empty());
    }

    #[test]
    fn levels_and_ghosts() {
        let mut s = Scene::default();
        let mut h = History::default();
        h.edit(&mut s, &Edit::Level(Some(1)));
        h.edit(&mut s, &Edit::Ghost(Query::Cells(vec![3, 4])));
        h.edit(&mut s, &Edit::Ghost("u 7".parse().unwrap()));
        assert_eq!(s.hiding.level, Some(1));
        assert_eq!(s.hiding.ghost.len(), 2);
        // Cells come out of the lists of ghost cells; the rule stays.
        h.edit(&mut s, &Edit::Unghost(Query::Cells(vec![3])));
        assert_eq!(s.hiding.ghost[0], Query::Cells(vec![4]));
        h.edit(&mut s, &Edit::Unghost(Query::Cells(vec![4])));
        assert_eq!(s.hiding.ghost, vec!["u 7".parse::<Query>().unwrap()]);
        // Showing all keeps the level and the ghosts.
        h.edit(&mut s, &Edit::ShowAll);
        assert_eq!((s.hiding.level, s.hiding.ghost.len()), (Some(1), 1));
        h.edit(&mut s, &Edit::SeeThrough(true));
        assert!(s.hiding.see_through);
        assert_eq!(h.next_undo(), Some("see through gaps"));
        h.undo(&mut s, &mut PlaneState::default());
        assert!(!s.hiding.see_through);
        h.edit(&mut s, &Edit::IsolatedProblems(ProblemScope::All));
        assert_eq!(s.hiding.problems, ProblemScope::All);
        assert_eq!(h.next_undo(), Some("every problem"));
        h.undo(&mut s, &mut PlaneState::default());
        assert_eq!(s.hiding.problems, ProblemScope::Selection);
        h.edit(&mut s, &Edit::NoGhosts);
        assert!(s.hiding.ghost.is_empty());
        assert_eq!(h.next_undo(), Some("no ghosts"));
        h.undo(&mut s, &mut PlaneState::default());
        assert_eq!(s.hiding.ghost.len(), 1);
    }

    #[test]
    fn undo_and_redo_whole_scenes() {
        let mut s = Scene::default();
        let mut h = History::default();
        h.edit(&mut s, &Edit::Hidden(Hidden::Cells, vec![1, 2]));
        h.edit(&mut s, &Edit::AddPin(pin(1.0)));
        h.edit(&mut s, &Edit::Colouring(Colouring::Material));
        assert_eq!(h.next_undo(), Some("colour by material"));
        assert_eq!(
            h.undo(&mut s, &mut PlaneState::default()).as_deref(),
            Some("colour by material")
        );
        assert_eq!(s.colour_by, Colouring::Cell);
        h.undo(&mut s, &mut PlaneState::default());
        assert!(s.pins.is_empty() && s.hiding.cells == vec![1, 2]);
        h.redo(&mut s, &mut PlaneState::default());
        assert_eq!(s.pins, vec![pin(1.0)]);
        // A new edit drops what could be redone.
        h.edit(&mut s, &Edit::ClearPins);
        assert_eq!(h.next_redo(), None);
        assert!(h.redo(&mut s, &mut PlaneState::default()).is_none());
        h.undo(&mut s, &mut PlaneState::default());
        h.undo(&mut s, &mut PlaneState::default());
        h.undo(&mut s, &mut PlaneState::default());
        assert_eq!(s, Scene::default());
        assert!(h.undo(&mut s, &mut PlaneState::default()).is_none());
    }

    #[test]
    fn dragging_a_slider_is_one_step_and_no_ops_are_none() {
        let mut s = Scene::default();
        let mut h = History::default();
        h.edit(&mut s, &Edit::HideVoid(true)); // already so: no step
        assert_eq!(h.next_undo(), None);
        for k in 0..50 {
            let clip = ClipState {
                point: [k as f64, 0.0, 0.0],
                normal: [1.0, 0.0, 0.0],
            };
            h.edit(&mut s, &Edit::Clip(Some(clip)));
        }
        assert_eq!(s.clip.unwrap().point[0], 49.0);
        h.undo(&mut s, &mut PlaneState::default());
        assert_eq!(s.clip, None);
        assert_eq!(h.next_undo(), None);
    }

    #[test]
    fn pins_by_index() {
        let mut s = Scene::default();
        for x in [1.0, 2.0, 3.0] {
            Edit::AddPin(pin(x)).apply(&mut s);
        }
        Edit::RemovePin(1).apply(&mut s);
        Edit::RemovePin(9).apply(&mut s);
        Edit::PinNote(1, "here".into()).apply(&mut s);
        assert_eq!(s.pins.len(), 2);
        assert_eq!(s.pins[1].point[0], 3.0);
        assert_eq!(s.pins[1].note, "here");
    }
}
