//! Interactive viewer of MCNP models on llevant: a 3D view traced on the
//! GPU (or the CPU) at a resolution that keeps the window interactive and
//! refines when the camera stops, and exact 2D slices.
//!
//! The binary `llevant-viewer` runs [`ViewerApp`]. Other programs can show
//! the same views: [`view3d::View3d`] and [`slice::SliceView`] are egui
//! widgets, [`ViewState`] is the view as data (the `--state` file), and
//! [`view3d::passes::PostPass`] adds image effects.
//!
//! How the application is put together (README, "Inside the application"):
//!
//! - [`scene::Scene`]: what is shown (colours, hidden cells, clip, effects,
//!   materials, pins), edited only through [`command::Edit`]s, which the
//!   [`command::History`] undoes.
//! - [`document::Document`]: the model, the scene, the selection and the
//!   views; it runs [`command::Command`]s and updates the views for what
//!   changed.
//! - [`tools`]: what the left button does ([`tools::Tool`]); [`navigation`]
//!   moves the camera whatever the tool.
//! - [`mesh`]: FMESH tallies laid out over the model (in Tally), and
//!   their cards.
//! - [`viewport::ViewPort`]: either view as tools and [`overlay`]s see it
//!   (projection, picking).
//! - [`panels`]: the side panels, the tool row, the views' headers and the
//!   status bar, built from the house widgets of [`theme`].

pub mod app;
pub mod camera;
pub mod command;
pub mod context;
pub mod cutpreview;
pub mod cuts;
pub mod derive;
pub mod document;
pub mod export;
pub mod gizmo;
pub mod icons;
pub mod jobs;
pub mod logo;
pub mod marks;
pub mod mesh;
pub mod messages;
pub mod navigation;
pub mod overlay;
pub mod panels;
pub mod pick;
pub mod ruler;
pub mod run;
pub mod scene;
pub mod seen;
pub mod selection;
pub mod settings;
pub mod slice;
pub mod state;
pub mod tasks;
pub mod theme;
pub mod tools;
pub mod view3d;
pub mod viewport;

pub use app::{DevOptions, ViewerApp};
pub use run::run;
pub use state::ViewState;
