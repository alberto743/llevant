// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! Cutting one cell out of another (an overlap fixed), step by step, in the
//! inspector. It lives in the [`Document`](crate::document::Document), so the
//! Problems panel (which starts it), the inspector (which shows it) and the
//! application (which writes it) all see it.
//!
//! The steps, as `docs/guide/viewer.md` numbers them, and their phases:
//!
//! 1. Proposed ([`Phase::Planning`]): the cell with `#B` added, planned in
//!    the background; Apply or Cancel.
//! 2. Written ([`Phase::Applying`]): `#B` written as what it stands for.
//! 3. Verified ([`Phase::Verifying`]): the check of what changed says
//!    whether the overlap is solved, or remains, or a cell that depends on A
//!    now overlaps B; Revert takes the cut back.
//! 4. Simplified ([`Phase::Simplifying`]), on request, in the background
//!    (it stops at its limits): the shorter geometry, proven equal, shown as
//!    a diff.
//! 5. Accepted or discarded (or the whole cut reverted): the flow closes.

use llevant::{Aabb, Model, Scope};
use llevant_scene::source::{CutExpand, Limits, Simplified, Source};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A result made in the background, once it is made.
pub type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

/// The longest the simplification may take.
pub const SIMPLIFY_LIMIT: Duration = Duration::from_secs(20);

/// The longest planning the cut (its proof) may take.
pub const PLAN_LIMIT: Duration = Duration::from_secs(30);

pub struct CutFlow {
    /// The cell that changes, and the cell taken out of it.
    pub keep: i64,
    pub cut: i64,
    /// The input it was made from (its `Source` and revision).
    key: (usize, u64),
    /// Stops the planning when the flow is closed or replaced.
    plan_cancel: Arc<AtomicBool>,
    /// Where the two cells meet, in the model's frame, before the cut
    /// ([`where_they_meet`]): what a check must have covered to judge it.
    pub meet: Vec<Aabb>,
    /// The last check before the cut, as far as it bears on `cut`
    /// (`None`: there was none).
    pub before: Option<Before>,
    pub phase: Phase,
}

pub enum Phase {
    /// The cut being planned, then proposed (the plan, once made).
    Planning(Slot<CutExpand>),
    /// Approved: being written and built.
    Applying(CutExpand),
    /// Written. `check_before`: the last check started when it was written
    /// (only one started later looked at the model with the cut).
    Verifying { plan: CutExpand, check_before: u64 },
    /// Being simplified, then shown (the result, once made).
    Simplifying {
        slot: Slot<Simplified>,
        cancel: Arc<AtomicBool>,
    },
}

fn key(source: &Arc<Source>) -> (usize, u64) {
    (Arc::as_ptr(source) as usize, source.revision())
}

impl CutFlow {
    /// Plan the cut of `cut` out of `keep`, in the background.
    pub fn start(ctx: &egui::Context, source: &Arc<Source>, keep: i64, cut: i64) -> Self {
        let slot: Slot<CutExpand> = Arc::new(Mutex::new(None));
        let (src, out, ctx) = (source.clone(), slot.clone(), ctx.clone());
        let plan_cancel = Arc::new(AtomicBool::new(false));
        let limits = Limits {
            cancel: Some(plan_cancel.clone()),
            ..Limits::within(PLAN_LIMIT)
        };
        std::thread::spawn(move || {
            let r = src.cut_expand(keep, cut, &limits);
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            ctx.request_repaint();
        });
        CutFlow {
            keep,
            cut,
            key: key(source),
            plan_cancel,
            meet: Vec::new(),
            before: None,
            phase: Phase::Planning(slot),
        }
    }

    /// Whether it still holds for `source`: the input is as it was made
    /// from (while the cut is being written, it is changing).
    pub fn is_of(&self, source: Option<&Arc<Source>>) -> bool {
        matches!(self.phase, Phase::Applying(_)) || source.is_some_and(|s| self.key == key(s))
    }

    /// Whether it is the cut of `cut` out of `keep`.
    pub fn is(&self, keep: i64, cut: i64) -> bool {
        (self.keep, self.cut) == (keep, cut)
    }

    /// Whether the cut is written (and Revert has an edit to take back).
    pub fn is_written(&self) -> bool {
        matches!(
            self.phase,
            Phase::Verifying { .. } | Phase::Simplifying { .. }
        )
    }

    /// The cut was written (`source` is the input now) and the check of
    /// what changed is next; `check_before` identifies the check before it.
    pub fn written(&mut self, source: Option<&Arc<Source>>, check_before: u64) {
        if let Phase::Applying(plan) = &self.phase {
            if let Some(s) = source {
                self.key = key(s);
            }
            self.phase = Phase::Verifying {
                plan: plan.clone(),
                check_before,
            };
        }
    }

    /// Simplify the geometry of the cell as it is now, in the background:
    /// only once the cut is written and being verified (a flow still
    /// planning would otherwise pass for one written, and Revert take back
    /// another edit). Whether it started.
    pub fn simplify(&mut self, ctx: &egui::Context, source: &Arc<Source>) -> bool {
        if !matches!(self.phase, Phase::Verifying { .. }) {
            return false;
        }
        let slot: Slot<Simplified> = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let (src, out, ctx, keep) = (source.clone(), slot.clone(), ctx.clone(), self.keep);
        let limits = Limits {
            cancel: Some(cancel.clone()),
            ..Limits::within(SIMPLIFY_LIMIT)
        };
        std::thread::spawn(move || {
            let r = src.cell_simplify(keep, &limits);
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            ctx.request_repaint();
        });
        self.key = key(source);
        self.phase = Phase::Simplifying { slot, cancel };
        true
    }

