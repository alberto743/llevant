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
//!    whether the overlap is solved, or remains, or a cell that writes `#A`
//!    now overlaps B; Revert takes the cut back.
//! 4. Simplified ([`Phase::Simplifying`]), on request, in the background
//!    (it stops at its limits): the shorter geometry, proven equal, shown as
//!    a diff.
//! 5. Accepted or discarded (or the whole cut reverted): the flow closes.

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
        std::thread::spawn(move || {
            let r = src.cut_expand(keep, cut, &Limits::within(PLAN_LIMIT));
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            ctx.request_repaint();
        });
        CutFlow {
            keep,
            cut,
            key: key(source),
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

    /// Simplify the geometry of the cell as it is now, in the background.
    pub fn simplify(&mut self, ctx: &egui::Context, source: &Arc<Source>) {
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
    }

    /// Stop the work under way (it ends with what it has).
    pub fn stop(&self) {
        if let Phase::Simplifying { cancel, .. } = &self.phase {
            cancel.store(true, Ordering::Relaxed);
        }
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
