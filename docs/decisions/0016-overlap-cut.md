<!-- SPDX-FileCopyrightText: 2026 Alberto Previti -->
<!-- SPDX-License-Identifier: EUPL-1.2 -->
# 0016: An overlap is fixed by cutting one cell out of the other, shortened by Boolean algebra only

2026-11-19, rewritten 2026-11-22 with what was measured on the way. Accepted.

## Context

`check` names the cells of an overlap, but correcting one meant editing the card by hand.
The usual correction is `A := A ∩ #B`, which grows the geometry with every cut, and
nothing shortened a geometry. A first version planned and simplified the cut in one step:
on real cells the user waited for the slowest part before seeing anything, and never saw
whether the overlap was actually solved.

## Decision

- **The viewer cuts in steps, each its own edit** (`cutflow.rs`, `panels/inspector/cut.rs`):
  1. Proposed: `A` with `#B` added, Apply or Cancel.
  2. Written: `Source::cut_expand` plans `#B` as what it stands for, the complement of B
     by De Morgan only (cheap at any size), even when that is no shorter, and
     `Source::apply_expand` writes that plan; the check and the simplification work on
     the card written.
  3. Verified: the check of what changed. `A ∩ ¬B` and `B` are disjoint by construction,
     so what can go wrong is a cell that writes `#A`. It takes in `A ∩ B` and may now overlap
     B, and the verdict reports that as a new overlap (`CutExpand::users`). Revert takes the
     cut back.
  4. Simplified, on request, in the background (20 s, Cancel): `Source::cell_simplify`,
     shown as a `diff -u`.
  5. Accept writes it (a second edit); Discard keeps step 2; Revert takes the whole cut back.

  Python keeps the one-step `Deck.cut` (`Source::cut_plan`): there the plain `A #B` is kept
  on a tie, so the cut keeps following cell B. Both paths share their checks
  (`cut_checks`).
- **Only Boolean algebra.** `+s` and `-s` are a variable and its negation, a macrobody
  facet is a variable of its own, a `#n` kept is a variable too, and nothing is assumed
  about where surfaces are (`-1` does not imply `-2` for nested spheres). A form equal so
  is the same region in any geometry.
- **Nothing is written unproved.** Each form is proven with reduced ordered BDDs, then the
  card about to be written is proven again from its text, as MCNP reads it
  (`Source::proves`), so a printing or line-breaking mistake cannot pass. A card that
  cannot be proven is written as the plain `#B`, or not written at all.
- **Simplification** (`algebra::shortest`, `simplify_bool`), from the cheap to the costly:
  - The other `#n` of the cell are kept as variables (the cell still follows those cells)
    and, as a second try, expanded. The shorter wins, keeping them on a tie.
  - The first tier is a tree pass in the spirit of GEOUNED's `BoolSequence.simplify`
    (which expands on each variable and may use geometric constraints; this one does
    neither): a literal known from the products and sums around it is a constant below;
    `x·¬x = 0`, absorption. It is one visit of the expression, at any size.
  - The second tier takes disjunctive normal form, all prime implicants, an irredundant
    cover and factoring. It runs on the expression and on its complement negated back, so
    it yields both a union of products and an intersection of unions. Each side runs only
    up to 256 products, within the time limit and the cancel flag (`Limits`).
  - The shortest proven form wins, ties keeping the earlier form, and it is written only
    if it has fewer terms than the cell as it is.
- It stays in llevant-scene on migjorn's `Expr` (parser and printer); the engine is not
  involved, and the CLI is unchanged. The cards written keep MCNP's 128 columns
  (`wrap_long_lines`: unions printed without blanks are broken at their `:`).

## Evidence

Release build, 6 cores. Cell A has m groups `(a:b)`; cell B has 16 planes.

| m | tier 1 | its terms | products | normal form | prime implicants | redundancy test | tier 2's terms |
|---|---|---|---|---|---|---|---|
| 8 | 26 µs | 17 | 256 | 1.7 ms | 3.3 ms | 42 ms | 511 |
| 10 | 39 µs | 21 | 1 024 | 30 ms | 68 ms | 0.8 s | 2 047 |
| 12 | 37 µs | 25 | 4 096 | 0.54 s | 1.1 s | 13.8 s | 8 191 |
| 13 | 43 µs | 27 | 8 192 | 2.1 s | 4.7 s | 58 s | 16 383 |
| 14 | 40 µs | 29 | 16 384 | 9.7 s | 21 s | > 119 s | 32 767 |

- **The normal form alone was the wrong shape.** A cell written as an intersection of
  unions has 2^m products, all essential, and factoring does not bring the product back.
  The cost grew about 4 times per group, most of it in the redundancy test. The BDD proof
  stayed under 3 ms, and memory was not the limit (63 MB at 16 384 products).
- **Before the first tier, the one-step plan was slow.** It took 1.5 s at m = 10 and 26 s
  at m = 12, for a result often no shorter. The BDD variable order was not the cause (it is
  the order of first appearance).
- **With the complement.** The groups case takes 3 ms up to m = 18. `(a:b1)…(a:b9)` with k
  groups, which neither tier shortened before, becomes `(a : b1 … b9)` with the groups
  (26 → 18 terms at k = 4, 50 → 42 at k = 16) in 1–6 ms.
- **No case for more time.** A search of 4 000 random expressions found none still over
  256 products after the first tier.

## Consequences

- Some results stay longer than a geometric simplifier would give.
- Once `#B` is written out, cell A no longer follows later edits of cell B. The same holds
  for any other `#n` that Simplify expands; when the kept form is as short, it is kept.
- Refused: a `keep` with a `TRCL` (MCNP moves a `#n` by the cell's own `TRCL`), a `LIKE`
  cell or a LIKE `cut`, a `keep` that a `LIKE` cell copies (it would be cut too), a `keep` cut
  by `#B` already, cells in different universes (as their `u=` say), and a `cut` that writes
  `#keep`, directly or through other cells (the two would complement each other). A `cut`
  with a `TRCL` is written as `#B`.
- Only `#B` has to be expandable. Another `#n` of A that is not (a cell with a `TRCL`) stays
  as written, and the proof treats it as the same unknown region on both sides, so
  `-1 #3` cut by `-2` is `-1 #3 2` (`a_complement_that_cannot_be_expanded_does_not_stop_the_cut_one`).
- A plan is bound to the input revision it was made at; any edit before Apply makes a new
  preview necessary. The verdict counts only a check that looked everywhere A and B met
  before the cut (their instance boxes intersected, in the model's frame: a cell's own box is
  in its universe's frame) and that started after the cut was written (checks are numbered as they start, so one
  begun on the model before the cut is not taken for its check even if it ends after).
- A cell that writes `#keep` is named before Apply, and after it if it overlaps B. It is
  never changed.
- A geometry rewritten loses the comments inside it.
- The "solved" verdict comes from a line-trace check of the changed region: it finds
  overlaps, it does not prove that none is left.

## Left out

- Geometric redundancy removal (surfaces known to lie inside one another): its results
  hold only for those surfaces, and nothing proves them.
- A second tier run in parallel, and a "thorough" Simplify without a time limit: on the
  numbers above they would spend minutes to confirm the first tier. To be looked at again
  if a real cell is still slow or not shortened.
