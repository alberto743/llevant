// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! A cut of one cell out of another (an overlap fixed), planned in the
//! background and shown in the inspector as what Apply would change. The
//! Problems panel starts it; it lives in the [`Document`](crate::document::Document)
//! so that both panels see it.

use llevant_scene::source::{CutPlan, Source};
use std::sync::{Arc, Mutex};

/// A plan, or why there is none.
pub type Plan = Result<CutPlan, String>;

/// A cut plan made in the background (it can take long: the Boolean algebra
/// of large cells), by what it was made from: the input (its `Source` and
/// revision), the two cells and `simplify`.
pub struct CutPreview {
    key: (usize, u64),
    pub keep: i64,
    pub cut: i64,
    pub simplify: bool,
    plan: Arc<Mutex<Option<Plan>>>,
}

impl CutPreview {
    pub fn start(
        ctx: &egui::Context,
        source: &Arc<Source>,
        keep: i64,
        cut: i64,
        simplify: bool,
    ) -> Self {
        let plan = Arc::new(Mutex::new(None));
        let (src, slot, ctx) = (source.clone(), plan.clone(), ctx.clone());
        std::thread::spawn(move || {
            let p = src.cut_plan(keep, cut, simplify);
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
            ctx.request_repaint();
        });
        Self {
            key: (Arc::as_ptr(source) as usize, source.revision()),
            keep,
            cut,
            simplify,
            plan,
        }
    }

    /// Whether it was made from `source` as it is (an input since changed
    /// makes it stale).
    pub fn is_of(&self, source: Option<&Arc<Source>>) -> bool {
        source.is_some_and(|s| self.key == (Arc::as_ptr(s) as usize, s.revision()))
    }

    /// Whether it is the cut of `cut` out of `keep` with `simplify`.
    pub fn is(&self, keep: i64, cut: i64, simplify: bool) -> bool {
        (self.keep, self.cut, self.simplify) == (keep, cut, simplify)
    }

    /// The plan, once made (`None`: still being made).
    pub fn with_plan<R>(&self, f: impl FnOnce(Option<&Plan>) -> R) -> R {
        let g = self.plan.lock().unwrap_or_else(|e| e.into_inner());
        f(g.as_ref())
    }
}