    /// Stop the work under way (it ends with what it has).
    pub fn stop(&self) {
        self.plan_cancel.store(true, Ordering::Relaxed);
        if let Phase::Simplifying { cancel, .. } = &self.phase {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

/// The most pairs of boxes [`where_they_meet`] intersects one by one.
const MEET_PAIRS_MOST: usize = 200_000;

/// Where cells `keep` and `cut` of `m` can meet, in the model's frame: each
/// instance box of one (as [`Model::instance_boxes`] gives it, as the check
/// of what changed covers it) cut by each of the other's that it meets.
/// Past `MEET_PAIRS_MOST` pairs, the instance boxes of `keep` that meet
/// any of `cut`'s.
pub fn where_they_meet(m: &Model, keep: i64, cut: i64) -> Vec<Aabb> {
    let (Some(a), Some(b)) = (m.cell_index(keep), m.cell_index(cut)) else {
        return Vec::new();
    };
    let problem = m.bounds(Scope::Problem);
    let (ka, kb) = (
        m.instance_boxes(&[a], problem, 0.0),
        m.instance_boxes(&[b], problem, 0.0),
    );
    if ka.len().saturating_mul(kb.len()) > MEET_PAIRS_MOST {
        let all_b = kb.iter().fold(Aabb::EMPTY, |u, x| u.union(x));
        return ka
            .into_iter()
            .filter(|x| !x.intersect(&all_b).is_empty())
            .collect();
    }
    ka.iter()
        .flat_map(|x| kb.iter().map(move |y| x.intersect(y)))
        .filter(|x| !x.is_empty())
        .collect()
}

/// What the last check before a cut said of the cut cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Before {
    /// The cells it found overlapping the cut cell.
    pub overlapping: Vec<i64>,
    /// Where it looked ([`crate::jobs::CheckRun::covered`]): it says
    /// nothing of an overlap elsewhere.
    pub covered: Vec<Aabb>,
}

/// What the last check of `doc` said of `cell` (`None`: no check has run).
pub fn before(doc: &crate::document::Document, cell: i64) -> Option<Before> {
    let covered = doc.results.check.as_ref()?.covered.clone();
    Some(Before {
        overlapping: overlapping(doc, cell)?,
        covered,
    })
}

/// The cells the last check of `doc` found overlapping `cell` (`None`:
/// no check has run).
fn overlapping(doc: &crate::document::Document, cell: i64) -> Option<Vec<i64>> {
    let run = doc.results.check.as_ref()?;
    let m = &doc.model;
    let c = m.cell_index(cell)?;
    let v = &run.volumes;
    let mut out: Vec<i64> = v
        .problems()
        .iter()
        .filter(|&&i| v.labels[i].status == llevant::Status::Overlap)
        .map(|&i| v.labels[i].involved.as_slice())
        .filter(|inv| inv.contains(&c))
        .flat_map(|inv| inv.iter().filter(|&&x| x != c).map(|&x| m.cell_id(x)))
        .collect();
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// A flow closed or replaced stops what it still had running: its work is
/// no longer wanted.
impl Drop for CutFlow {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What a background result is, as it is now (`None`: still being made).
pub fn read<T, R>(slot: &Slot<T>, f: impl FnOnce(Option<&Result<T, String>>) -> R) -> R {
    let g = slot.lock().unwrap_or_else(|e| e.into_inner());
    f(g.as_ref())
}

/// The last check started (a larger number is a check started later).
pub fn check_id(doc: &crate::document::Document) -> u64 {
    doc.results.checks_started
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flow closed or replaced while it is planning tells the planning to
    /// stop, so proposals made one after another do not pile up proofs.
    #[test]
    fn closing_a_flow_stops_its_planning() {
        let ctx = egui::Context::default();
        let src = Arc::new(Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 imp:n=1\n\n1 so 5\n2 sx 4 5\n\n",
        ));
        let f = CutFlow::start(&ctx, &src, 2, 1);
        let flag = f.plan_cancel.clone();
        assert!(!flag.load(Ordering::Relaxed));
        drop(f);
        assert!(flag.load(Ordering::Relaxed));
    }

    /// Two cells of a universe placed 100 cm along x meet there, not where
    /// their own boxes (in the universe's frame) are.
    #[test]
    fn cells_meet_where_they_are_placed() {
        let m = Model::from_text(
            "t\n10 0 -10 fill=1 (100 0 0) imp:n=1\n11 0 10 -99 imp:n=1\n12 0 99 imp:n=0\n\
             1 0 -1 u=1 imp:n=1\n2 0 -2 u=1 imp:n=1\n3 0 1 2 u=1 imp:n=1\n\n\
             1 so 5\n2 sx 4 5\n10 s 100 0 0 20\n99 so 500\n\n",
            &llevant::Options::default(),
        );
        let meet = where_they_meet(&m, 1, 2);
        assert!(!meet.is_empty());
        for b in &meet {
            assert!(b.lo[0] > 90.0 && b.hi[0] < 110.0, "{b:?}");
        }
        // The cells' own boxes are near the origin.
        assert!(m.cell(m.cell_index(1).unwrap()).bbox.hi[0] < 10.0);
    }
}
