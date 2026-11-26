# Contracts

What each query of the engine (`crates/llevant`) guarantees, and the tests that hold it
to that. A change that breaks one of these is a bug, unless this page changes with it.
The semantics themselves (what a cell is, the problem box, lattices) come from MCNP and
are checked against it by `mcnp-oracle` (`docs/internals/engine.md`, "Checks"); the deliberate
departures are in `docs/internals/engine.md`, "Deliberate semantics".

The reference for the fast paths is `llevant::reference`: brute force, every cell of
each universe a candidate, with no boxes, BVH or octrees.

| Query | Guarantee | Enforced by |
|---|---|---|
| `Model::classify(points, depth)` | Exactly the labels of `reference::classify`, for any `GridMode`, with or without residual programs, at every `Depth`. | `tests/properties.rs` (`fast_path_equals_brute_force`, `octree_path_equals_brute_force`), `tests/levels.rs` (`points_to_a_level`), `tests/submodel.rs`, `tests/big_model.rs` (ignored; reference model), `llevant-bench verify` |
| | The cell MCNP reports for the golden points of the zoo models. | `tests/mcnp_golden.rs`; `mcnp-oracle check` regenerates them |
| `Model::trace(lines, depth)` | Agrees with `reference::trace` as `reference::traces_agree` says: segment by segment the same labels, ends within the tolerance. Where several surfaces lie within the tolerance of an end, which one is named may differ, and on-surface slivers up to 1e-6 cm may appear in one and not the other. | `tests/properties.rs` (`tracing_equals_brute_force_and_points`, `tracing_with_octrees_equals_brute_force_and_points`), `tests/levels.rs` (`tracing_to_a_level`), `tests/mcnp_golden.rs`, `tests/big_model.rs`, `llevant-bench compare-trace` (octrees on vs off) |
| | Segments cover `[t_min, t_max]` without gaps between them; a point inside a segment classifies to its label, except on-surface where the line runs within the tolerance of a surface. | `tests/tracing.rs` (`segments_cover_the_line_with_exact_ends`, `segment_labels_match_point_labels`), the property tests above |
| `Model::first_hits(rays, vis, skip)` | The first visible segment `trace` gives: the same label and surface, `t` within the tolerance (a ray traced in windows can round a crossing one ULP away). With a `Skip` built for the same visibility, the same hits as without it. With ghosts, the ghost record of each ray is the boundaries `trace` crosses before the hit, the first named as the GPU names it; with veils, which gaps and overlaps shown come before it. | `tests/properties.rs` (`first_hit_is_the_first_visible_segment`, `skipped_first_hits_are_the_first_hits`), `tests/levels.rs` (with ghosts and veils, at levels), `tests/hits.rs`, `tests/big_model.rs` |
| `Model::slice`, `Model::render`, `Model::pixel_hits` | A slice pixel holds the label of the point at its centre; a render pixel is the first hit of the camera's ray through it. Cutaways only remove what they say. | `tests/views.rs` |
| Rules by label (`label`, `$`) | The cells whose `$` comments (not `c` lines, not `FCn` text) contain the text, any case; the same cells from a model read from the cache (labels read from the input); an error, not an empty set, when no labels were given. Rules round-trip through their text. | `llevant-scene` (`labels`, `query`, `hiding` tests), `tests/cli.rs` (`label_rules`), `tests/test_api.py` (`test_label_rules`), the viewer's `labels_select_and_isolate` |
| `Visibility::problem_cells` | First hits show a gap or problem only where it belongs to a masked cell (`Visibility::owns_problem`), and are otherwise what tracing says (random masks among the random visibilities). | `tests/levels.rs` (`first_hits_at_a_level_with_ghosts`), `tests/hits.rs` (`problem_cells_keep_only_their_problems`), `llevant-scene` `hiding` tests |
| Bounding boxes (cells, nodes, problem box) | Conservative: padded outward; a failed refinement keeps the previous box. A box that cut into its cell would change labels, which the fast-path tests above would catch. | `bbox.rs` unit tests, the fast-path tests |
| Every query | Results are identical for any number of threads. | `tests/properties.rs` (`results_do_not_depend_on_thread_count`, `tracing_does_not_depend_on_thread_count`), `tests/volumes.rs`, `tests/fingerprints.rs` (`precision_and_threads`), `llevant-bench trace` (one thread vs all) |
| Bad arguments | An `Error`, never a panic: non-finite lines, non-unit directions, a visibility of the wrong size, degenerate cameras and grids, missing files. | `tests/errors.rs` |
| `Model::placed_boxes` | Every point of a cell, at every level, is inside one of its instances' boxes (TRCL fills, lattices, level 0); each cell once, in the order asked. | `tests/volumes.rs` (`placed_boxes_hold_every_point_of_their_cells`), `llevant-scene` `bounds` tests, `tests/cli.rs` (`bbox_and_isolated_framing`) |
| `Model::cells_kept` (the viewer's `shown` rule) | Conservative: a cell with any point, at any level, that the cutaway does not remove is kept (planes, wedges, kept and removed boxes; TRCL fills, lattices); a cell wholly cut away by its boxes is not. | `tests/volumes.rs` (`cells_kept_by_cuts`), the viewer's `shown_rule_follows_the_view` |
| `Model::volumes_within`, `instance_boxes` | Over the whole region as one box, the same volumes as `volumes`; within a cell's instance boxes, the cell's whole volume and the problems the whole check finds there (rotated fills and lattices too). | `tests/volumes.rs` (`a_check_within_boxes_finds_what_the_whole_check_finds_there`, `instance_boxes_cover_every_instance`) |
| `Model::volumes`, `volumes_to`, `instance_volumes` | Deterministic for a given grid. The standard error is honest (the exact volume is within a few sigma at the rate a normal distribution says). Gaps and overlaps planted in a model are found. `Progress` is reported and cancelling stops the call with `Error::Cancelled`. | `tests/volumes.rs` (`sigma_is_an_honest_standard_error`, `planted_gaps_and_overlaps_are_found`, `volumes_do_not_depend_on_thread_count`), `tests/errors.rs` |
| `Model::fingerprints` | The same for inputs that compile to the same geometry (renumbering, card order, equivalent rewritings); different for any change, and only for the cells that contain it. | `tests/fingerprints.rs` |
| `Model::instances` | Every place a cell occupies has one id; ids and paths map to each other; the instance of a point's label is its path's. | `tests/instances.rs` |
| `Model::universe` | A universe opened on its own answers as brute force does on it, and as the whole model does inside it. | `tests/submodel.rs` |
| `Model::save` / `Model::read` | A model read back answers exactly as the compiled one. Bytes with another magic, format version or engine build, or cut short, are refused with `Error::Format`. | `tests/saved.rs` |
| `Model::compile_staged` / `Model::complete` | A staged model (octrees pending) answers as the complete one: the same labels for points, traces that agree. Completing it gives a model whose saved bytes are exactly those of `Model::compile`, in every `GridMode`. A staged model cannot be saved. | `tests/staged.rs` |
| `Model::recompile` | A model recompiled from an older one (complete or staged) and then completed saves exactly the bytes of `Model::compile` of the new input, for any edit (surfaces, transforms, cells added, removed or renumbered, fills). An unchanged input reuses every universe; other `Options` reuse none. | `tests/recompile.rs`; the reference model in `tests/big_model.rs` (`--ignored`) |
| `Draft::set`, `Draft::snapshot` | After any sequence of steps, the draft's model classifies points exactly as `reference::classify` of a fresh compile of the edited input, its traces agree with that compile's (`reference::traces_agree`), and its cell boxes contain that compile's. It cannot be saved. A refused step changes nothing. `Draft::begin` refuses what it cannot follow, with the reason. A draft of a universe cut out on its own answers as that universe cut, to the same box, from a fresh compile. | `tests/draft.rs` |
| `GpuModel::patch` | A draft's patch written into its base's upload renders as an upload of a fresh compile of the edited input (with the skip widened by `Draft::widen_skip`). The revert patch gives back the base, and the shader stays the same. | `llevant-gpu/tests/draft.rs` |
| `export` (format `EXPORT_VERSION`) | Indices, programs and boxes as the module documents them; any change to the layout or meaning raises the version, and `llevant-gpu` refuses to build against another one. | the compile-time check in `llevant-gpu`, `llevant-gpu` tests |

## The command line's output

| Output | Guarantee | Enforced by |
|---|---|---|
| `--format json` | A label, a surface and a box have one shape in every command (`crates/llevant-cli/src/json.rs`, the fields of Python's `Label` and `Surface`; `docs/guide/cli.md`, "Scripting and agents"): numbers are numbers, none is `null`. | `tests/cli.rs` (`json_outputs_share_one_vocabulary`) |
| `trace --lines` | Each line traced as a single `--from/--to` run traces it. | `tests/cli.rs` (`trace_many_lines`) |
| Exit status | 1 when the answer is "no" (`check` found problems, `volume --rel-sigma` fell short, `bbox` found nothing, `fingerprint --diff` found differences); 2 on errors, which with JSON output are also `{"error": ...}` on stdout. | `tests/cli.rs` (`check_finds_the_one_real_overlap_of_the_zoo`, `bad_arguments_are_errors`, `trace_many_lines`, `fingerprint_diff_as_json`, `stdin_and_json_errors`) |

## The GPU renderer

`llevant-gpu` is approximate by design (f32, a bounded number of steps): it is held to
agreement with `Model::first_hits`, not to equality. `tests/vs_cpu.rs` and
`llevant-bench gpu-check <model> 150` compare ray by ray over nine views and report the
share of pixels whose cell differs and where; the thresholds are in those tests
(`docs/internals/rendering.md`). Gaps seen through (veils) are held to the CPU's on a
model with holes at every level, with octrees and BVHs (`gaps_seen_through`); rays
that meet an overlap, stopping at it or seeing through it, are not compared.

## Cutting a cell out of another

`Source::cut_plan(keep, cut, simplify)` gives cell `keep` the geometry `A ∩ ¬B` (written
`A #B`, or in fewer terms when `simplify`). The shorter form uses only the laws of
Boolean algebra on the surface senses as independent variables (a macrobody facet is a
variable of its own, and so is each `#n` kept), so it is the same region for any
surfaces. Every call proves it on the card about to be written, read back from its text
as MCNP reads it (`Source::proves`, `algebra::same`): reduced ordered BDDs of `A ∩ ¬B`
and of what the card reads as must be the same node, first with each `#n` a variable of
its own, else with every `#n` expanded by De Morgan. A card proven different is an
error and nothing is written; one too large to prove is written as the plain `A #B`
(`source/algebra.rs` tests, among them 400 random cuts compared with truth tables, and
the BDDs telling a wrong form from a right one; `source/cut.rs`
`the_card_written_is_proven_from_its_text` tells a card that lost its parentheses). A
`keep` with a `TRCL`, a `LIKE` cell, a `keep` that a `LIKE` cell copies, a `keep` cut by
`#cut` already and cells in different universes are refused (the one-step and the
two-step cuts alike, `cut_checks`); a `cut` with a `TRCL` is written as `#n` and not
expanded; the cells that write `#keep` are named in the warnings and in
`CutExpand::users`. `apply_cut` refuses a plan made when `keep`'s card was not as it is
now (`source/cut.rs` tests). `crates/llevant-scene/tests/cut.rs` compiles decks before and after: the
overlap is gone, the points of `A ∩ B` now belong to the cell not cut, every other point
keeps its cell, and the simplified and plain cuts label every point alike.

The viewer cuts in two steps, each its own edit. `Source::cut_expand` writes `A` with
`#B` replaced by the complement of cell B (De Morgan only, so it costs a visit of B's
geometry at any size); its card is proven equal to `A ∩ ¬B` as above, and `empty` is set
when nothing is left (`source/cut.rs` tests). `Source::cell_simplify` shortens the
geometry as it is then, two ways (`algebra::shortest`): with each `#n` kept as a variable,
and with every `#n` expanded; the shorter wins, the first on a tie
(`simplify_keeps_the_other_complements`). Each way, `simplify_bool` tries a tree pass first (`source/algebra/tree.rs`: a literal
known from the products and sums around it is replaced by 0 or 1, `x·¬x = 0`, absorption;
a random test of 400 expressions checks that each result is the same function and not
longer), then the disjunctive-normal-form path, on the expression and on its complement
negated back (an intersection of unions), each only for what has at most 256 products,
under a time limit and a cancel flag (`Limits`)
(`the_dual_form_shortens_an_intersection_of_unions`,
`groups_cut_by_planes_stay_short_and_fast`). The shortest form is given only when
the BDDs prove it equal; the proof or the limits failing gives no form. A cut of 14 groups
by 16 planes, which took seconds in the normal-form path, is done at once (`algebra.rs`
tests). Every plan carries the `Source::revision` it was made at: planning fails if the input changed
while it ran, and `apply_card` writes only if the revision is still that one, the check made
under the lock that writes, so an edit of any card the plan was proven from (the cut cell's,
a `#n` it expanded) refuses it too (`a_plan_is_refused_when_a_cell_it_was_proven_from_changed`,
`a_stale_plan_is_refused`). Revisions are drawn from one counter for the whole process, so no
two sources share one and a plan is never written into another input
(`a_plan_is_bound_to_the_input_it_was_made_from`). A cut that would make the two cells write each other's complement,
directly or through other cells (a `LIKE m BUT` cell counts as writing what m writes, also
among the cells named as writing `#A`, `a_like_copy_of_a_cell_writing_the_complement_is_named`),
any cut of an input that gives `U` or `TRCL` on a data card, which is not read
(`a_cut_is_refused_when_cells_are_placed_by_a_data_card`), and a
`LIKE` cut cell are refused
(`a_cut_that_would_make_cells_complement_each_other_is_refused`,
`a_like_cell_cannot_be_cut_out_of_another`). The Boolean work looks at the limits at the
start of a proof (and of a simplification: none is given once they are over), every 1024
steps of it and while absorbing; a tier-1 product with an atom of both senses is a constant and
absorbs nothing (`a_contradiction_absorbs_nothing`); a `#b` whose copies would be more than a
proof can take is not expanded (`a_complement_written_too_many_times_is_not_expanded`); and it
gives up a product of
unions with over twice the cap of products before absorbing it (absorbing is quadratic), and keeps a product that absorbs below the cap
(`a_complement_that_cannot_be_expanded_does_not_stop_the_cut_one` (a `#n` that cannot be
expanded is one unknown region on both sides of a proof: equal so is equal; unequal so says
nothing), `a_cancelled_proof_gives_no_answer`, `a_huge_product_of_unions_is_given_up_at_once`, `products_that_absorb_are_kept_whatever_they_are_before`).
`crates/llevant-scene/tests/cut.rs` compiles the two steps as well: the expansion fixes
the overlap and changes nothing else, the simplification labels every point as the
expansion, and a cell that writes `#A` takes in `A ∩ B` (the overlap the viewer's verdict
then reports as new, `panels/inspector/cut.rs` `judge` tests). The verdict counts only a
check that covered everywhere A and B met before the cut, in the model's frame
(`cells_meet_where_they_are_placed`; a check covers its boxes only within its region,
`a_check_covers_its_boxes_only_within_its_region`) and that started after
the cut was written (`a_check_started_before_the_cut_does_not_judge_it`, `tests/app/editing.rs`).

## Editing the input

`llevant_scene::source::Source` keeps an input's migjorn tree and changes one card of
it in place (`Source::edit`; `edit_card` on text): every other byte stays as written
(the comment lines just above a card are part of its text), and the cards and lines it
gives after edits are those of a parse of the text (`source.rs` tests). An edit that
cannot be made (no such card, one in a `READ` file, text that does not read as a card
of that kind, a line over 128 columns) is an error and changes nothing; `restore` and
`redo` take an edit back and make it again exactly. The viewer compiles an edit with
`Model::recompile_parsed` from the tree, which gives the bytes of `Model::compile` of
its text (`tests/recompile.rs`, `tests/big_model.rs`), so the model shown after it is
the one `compile` would give; the app test `cards_edited_undone_and_saved` edits,
takes back, saves, edits from the inspector, edits a `TRn` card and saves as.
A drag in the inspector (`surfaces_dragged_live`) has these guarantees:

- each step is previewed by a draft;
- the release writes the card through `Source::surface_with_coeffs` (migjorn's
  `set_coeffs`: everything else as written) and compiles it as any edit;
- a drag cancelled, or taken back to where it began, edits nothing and shows the model
  again.

Cards edited by fields (`Source::cell_with`, `material_with`, `surface_retyped`) are
changed by migjorn on the card alone, so only the words changed differ and the input
itself is touched only by the edit that follows (`source/fields.rs` tests, with the
app test `cell_fields_edited`: a parameter added leaves the rest of the card byte for
byte). A geometry with as many terms keeps the card's line breaks; any other is
written again within 128 columns (`geometry_written_term_by_term`). `wrap_long_lines`
breaks a line over 128 columns between words, or inside a word of geometry before a `:`
or a `)` and after a `(` (a union printed without blanks, as the expansion of a `#n`),
never inside a number or a keyword (nor in what follows the first keyword: a `fill=` range
`0:0` stays whole, on the line of the keyword or a later one,
`a_fill_range_is_never_broken_at_its_colon`, `a_fill_range_on_a_continuation_line_is_never_broken`), counting characters, not bytes. A `$` comment stays
at the end of the last line, with a word moved down to make room, else on a continuation
line of its own while it fits in 128 columns (still a `$` comment, so a label stays one,
`a_label_stays_a_label_when_the_card_wraps`), else it becomes `c` lines after it, all of its
text kept; a cut that makes that happen warns of it before Apply
(`a_label_that_cannot_stay_a_label_is_warned_of`). Every line
it writes fits, except a comment line that was too long already and a piece that cannot be
broken and is longer than the 123 columns a continuation line has after its five blanks,
which are left as they are (`Source::edit` then refuses the card: nothing is written); and the card reads back
the same geometry and parameters (`a_union_without_spaces_is_broken_at_its_colons`,
`a_comment_is_kept_and_the_lines_still_fit`, `non_ascii_lines_are_wrapped_by_characters`,
and 300 random long cards in `random_long_cards_fit_and_read_back_the_same`; a cut by a
cell of 40 planes in `source/cut.rs` `a_long_cut_fits_mcnp_columns`).
Many cards are written as one edit with `Source::edit_many` (all or none: on an error
the cards written are put back) and taken back with `restore_all`; `cells_with` gives
the cards of many cells with the same changes (`many_cards_all_or_none`,
`cell_changes_keep_the_card`). `Source::renumber` changes numbers in every card that
uses them through migjorn's `renumber`, and records each card it changed, before and
after: it snapshots the cards in which one of the old numbers appears as a run of
digits (every card a renumbering can change), so `restore_all` gives the input back
byte for byte (`numbers_changed_everywhere_and_taken_back`). Numbers in use, numbers
not defined, two cells given one number and numbers outside MCNP's range are refused
and change nothing (`renumbering_refused`). The app test `cells_edited_together`
edits three cells from the selection, takes it back with one Undo, renumbers two and
takes that back to the same text.
`Model::point_in_unit` gives a point in the frame of a universe through the fills
above it (`tests/surfaces.rs`), which the slice's marks of a cell's surfaces use
(`marks.rs` test: a sphere's trace within a grid step of it). The cell as edited is
compiled alone from `Source::cell_alone` (`cells_alone_for_the_preview`) and asked with
`Model::cell_holds`, its own program in its universe's frame (`tests/surfaces.rs`);
the `marks.rs` test finds the edge of a moved cell within a grid step, and the app test
`cell_geometry_edited` waits for it on the slice before Apply, and for its shell
traced for the 3D view. `Model::unit_frame` gives the same map as a transform and
`Model::shape_record` a surface written in the problem's frame through it
(`points_in_a_universe_frame`); the 3D view's marks pass (`view3d/edits.rs`) draws a
marked plane within a pixel or two of where it cuts the faces seen, lays a shell with
its outline over them, and leaves the image unchanged, pixel for pixel, when nothing
is marked (`tests/images.rs`, `edit_marks_drawn`). `Source::add_card` adds a card at the
end of its block, with one more value in each data card given per cell, and
`restore_all` takes it out (found by its number) and gives the input back byte for
byte; a number in use, or a card that does not read, changes nothing
(`cards_added_and_taken_out`). `Source::cell_cut` refuses cells with a `TRCL` or
`LIKE` (`cells_cut`). The app test `new_cards_added` adds a surface and a cell inside
it cut out of the cell around, finds the point in the new cell, and takes both back
to the same text. A surface moved in the
inspector writes nothing until Apply (`surfaces_dragged_live`: released, the card is
as it was and the drag held).

## Property tests

The property tests run with a fixed seed in CI and with random seeds nightly
(`.github/workflows/nightly.yml`). A failing seed goes into the test's
`.proptest-regressions` file, so it is replayed from then on.
