# Decisions

One file per decision that shapes the code for a long time: what was decided, why,
and the measurements behind it. A decision that is revisited gets a new file that
supersedes the old one; the old one stays, marked superseded.

| # | Decision | Date |
|---|---|---|
| [0001](0001-cpu-exact-gpu-approximate.md) | The CPU engine is exact and the reference; the GPU only draws | 2026-10-08 |
| [0002](0002-residual-programs.md) | Octree leaves keep each candidate's program specialised to the leaf | 2026-10-17 |
| [0003](0003-problem-box.md) | Beyond universe 0's problem box, only the `IMP:N=0` cells are checked | 2026-10-18 |
| [0004](0004-model-cache.md) | Compiled models are cached on disk, keyed by the input, the options and the engine build | 2026-10-17 |
| [0005](0005-sliced-gpu-dispatches.md) | GPU rays are traced in short slices, never in one long dispatch | 2026-10-28 |
| [0006](0006-reprojection-gpu-only.md) | The 3D view reprojects the hits shown every frame and traces on the GPU alone | 2026-10-31 |
| [0007](0007-no-secondary-rays.md) | Realistic images use direct shading only: no shadows, reflections or AO rays | 2026-10-31 |
| [0008](0008-ui-thread-rule.md) | The viewer's UI thread does no model-sized work | 2026-10-31 |
| [0009](0009-mimalloc.md) | The binaries and the Python module allocate with mimalloc | 2026-11-01 |
| [0010](0010-staged-models-and-recompile.md) | Models are usable before their octrees, and an edit recompiles against the last version | 2026-11-01 |
| [0011](0011-live-drag-drafts.md) | A surface dragged is previewed by a draft, patched in place; the edit compiles on release | 2026-11-03 |
| [0012](0012-viewer-redesign.md) | The viewer is organised in four workspaces around an inspector | 2026-11-06 |
| [0013](0013-viewer-tracing-latency.md) | The 3D view is traced for latency: one job per still view, previews bound each ray's work | 2026-11-14 |
| [0014](0014-gpu-exit-bound.md) | A cell's GPU exit is where its program can first turn false | 2026-11-14 |
| [0015](0015-gpu-state-per-model-and-timing.md) | The GPU keeps per-level state for the model's own levels; GPU frames are timed back to back | 2026-11-15 |
| [0016](0016-overlap-cut.md) | An overlap is fixed by cutting one cell out of the other, shortened by Boolean algebra only | 2026-11-19 |
