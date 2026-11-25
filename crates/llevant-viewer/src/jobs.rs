//! Long work on a thread of its own: volumes (to a target, or of the
//! selected cells alone), the full check for gaps and overlaps,
//! fingerprints. Each job is a [`Task`] with a [`Progress`] (a bar in the
//! status bar, a Cancel button) and sends its result back to the document
//! that started it, which keeps the last result of each kind; a new job of
//! a kind cancels the one before. Volumes to a target also send what they
//! have before each further round, kept if the job is cancelled.

use std::sync::Arc;
use std::time::Instant;

use crate::tasks::Task;

use llevant::{
    Aabb, BinTarget, GroupBy, Model, Progress, Status, VolumeGrid, VolumeTarget, Volumes,
};

/// What a job does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    /// Volumes of everything in a region.
    Volumes,
    /// Gaps and overlaps in a region (volumes, keeping the problems).
    Check,
    /// Universes alike up to numbering.
    Fingerprints,
}

impl JobKind {
    pub fn name(self) -> &'static str {
        match self {
            JobKind::Volumes => "volumes",
            JobKind::Check => "check",
            JobKind::Fingerprints => "fingerprints",
        }
    }
}

/// What a volume job measures.
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeSpec {
    pub region: Aabb,
    /// Spacing of the first lines, cm.
    pub h: f64,
    /// `sigma / volume` to refine every cell to (down to `h / 2^`
    /// [`HALVINGS`]); `None`: one pass at `h`.
    pub target: Option<f64>,
    /// Only these cells (indices), every instance of each, lines traced
    /// around them alone (`Model::instance_volumes`, over `region`).
    pub cells: Option<Vec<u32>>,
    /// With `cells`: also each instance of each cell on its own, to the
    /// same target, for cells with 2 to [`MAX_LISTED_INSTANCES`] of them
    /// ([`VolumeRun::instances`]).
    pub instances: bool,
    /// What is measured (for the panel).
    pub scope: String,
}

/// Halvings of `h` a volume job refines to at most.
pub const HALVINGS: i32 = 6;

/// How a cell's volume ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellOutcome {
    /// One pass, no target.
    Measured,
    Converged,
    /// Above the target at the finest spacing.
    Unconverged,
    /// No line met it, though it may hold something (smaller than about
    /// the last spacing cubed).
    Unseen,
    /// Its boxes are empty: volume exactly 0.
    Empty,
}

/// One cell's volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellVolume {
    /// MCNP number.
    pub id: i64,
    pub volume: f64,
    pub sigma: f64,
    /// Spacing that gave it, cm.
    pub h: f64,
    pub outcome: CellOutcome,
}

/// Instances of a cell up to which a volume job measures each on its own
/// ([`VolumeSpec::instances`]).
pub const MAX_LISTED_INSTANCES: u64 = 1000;

/// The volume of one instance of a cell: the piece of it at one place of
/// the model (with one filler in several envelopes, what one envelope
/// clips of it).
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceVolume {
    /// MCNP number of its cell.
    pub cell: i64,
    /// Its id ([`llevant::Instances`]).
    pub id: u64,
    /// Where it is, in MCNP's repeated-structure notation, from the cell
    /// holding it up (`100[1 0 0] < 5`).
    pub place: String,
    /// The cell holding it (index): the envelope it is in.
    pub holder: Option<u32>,
    pub volume: f64,
    pub sigma: f64,
    /// Spacing that gave it, cm.
    pub h: f64,
    pub outcome: CellOutcome,
}

/// Volumes by cell, and what they were asked for.
pub struct VolumeRun {
    pub cells: Vec<CellVolume>,
    /// Each instance on its own, by cell, in order of id
    /// ([`VolumeSpec::instances`]; empty otherwise).
    pub instances: Vec<InstanceVolume>,
    /// Gaps, overlaps and other labels not inside a cell, by name (none
    /// for the selection alone).
    pub problems: Vec<(String, f64, f64)>,
    pub spec: VolumeSpec,
    pub seconds: f64,
    /// Volumes so far: rounds were still to go when they were taken
    /// (cells yet to be refined are [`CellOutcome::Unconverged`]).
    pub partial: bool,
}

impl VolumeRun {
    /// Total volume of the cells.
    pub fn total(&self) -> f64 {
        self.cells.iter().map(|c| c.volume).sum()
    }

    /// Cells that did not reach the target.
    pub fn short(&self) -> usize {
        self.cells
            .iter()
            .filter(|c| matches!(c.outcome, CellOutcome::Unconverged | CellOutcome::Unseen))
            .count()
    }
}

