// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! `Deck`: an MCNP input as written, to correct (cut a cell out of another
//! one to fix an overlap) and save. Nothing else of the input changes.

use std::path::PathBuf;
use std::sync::Mutex;

use llevant_scene::source::{CutPlan, Edited, Source};
use pyo3::prelude::*;

use crate::convert::{err, invalid};

/// What cutting a cell out of another would do (`Deck.cut`); nothing is
/// written until `Deck.apply`.
#[pyclass(frozen, module = "llevant")]
pub struct Cut {
    /// The cell that changes.
    #[pyo3(get)]
    pub keep: i64,
    /// The cell taken out of it.
    #[pyo3(get)]
    pub cut: i64,
    /// The card of `keep` as it is.
    #[pyo3(get)]
    pub old_card: String,
    /// The card of `keep` after the cut.
    #[pyo3(get)]
    pub new_card: String,
    /// The geometry written.
    #[pyo3(get)]
    pub geometry: String,
    /// The plain `A #B`.
    #[pyo3(get)]
    pub plain: String,
    /// Terms (surfaces and `#n`) of the plain geometry and of the one written.
    #[pyo3(get)]
    pub terms_before: usize,
    #[pyo3(get)]
    pub terms_after: usize,
    /// The geometry is shorter than the plain one.
    #[pyo3(get)]
    pub simplified: bool,
    /// The algebra proves `keep` lies inside `cut`: there is nothing to
    /// write (one inside only by where its surfaces are is not found).
    #[pyo3(get)]
    pub empty: bool,
    /// The laws of Boolean algebra applied, with how many times.
    #[pyo3(get)]
    pub trace: Vec<String>,
    /// Why it was not simplified, or anything else to look at.
    #[pyo3(get)]
    pub warnings: Vec<String>,
    plan: CutPlan,
}

#[pymethods]
impl Cut {
    fn __str__(&self) -> String {
        if self.empty {
            return llevant_scene::source::nothing_left(self.keep, self.cut);
        }
        format!(
            "- {}+ {}({} terms, was {})",
            self.old_card, self.new_card, self.terms_after, self.terms_before
        )
    }

    fn __repr__(&self) -> String {
        format!(
            "<Cut {} out of {}: {} terms, was {}>",
            self.cut, self.keep, self.terms_after, self.terms_before
        )
    }
}

/// An MCNP input as written.
#[pyclass(frozen, module = "llevant")]
pub struct Deck {
    source: Source,
    path: Option<PathBuf>,
    /// The edits made, to undo them one by one.
    done: Mutex<Vec<Edited>>,
}

#[pymethods]
impl Deck {
    /// Read the input file at `path`.
    #[staticmethod]
    fn open(py: Python<'_>, path: PathBuf) -> PyResult<Deck> {
        let source = py.detach(|| Source::load(&path)).map_err(err)?;
        Ok(Deck {
            source,
            path: Some(path),
            done: Mutex::new(Vec::new()),
        })
    }

    /// The input text `text`.
    #[staticmethod]
    fn from_text(text: &str) -> Deck {
        Deck {
            source: Source::from_str(text),
            path: None,
            done: Mutex::new(Vec::new()),
        }
    }

    /// What cutting cell `cut` out of cell `keep` would do, proven equal to
    /// `keep` outside `cut`; nothing is written. `ValueError`, saying why,
    /// for a cut that is refused.
    #[pyo3(signature = (keep, cut, *, simplify = true))]
    fn cut(&self, py: Python<'_>, keep: i64, cut: i64, simplify: bool) -> PyResult<Cut> {
        let plan = py
            .detach(|| self.source.cut_plan(keep, cut, simplify))
            .map_err(invalid)?;
        let r = &plan.result;
        Ok(Cut {
            keep,
            cut,
            old_card: plan.old_card.clone(),
            new_card: plan.new_card.clone(),
            geometry: r.geometry.clone(),
            plain: r.plain.clone(),
            terms_before: r.terms_before,
            terms_after: r.terms_after,
            simplified: r.simplified,
            empty: r.empty,
            trace: r.trace.clone(),
            warnings: r.warnings.clone(),
            plan,
        })
    }

    /// Write a cut into the deck. `ValueError` if it has nothing to write, or
    /// if the input has changed since the cut was planned (plan it again).
    fn apply(&self, cut: &Cut) -> PyResult<()> {
        // The history held while the input changes, so that edits and undos
        // from other threads go in its order.
        let mut done = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let e = self.source.apply_cut(&cut.plan).map_err(invalid)?;
        done.push(e);
        Ok(())
    }

    /// Undo the last cut applied; false if there is none.
    fn undo(&self) -> PyResult<bool> {
        let mut done = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let Some(e) = done.last() else {
            return Ok(false);
        };
        self.source.restore(e).map_err(invalid)?;
        done.pop();
        Ok(true)
    }

    /// The input as it is now.
    fn text(&self) -> String {
        self.source.to_source()
    }

    /// Write the input to `path` (default: the file it was read from).
    /// `ValueError` if the file read was not UTF-8 (writing it would change
    /// its other bytes: convert it first).
    #[pyo3(signature = (path = None))]
    fn save(&self, py: Python<'_>, path: Option<PathBuf>) -> PyResult<()> {
        let path = path
            .or_else(|| self.path.clone())
            .ok_or_else(|| invalid("this deck has no file: give a path"))?;
        if self.source.not_utf8() {
            return Err(invalid(
                "the input is not UTF-8: saving it would change its other bytes \
                 (convert it to UTF-8 first, e.g. with iconv)",
            ));
        }
        let text = self.source.to_source();
        py.detach(|| std::fs::write(&path, text))
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(format!("{}: {e}", path.display())))
    }

    fn __repr__(&self) -> String {
        match &self.path {
            Some(p) => format!("<Deck {}>", p.display()),
            None => "<Deck>".into(),
        }
    }
}
