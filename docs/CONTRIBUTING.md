# Contributing to llevant

## How to contribute

llevant is open source, and open to ideas, but its code changes come from its
maintainers. **Open an issue, not a pull request.** A pull request from anyone else is
closed automatically, with a pointer back here, unless a maintainer asked for it.

Why: what llevant is worth rests on three things: exact answers (the fast path equals
brute force, boxes stay conservative, MCNP is the reference), speed on models of a
million surfaces, and code small enough to reason about. Code is cheap to write now,
with or without an agent; what is expensive is deciding whether a change belongs, how it
fits the crates, and checking that it keeps every guarantee in
[`contracts.md`](contracts.md). Each feature someone needs is reasonable on its own; a
hundred of them, each designed separately, are how a tool like this gets slow, wrong
in corners and hard to change. So the design of every change stays with the
maintainers, and a good issue is the most useful thing you can give.

What helps most:

- **A disagreement with MCNP.** A small input where llevant and MCNP disagree on what is
  at a point, along a line, or which cells are filled. Models are often confidential: a
  few cells cut out of the model are usually enough, and `mcnp-oracle submodel` can cut
  them. These become tests in `tools/mcnp-oracle/models`.
- **A bug.** What you ran, what you expected, what happened, `llevant --version`.
- **A slow case.** The command, the model's size (`llevant info model.i`), the time, and
  what you expected.
- **A feature request.** The job you are trying to do, and why the tool does not do it
  today, rather than the change you would make: the same need often has a simpler
  answer.