/// Volumes over a grid, kept whole for their problems ([`JobKind::Check`]).
pub struct CheckRun {
    pub volumes: Volumes,
    /// What the region is (for the panel).
    pub scope: String,
    /// Where it looked: the boxes it was limited to, else its region.
    pub covered: Vec<Aabb>,
    pub seconds: f64,
}

impl CheckRun {
    /// Whether one box it looked in holds all of `b` (an overlap there is
    /// found if it is: the check may say nothing of a place it did not
    /// look at).
    pub fn covers(&self, b: &Aabb) -> bool {
        covered_by(&self.covered, b)
    }
}

/// Whether one of `boxes` holds all of `b`, up to rounding.
fn covered_by(boxes: &[Aabb], b: &Aabb) -> bool {
    let tol = 1e-9;
    boxes.iter().any(|c| {
        (0..3).all(|i| {
            let slack = tol * (1.0 + c.lo[i].abs().max(c.hi[i].abs()));
            c.lo[i] - slack <= b.lo[i] && b.hi[i] <= c.hi[i] + slack
        })
    })
}

/// Cells points can end in: not filled, or a lattice some of whose
/// elements the lattice cell fills itself.
pub fn holds(m: &Model, c: u32) -> bool {
    let cell = m.cell(c);
    match &cell.fill {
        None => true,
        Some(llevant::Fill::Universe(_)) => false,
        Some(llevant::Fill::Lattice(_)) => m.fill_targets(c).any(|t| t.unit() == cell.unit),
    }
}

/// Cells and problems of the groups by cell `groups`, each refined to
/// `h[i]` and ended as `outcome(i)`.
fn of_groups(
    groups: &[llevant::Group],
    h: &[f64],
    outcome: impl Fn(usize) -> CellOutcome,
) -> (Vec<CellVolume>, Vec<(String, f64, f64)>) {
    let mut cells = Vec::new();
    let mut problems = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        match (g.status, g.number) {
            (Status::Inside, Some(id)) => cells.push(CellVolume {
                id,
                volume: g.volume.estimate,
                sigma: g.volume.sigma,
                h: h[i],
                outcome: outcome(i),
            }),
            (Status::OnSurface, _) => {}
            _ => problems.push((g.name.clone(), g.volume.estimate, g.volume.sigma)),
        }
    }
    (cells, problems)
}

/// How each of `n` results ended, from the lists of those unconverged,
/// unseen (also unconverged) and empty; `rest` for the others.
fn outcomes(
    n: usize,
    unconverged: &[usize],
    unseen: &[usize],
    empty: &[usize],
    rest: CellOutcome,
) -> Vec<CellOutcome> {
    let mut out = vec![rest; n];
    for (list, o) in [
        (unconverged, CellOutcome::Unconverged),
        (unseen, CellOutcome::Unseen),
        (empty, CellOutcome::Empty),
    ] {
        for &i in list {
            out[i] = o;
        }
    }
    out
}

/// The cells and problems of `volumes_to`'s `c`.
fn of_converged(c: &llevant::Converged) -> (Vec<CellVolume>, Vec<(String, f64, f64)>) {
    let o = outcomes(
        c.groups.len(),
        &c.unconverged,
        &c.unseen,
        &c.empty,
        CellOutcome::Converged,
    );
    of_groups(&c.groups, &c.h, |i| o[i])
}

