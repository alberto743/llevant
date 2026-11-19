//! Python bindings of llevant: load a model once, then query it from
//! scripts and notebooks. Batch queries take and return numpy arrays; all
//! the work happens in Rust, in parallel, without the GIL.
//!
//! `python -m llevant ...` runs the command line of `llevant-cli` in the
//! Python process (a `.pyd`/`.so` is loaded by the interpreter, so it works
//! where a stand-alone executable may not be allowed).

/// The system heap on Windows does not scale to large parallel batches
/// (docs/decisions/0009-mimalloc.md).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod batch;
mod convert;
mod deck;
mod field;
mod image;
mod model;
mod results;

use std::path::PathBuf;

use pyo3::prelude::*;

use crate::convert::invalid;
use crate::model::{options, Model};

/// Runs the command line `argv` (the first item is the program name) and
/// returns the exit status (0, 1 when `check` finds problems, 2 for errors).
/// Releases the GIL while it works.
#[pyfunction]
fn main(py: Python<'_>, argv: Vec<String>) -> i32 {
    py.detach(|| llevant_cli::run_cli(argv)).into()
}

/// Read and compile an MCNP input file.
///
/// `tolerance` (cm): points closer to a surface are on it. `world_half_size`
/// (cm): half-size of the cube that bounds level 0. `grid`: octrees of
/// candidate cells, `auto`, `always` or `off`.
#[pyfunction]
#[pyo3(signature = (path, *, tolerance = 1e-8, world_half_size = 1e6, grid = "auto"))]
fn load(
    py: Python<'_>,
    path: PathBuf,
    tolerance: f64,
    world_half_size: f64,
    grid: &str,
) -> PyResult<Model> {
    Model::load(py, path, options(tolerance, world_half_size, grid)?)
}

/// Compile MCNP input text (see `load`).
#[pyfunction]
#[pyo3(signature = (text, *, tolerance = 1e-8, world_half_size = 1e6, grid = "auto"))]
fn from_text(
    py: Python<'_>,
    text: &str,
    tolerance: f64,
    world_half_size: f64,
    grid: &str,
) -> PyResult<Model> {
    let opts = options(tolerance, world_half_size, grid)?;
    let m = py.detach(|| llevant::Model::from_text(text, &opts));
    Ok(Model::new(m, None, opts).with_text(text.into()))
}

/// Use `n` worker threads for every query from now on. Only possible
/// before the first query (the thread pool is made once).
#[pyfunction]
fn set_threads(n: usize) -> PyResult<()> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build_global()
        .map_err(|e| invalid(format!("set_threads: {e}")))
}

/// Number of worker threads queries use.
#[pyfunction]
fn threads() -> usize {
    rayon::current_num_threads()
}

/// `llevant._frames.<f>(obj[, by])`: data frames are built in Python.
pub(crate) fn frames<'py>(
    py: Python<'py>,
    f: &str,
    obj: &Bound<'py, PyAny>,
    by: Option<&str>,
) -> PyResult<Bound<'py, PyAny>> {
    let m = py.import("llevant._frames")?;
    match by {
        Some(by) => m.call_method1(f, (obj, by)),
        None => m.call_method1(f, (obj,)),
    }
}

#[pymodule]
fn _llevant(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(main, m)?)?;
    m.add_function(wrap_pyfunction!(load, m)?)?;
    m.add_function(wrap_pyfunction!(from_text, m)?)?;
    m.add_function(wrap_pyfunction!(set_threads, m)?)?;
    m.add_function(wrap_pyfunction!(threads, m)?)?;
    m.add_class::<Model>()?;
    m.add_class::<deck::Deck>()?;
    m.add_class::<deck::Cut>()?;
    m.add_class::<results::Label>()?;
    m.add_class::<results::Surface>()?;
    m.add_class::<results::Segment>()?;
    m.add_class::<results::Diagnostic>()?;
    m.add_class::<results::Cell>()?;
    m.add_class::<results::Cells>()?;
    m.add_class::<results::Group>()?;
    m.add_class::<results::Volumes>()?;
    m.add_class::<results::Fingerprints>()?;
    m.add_class::<results::Changes>()?;
    m.add_class::<results::Diff>()?;
    m.add_class::<batch::Classification>()?;
    m.add_class::<batch::Tracing>()?;
    m.add_class::<batch::Hits>()?;
    m.add_class::<image::Image>()?;
    m.add_class::<field::Field>()?;
    m.add("LlevantError", m.py().get_type::<convert::LlevantError>())?;
    m.add(
        "STATUS_NAMES",
        pyo3::types::PyTuple::new(m.py(), convert::STATUS_NAMES)?,
    )?;
    m.add("NO_HIT", llevant::NO_HIT)?;
    m.add("DEFAULT_PRECISION", llevant::DEFAULT_PRECISION)?;
    Ok(())
}