The [issue forms](https://github.com/Fusion4Energy/llevant/issues/new/choose) ask for
these.

**Pull requests by invitation.** When an issue is accepted and a maintainer would like
its reporter to write the change (a fix in a part they know well, say), the issue gets
the label `pr-welcome`. A pull request whose description names that issue (`Fixes #123`)
stays open and is reviewed as any other: [Changes](#changes) and [What a change must
keep](#what-a-change-must-keep) below apply to it. Small fixes to the documentation
(a typo, a broken link) are welcome as issues too; they are fixed quickly.

**Credit.** A change made from your issue names you in its commit (`Reported-by:`) and
in the release notes.

**Forks are fine.** The licence is the EUPL-1.2: you may fork llevant and add what your
own work needs, as long as what you distribute stays under it (or a compatible licence)
with its source. If your fork does something others need, open an issue pointing at it.

### What belongs in llevant

The goals and non-goals are in [`architecture.md`](architecture.md#goals-and-non-goals).
In short, a change belongs here when it is about MCNP geometry (reading it, querying it,
checking it, showing it) and when many users of MCNP models would want it. Something
only one workflow needs usually belongs on top of llevant, through its
[Python API](guide/python.md), the [CLI's JSON](guide/cli.md#scripting-and-agents) or
view files ([viewer reference](reference/viewer.md)); an issue saying what is missing
from those is welcome. Transport, tallies computed by llevant, writing MCNP inputs and
matching the CAD are out of scope.

## Setup

```sh
git clone https://github.com/Fusion4Energy/llevant
cd llevant && cargo test --release
```

`release` is the quick optimised profile for tests and benchmarks; `--profile dist`
(fat LTO) is what is shipped and what published numbers are measured on. Large models
(the reference model of the benchmarks is one) are kept out of git, in `untracked/`.

## Commands

Tests and lints:

```sh
cargo test --release                  # engine, scene, render, CLI: unit, MCNP golden, property, CLI tests
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all
cargo test --release -p llevant-gpu -p llevant-viewer   # GPU renderer and viewer (skip without
                                      # a GPU adapter, or fail with LLEVANT_GPU_REQUIRED=1 as in
                                      # CI; Mesa's lavapipe is enough, and slow; WARP only
                                      # when named by WGPU_ADAPTER_NAME)
LLEVANT_BIG_MODEL=<model> cargo test --release -- --ignored   # checks on a large model
PROPTEST_RNG_SEED=20261031 cargo test --release   # property tests with CI's seed
                                      # (without it, every run draws new cases)
cd crates/llevant-py && maturin develop --release && pytest tests   # Python (a venv with numpy)
```

Without a GPU or a Rust toolchain, the GPU and viewer tests run in a throwaway container
on lavapipe (podman or docker; the build cache in a named volume):

```sh
podman run --rm -v "$PWD":/work:Z -v llevant-target:/target -e CARGO_TARGET_DIR=/target \
  -e LLEVANT_GPU_REQUIRED=1 -w /work docker.io/library/rust:1.95 bash -c '
  apt-get update -q && apt-get install -yq mesa-vulkan-drivers libvulkan1 libxkbcommon0 \
    libwayland-client0 libx11-6 && cargo test -p llevant-gpu -p llevant-viewer'
```

Checks and benchmarks on any model (`llevant-bench`; `cargo run --release -p llevant-bench`
lists every command):

```sh
cargo run --release -p llevant-bench -- verify <model> 50000          # BVH vs brute force
cargo run --release -p llevant-bench -- compare-trace <model> <lines.f64>   # octrees on vs off
cargo run --release -p llevant-bench -- points <model> 4000000
cargo run --release -p llevant-bench -- trace <model> 200000 100
cargo run --release -p llevant-bench -- view <model> 1000 [out_dir]   # slices and views (+ PPM)
cargo run --release -p llevant-bench -- gpu-check <model> 150         # GPU vs CPU, ray by ray
cargo run --release -p llevant-bench -- work untracked/synthetic --check tools/bench/work-baseline.json
cargo build --profile dist -p llevant-bench && target/dist/llevant-bench report <models...> --json r.json --markdown r.md
                                      # the numbers of docs/benchmarks.md
python3 tools/synthetic/gen_models.py untracked/synthetic   # benchmark models of other shapes
python3 tools/synthetic/gen_fields.py <model> untracked/fields   # mock results for --field
```

Environment variables for A/B runs: `LLEVANT_BENCH_SET=<dir>` saves and replays a
workload, `LLEVANT_GRID=off|auto|always` and `LLEVANT_RESIDUALS=off` change the search
structures, `LLEVANT_CACHE=off` compiles instead of reading the cache,
`LLEVANT_UNIVERSE=<u>` takes one universe on its own. Profiles:
`cargo build --profile bench -p llevant-bench`, then
`RAYON_NUM_THREADS=1 LLEVANT_PROFILE=1 target/bench/llevant-bench trace <model> 20000 100`
prints the functions with the most samples (by self and inclusive count).

The command line and the viewer:

```sh
cargo run --release -p llevant-cli -- <command> <model> ...   # `llevant`; --help on each
cargo run --release -p llevant-viewer -- <model>
LLEVANT_VIEWER_TEST_PNG=out.png cargo test --release -p llevant-viewer --test app
                                      # the whole app driven headlessly, final window as a PNG
target/release/llevant-viewer <model> --storm   # the UI thread's work per frame through edits
```

The Python module from a checkout, into the active virtual environment, and a Windows
wheel built on Linux (once: `rustup target add x86_64-pc-windows-msvc` and
`cargo install cargo-xwin`):

```sh
cd crates/llevant-py && maturin develop --release
cd crates/llevant-py && rm -rf python/llevant/__pycache__ \
  && XWIN_VERSION=17 XWIN_SDK_VERSION=10.0.22621 XWIN_INCLUDE_ATL=true \
     maturin build --release --target x86_64-pc-windows-msvc -i python3.12 -o ../../untracked/dist
                                      # (without removing __pycache__ it ends up in the wheel)
```

The DXC linked in on Windows (wgpu's `static-dxc`) needs ATL (`atls.lib`) and the VS 2022
C++ library (`__std_find_trivial_*` and the like), which cargo-xwin's default VS 2019
download lacks; its newest SDK (10.0.26100) has no x86_64 headers in xwin, hence the pin.
cargo-xwin downloads once into `~/.cache/cargo-xwin/xwin`: after changing these
variables, remove that folder so that it downloads again.

The documentation site (`zensical.toml`; published from `main` by
`.github/workflows/docs.yml`):

```sh
uvx --with-requirements docs/requirements.txt zensical serve   # http://localhost:8000, rebuilt on every save
uvx --with-requirements docs/requirements.txt zensical build --strict   # as CI: broken links fail it
```

Without uv: `pip install -r docs/requirements.txt`, then `zensical serve`. A bare
`uvx zensical` fails ("mkdocstrings is not installed"): the Python API page needs
`mkdocstrings-python` beside Zensical.

The pages under `docs/` serve two readers. The user pages (`index.md`, `install.md`,
`getting-started.md`, `how-to.md`, `help.md`, `guide/`, `reference/`) are for people
who use llevant and may not program: plain words, a task first, every command shown
runnable on `docs/examples/cask.i`. The rest (`architecture.md`, `contracts.md`,
`internals/`, `decisions/`, `benchmarks.md`) is for developers and agents, and must
stay exact. Images in the user pages come from the commands beside them; regenerate
them when the output changes. The Python API page is generated from the docstrings
in `_llevant.pyi`: change the docstring, not the page.

The viewer screenshots (`docs/img/viewer*.png`, in the README and the site) are of
`docs/examples/core.i` and `cask-v2.i`, at the views in `docs/examples/views`. The
settings file picks the workspace (`Explore`, `Results` or `Check`), so that your own
viewer settings are left alone:

```sh
python docs/examples/gen_core.py --power   # core.i and core-power.csv (the results view's field)
echo '{"frame":{"workspace":"Results"}}' > shot-settings.json
LLEVANT_VIEWER_SETTINGS=shot-settings.json target/release/llevant-viewer docs/examples/core.i \
  --state docs/examples/views/results.view.json --screenshot docs/img/viewer-results.png
```

then scale them to 1600 pixels wide.

Against MCNP itself (needs `mcnp6`; a large model takes MCNP tens of minutes to set up):

```sh
cargo run --release -p mcnp-oracle -- check <model> --points 60000
cargo run --release -p mcnp-oracle -- submodel big.mcnp sub.mcnp --keep 30
```

## Changes

- Every change starts from an issue ([How to contribute](#how-to-contribute)); the pull
  request names it.
- Branch from `main`; one pull request per change, small enough to review in one sitting.
  `main` is always releasable: CI must be green before merging. `main` is protected:
  changes reach it through a pull request with the maintainer's approval (the code
  owner, `.github/CODEOWNERS`); only the maintainer may push to it directly.
- Pull requests from people who are not collaborators are closed on opening by
  `.github/workflows/pr-gate.yml`, unless their description names an issue labelled
  `pr-welcome`. A maintainer can reopen one by hand; reopening does not run the gate.
- Commit messages: a summary line in the imperative, under 72 characters, naming what
  changes for a user or a caller (`Trace lines in windows of doubling length`, not
  `Improvements`); a body when the reason is not obvious, with measurements when the
  change is about speed.
- Before pushing: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --release`. Changes to the GPU renderer or the viewer also run
  `cargo test --release -p llevant-gpu -p llevant-viewer`.

## What a change must keep

- **MCNP is the reference for semantics.** When unsure what MCNP does, probe it with a tiny
  input or extend the models in `tools/mcnp-oracle/models` (and regenerate their `.golden`
  tables with `mcnp-oracle check <model> --sample cells --golden <file>`).
- **The fast path gives exactly the brute-force labels** for points; lines agree with brute
  force as `reference::traces_agree` says. Bounding boxes stay conservative. What each
  query guarantees, and the test that holds it: `docs/contracts.md`.
- **Results do not depend on the number of threads.**
- Property tests run with a fixed seed in CI and with random seeds nightly. When the nightly
  run finds a case, commit the line it adds to the `*.proptest-regressions` file with the fix.
- Speed changes come with an A/B on a saved workload (`LLEVANT_BENCH_SET`), old build first.
- The nightly job also compares the engine's work counters on the synthetic models with
  `tools/bench/work-baseline.json`. A change meant to move them (a better octree, say) writes
  the baseline again in the same commit:
  `llevant-bench work untracked/synthetic --write tools/bench/work-baseline.json`, after
  `python tools/synthetic/gen_models.py untracked/synthetic`.

## Releases

The version lives in `Cargo.toml`, and a version on `main` with no tag yet is released
(`.github/workflows/release.yml`): nobody types a tag. On every push to `main` the
workflow's `check` job compares the version with the tags; when it is new, the binaries,
the wheels and the sdist are built and the wheels tested, then, after an approval on the
`release` environment, the crates go to crates.io, the wheels and the sdist to PyPI, and
a GitHub release (notes generated from the commits, the binaries attached) creates the
tag `vX.Y.Z` on that commit. Versions are never published twice, so:

1. These pass locally (nothing is uploaded):
   ```sh
   cargo publish --workspace --dry-run --exclude llevant-py --exclude llevant-bench --exclude mcnp-oracle
   cd crates/llevant-py && maturin build --release && maturin sdist   # then `twine check` them
   ```
2. `tools/release.sh X.Y.Z` on a clean `main`: it sets `version` in `[workspace.package]`
   and the pins of the workspace's own crates in `[workspace.dependencies]` (the Python
   package takes the same version), updates `Cargo.lock` and commits. It refuses a
   version already released.
3. Push. Approve the release's publishing jobs once CI is green on the commit.

Run by hand (Actions, "Release", "Run workflow"), the workflow only builds and tests.

The parser (migjorn and migjorn-syntax) is pinned exactly in `Cargo.toml`: what it
parses is part of the engine build that keys the cache of compiled models, so a new
migjorn is taken by raising both pins, and reaches users with a new llevant release.

Set up once, in the repository settings and on the two registries:

- a GitHub environment `release`; required reviewers on it make the crates.io and PyPI
  jobs wait for an approval after the builds (without them, pushing a new version
  publishes it straight away);
- in that environment, the secret `CARGO_REGISTRY_TOKEN`: a crates.io API token with the
  `publish-new` and `publish-update` scopes (once the crates exist, it can be narrowed to
  `llevant*` or replaced by crates.io trusted publishing);
- on PyPI, a trusted publisher for the project `llevant` (a "pending publisher" before
  the first release): owner `Fusion4Energy`, repository `llevant`, workflow `release.yml`,
  environment `release`. No PyPI token is stored.

If the crates job fails part-way, the crates already uploaded stay published: publish the
remaining ones by hand in dependency order (`cargo publish -p <crate>`; llevant,
llevant-scene, llevant-render, llevant-gpu, llevant-viewer, llevant-cli).