/// The volumes `spec` asks for; to a target over a region, `partial`
/// gets the volumes so far before each further round.
pub fn measure(
    m: &Model,
    spec: &VolumeSpec,
    p: &Progress,
    mut partial: impl FnMut(VolumeRun),
) -> llevant::Result<VolumeRun> {
    let start = Instant::now();
    let h_min = spec.h / 2f64.powi(HALVINGS);
    let mut instances = Vec::new();
    let (cells, problems) = if let Some(cells) = &spec.cells {
        let cells: Vec<u32> = cells.iter().copied().filter(|&c| holds(m, c)).collect();
        // One pass: the first round every probe meets.
        let rel = spec.target.unwrap_or(f64::INFINITY);
        let target = BinTarget::new(spec.region, rel, h_min);
        let inst = m.instances();
        // A set per cell (every instance), then one per instance listed:
        // one call, so they share lines.
        let mut sets: Vec<Vec<u64>> = cells.iter().map(|&c| inst.ids(c).collect()).collect();
        let mut alone: Vec<(u32, u64)> = Vec::new();
        if spec.instances {
            for &c in &cells {
                let ids = inst.ids(c);
                if (2..=MAX_LISTED_INSTANCES).contains(&(ids.end - ids.start)) {
                    for id in ids {
                        sets.push(vec![id]);
                        alone.push((c, id));
                    }
                }
            }
        }
        let v = m.instance_volumes(&sets, &target, Some(p))?;
        // Sets with no instance (a universe nothing fills): empty too.
        let mut empty = v.empty.clone();
        empty.extend((0..sets.len()).filter(|&b| v.instances[b] == 0));
        let rest = if spec.target.is_some() {
            CellOutcome::Converged
        } else {
            CellOutcome::Measured
        };
        let o = outcomes(sets.len(), &v.unconverged, &v.unseen, &empty, rest);
        let k = cells.len();
        instances = alone
            .iter()
            .enumerate()
            .map(|(i, &(c, id))| {
                let (place, holder) = place(m, id);
                InstanceVolume {
                    cell: m.cell_id(c),
                    id,
                    place,
                    holder,
                    volume: v.volumes[k + i].estimate,
                    sigma: v.volumes[k + i].sigma,
                    h: v.h[k + i],
                    outcome: o[k + i],
                }
            })
            .collect();
        let cells = cells
            .iter()
            .enumerate()
            .map(|(b, &c)| CellVolume {
                id: m.cell_id(c),
                volume: v.volumes[b].estimate,
                sigma: v.volumes[b].sigma,
                h: v.h[b],
                outcome: o[b],
            })
            .collect();
        (cells, Vec::new())
    } else {
        let grid = VolumeGrid::new(spec.region, spec.h);
        match spec.target {
            None => {
                let g = m.volumes(&grid, Some(p))?.grouped(m, GroupBy::Cell);
                of_groups(&g, &vec![spec.h; g.len()], |_| CellOutcome::Measured)
            }
            Some(rel) => {
                let target = VolumeTarget {
                    rel_sigma: rel,
                    h_min,
                    abs_sigma: 0.0,
                    max_probes: llevant::DEFAULT_PROBES,
                    by: GroupBy::Cell,
                };
                let inside = |g: &llevant::Group| g.status == Status::Inside;
                let c = m.volumes_to(
                    &grid,
                    &target,
                    inside,
                    Some(p),
                    Some(&mut |c| {
                        let (cells, problems) = of_converged(c);
                        partial(VolumeRun {
                            cells,
                            instances: Vec::new(),
                            problems,
                            spec: spec.clone(),
                            seconds: start.elapsed().as_secs_f64(),
                            partial: true,
                        });
                    }),
                )?;
                of_converged(&c)
            }
        }
    };
    Ok(VolumeRun {
        cells,
        instances,
        problems,
        spec: spec.clone(),
        seconds: start.elapsed().as_secs_f64(),
        partial: false,
    })
}

/// Where instance `id` is: its path from the cell holding it up, and that
/// cell. A lattice cell filled with itself keeps its element.
fn place(m: &Model, id: u64) -> (String, Option<u32>) {
    let Some(path) = m.instance_path(id) else {
        return (String::new(), None);
    };
    let own = path.last().is_some_and(|s| s.lattice.is_some());
    let up = if own {
        &path[..]
    } else {
        &path[..path.len().saturating_sub(1)]
    };
    let holder = path.iter().rev().nth(1).map(|s| s.cell);
    (m.format_path(up), holder)
}

/// Groups of universes alike (same fingerprint, different numbers), the
/// largest first.
pub struct Alike {
    pub groups: Vec<Vec<i64>>,
    pub seconds: f64,
}

pub enum Outcome {
    Volumes(VolumeRun),
    /// Volumes so far; the job goes on.
    Partial(VolumeRun),
    Check(CheckRun),
    Fingerprints(Alike),
    Failed(String),
}

pub struct Job {
    pub kind: JobKind,
    pub title: String,
    /// Whether it has sent volumes so far.
    pub sent_partial: bool,
    task: Task<Outcome>,
}

impl Job {
    /// Volumes of `m` as `spec` asks.
    pub fn volumes(m: Arc<Model>, spec: VolumeSpec, wake: Option<egui::Context>) -> Job {
        let title = match spec.target {
            Some(t) => format!(
                "volumes of {} to σ/V ≤ {}% (from h = {})",
                spec.scope,
                100.0 * t,
                short(spec.h)
            ),
            None => format!("volumes of {} (h = {})", spec.scope, short(spec.h)),
        };
        let task = Task::spawn("volumes", wake, move |p, send| {
            let outcome = match measure(&m, &spec, p, |run| send(Outcome::Partial(run))) {
                Ok(run) => Outcome::Volumes(run),
                Err(e) => Outcome::Failed(e.to_string()),
            };
            send(outcome);
        });
        Job {
            kind: JobKind::Volumes,
            title,
            sent_partial: false,
            task,
        }
    }

    /// Gaps and overlaps of `m` over `region` at spacing `h`; with
    /// `within`, on the lines of that grid inside those boxes alone.
    pub fn check(
        m: Arc<Model>,
        region: Aabb,
        h: f64,
        scope: String,
        within: Option<Arc<Vec<Aabb>>>,
        wake: Option<egui::Context>,
    ) -> Job {
        let title = format!("check of {scope} (h = {})", short(h));
        let task = Task::run("check", wake, move |p| {
            let start = Instant::now();
            let grid = VolumeGrid::new(region, h);
            let covered = match &within {
                Some(b) => b.to_vec(),
                None => vec![region],
            };
            let volumes = match &within {
                Some(b) => m.volumes_within(&grid, b, Some(p)),
                None => m.volumes(&grid, Some(p)),
            };
            match volumes {
                Ok(volumes) => Outcome::Check(CheckRun {
                    volumes,
                    scope,
                    covered,
                    seconds: start.elapsed().as_secs_f64(),
                }),
                Err(e) => Outcome::Failed(e.to_string()),
            }
        });
        Job {
            kind: JobKind::Check,
            title,
            sent_partial: false,
            task,
        }
    }

    /// Universes of `m` alike up to numbering.
    pub fn fingerprints(m: Arc<Model>, wake: Option<egui::Context>) -> Job {
        let task = Task::run("fingerprints", wake, move |_| {
            let start = Instant::now();
            match m.fingerprints(llevant::DEFAULT_PRECISION) {
                Ok(f) => {
                    let mut by_hash: std::collections::HashMap<_, Vec<i64>> = Default::default();
                    for (k, u) in m.units().iter().enumerate() {
                        by_hash.entry(f.units[k]).or_default().push(u.universe);
                    }
                    let mut groups: Vec<Vec<i64>> =
                        by_hash.into_values().filter(|g| g.len() > 1).collect();
                    for g in &mut groups {
                        g.sort_unstable();
                    }
                    groups.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
                    Outcome::Fingerprints(Alike {
                        groups,
                        seconds: start.elapsed().as_secs_f64(),
                    })
                }
                Err(e) => Outcome::Failed(e.to_string()),
            }
        });
        Job {
            kind: JobKind::Fingerprints,
            title: "fingerprints of every universe".into(),
            sent_partial: false,
            task,
        }
    }

    /// What it has sent since last asked, its outcome last (partial
    /// volumes before it).
    pub fn poll(&self) -> Vec<Outcome> {
        self.task.poll()
    }

    pub fn progress(&self) -> &Arc<Progress> {
        self.task.progress()
    }

    pub fn started(&self) -> Instant {
        self.task.started()
    }

    /// Fraction done, or `None` when the job cannot say.
    pub fn fraction(&self) -> Option<f64> {
        match self.kind {
            JobKind::Fingerprints => None,
            _ => Some(self.progress().fraction()),
        }
    }
}

/// `h` with three significant digits.
pub fn short(h: f64) -> String {
    format!("{:.3}", h)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// The grid spacing for about `lines` lines across `region` (along each of
/// the three axes, a third of them).
pub fn spacing_for(region: &Aabb, lines: f64) -> f64 {
    let d = [0, 1, 2].map(|k| (region.hi[k] - region.lo[k]).max(1e-9));
    // Lines along x: (dy · dz) / h², and so on.
    let area = d[1] * d[2] + d[0] * d[2] + d[0] * d[1];
    (area / lines.max(1.0)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_gives_the_lines_asked_for() {
        let r = Aabb {
            lo: [0.0; 3],
            hi: [10.0, 20.0, 30.0],
        };
        let h = spacing_for(&r, 1100.0);
        let lines = (20.0 * 30.0 + 10.0 * 30.0 + 10.0 * 20.0) / (h * h);
        assert!((lines - 1100.0).abs() < 1e-6);
        assert_eq!(short(0.12345), "0.123");
        assert_eq!(short(2.0), "2");
    }

    #[test]
    fn volumes_so_far_come_before_each_round() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/mcnp-oracle/models/periodic.i"
        );
        let m = Model::load(path, &llevant::Options::default(), None).unwrap();
        let region = m.bounds(llevant::Scope::Problem);
        let spec = VolumeSpec {
            region,
            h: spacing_for(&region, 3000.0),
            target: Some(1e-3),
            cells: None,
            instances: false,
            scope: "the problem".into(),
        };
        let mut so_far = Vec::new();
        let run = measure(&m, &spec, &Progress::new(), |r| so_far.push(r)).unwrap();
        assert!(!run.partial);
        assert!(so_far.len() > 1, "{} partial runs", so_far.len());
        assert!(so_far.iter().all(|r| r.partial && r.short() > 0));
        // The first is the first pass; each lists the same cells.
        assert!(so_far[0].cells.iter().all(|c| c.h == spec.h));
        for r in &so_far {
            let ids = |r: &VolumeRun| r.cells.iter().map(|c| c.id).collect::<Vec<_>>();
            assert_eq!(ids(r), ids(&run));
        }
        // One pass sends none.
        let one = VolumeSpec {
            target: None,
            ..spec
        };
        let run = measure(&m, &one, &Progress::new(), |_| panic!("partial")).unwrap();
        assert!(run.cells.iter().all(|c| c.outcome == CellOutcome::Measured));
    }

    /// One filler in two envelopes, same frame: each instance is the piece
    /// its envelope clips, and the pieces add up to the cell.
    #[test]
    fn instances_are_measured_one_by_one() {
        let m = Model::from_text(
            "t\n1 1 -1.0 -1 u=1 imp:n=1\n2 0 1 u=1 imp:n=1\n\
             10 0 -10 fill=1 imp:n=1\n11 0 -11 fill=1 imp:n=1\n99 0 99 imp:n=0\n\n\
             1 so 3\n10 rpp -5 1 -5 5 -5 5\n11 rpp 1 5 -5 5 -5 5\n99 rpp -5 5 -5 5 -5 5\n\n\
             m1 1001.80c 1\n",
            &llevant::Options::default(),
        );
        let c = (0..m.cells().len() as u32)
            .find(|&c| m.cell_id(c) == 1)
            .unwrap();
        let region = m.bounds(llevant::Scope::Problem);
        let spec = VolumeSpec {
            region,
            h: spacing_for(&region, 1e4),
            target: Some(1e-2),
            cells: Some(vec![c]),
            instances: true,
            scope: "c 1".into(),
        };
        let run = measure(&m, &spec, &Progress::new(), |_| panic!("partial")).unwrap();
        let whole = 4.0 / 3.0 * std::f64::consts::PI * 27.0;
        // The cap beyond x = 1 (height 2 of a radius 3).
        let cap = std::f64::consts::PI * 4.0 * 7.0 / 3.0;
        assert_eq!(run.cells.len(), 1);
        assert!((run.cells[0].volume - whole).abs() < 0.05 * whole);
        assert_eq!(run.instances.len(), 2);
        let by = |holder: i64| {
            run.instances
                .iter()
                .find(|i| i.holder.map(|h| m.cell_id(h)) == Some(holder))
                .unwrap()
        };
        let (a, b) = (by(10), by(11));
        assert_eq!((a.place.as_str(), b.place.as_str()), ("10", "11"));
        assert!(
            (a.volume - (whole - cap)).abs() < 0.05 * (whole - cap),
            "{}",
            a.volume
        );
        assert!((b.volume - cap).abs() < 0.05 * cap, "{}", b.volume);
        assert!(run
            .instances
            .iter()
            .all(|i| i.outcome == CellOutcome::Converged));
        // Without asking, none.
        let spec = VolumeSpec {
            instances: false,
            ..spec
        };
        let run = measure(&m, &spec, &Progress::new(), |_| panic!("partial")).unwrap();
        assert!(run.instances.is_empty());
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn a_check_covers_what_one_of_its_boxes_holds() {
        let b = |lo: f64, hi: f64| Aabb {
            lo: [lo; 3],
            hi: [hi; 3],
        };
        let boxes = [b(0.0, 10.0), b(20.0, 30.0)];
        assert!(covered_by(&boxes, &b(1.0, 9.0)));
        assert!(covered_by(&boxes, &b(20.0, 30.0)));
        assert!(
            !covered_by(&boxes, &b(5.0, 25.0)),
            "half in each is in neither"
        );
        assert!(!covered_by(&boxes, &b(40.0, 50.0)));
    }
}
