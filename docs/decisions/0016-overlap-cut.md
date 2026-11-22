<!-- SPDX-FileCopyrightText: 2026 Alberto Previti -->
<!-- SPDX-License-Identifier: EUPL-1.2 -->
# 0016: An overlap is fixed by cutting one cell out of the other, shortened by Boolean algebra only

2026-11-19. Accepted.

## Context

`check` names the cells of an overlap, but correcting one meant editing the card by hand.
The usual correction is `A := A ∩ #B`, which grows the geometry with every cut. The
viewer already added `#n` to a new cell's carved-out cell (`Source::cell_cut`), and
nothing shortened a geometry.

## Decision

- `Source::cut_plan` / `apply_cut` (llevant-scene) write `A #B` or the shortest of the
  forms the algebra gives: `#n` expanded by De Morgan, negation normal form, disjunctive
  normal form (`x·¬x = 0`, idempotence, absorption), all prime implicants by iterated
  consensus, an irredundant cover, factoring. The user picks which cell is cut.
- Only Boolean algebra: `+s` and `-s` are a variable and its negation, and nothing is
  assumed about where surfaces are (`-1` does not imply `-2` for nested spheres). The
  result is then the same region in any geometry, and the call proves it with reduced
  ordered BDDs. Geometric redundancy removal was left out for that reason.
- It stays in llevant-scene on migjorn's `Expr` (parser and printer); the engine is not
  involved. The CLI is unchanged; Python has `Deck`, the viewer a button per overlap.
- The plain `A #B` is kept when the algebra is not shorter (a tie too), so the cut keeps
  following cell B if B is edited later.

## Consequences

Some results stay longer than a geometric simplifier would give. A simplified cut has
every `#n` it went through (B, and those of A and B) written out as surfaces, so it no
longer follows later edits of those cells. Refused: a `keep` with a `TRCL` (MCNP moves a
`#n` by the cell's own `TRCL`), a `LIKE` cell, a `keep` that a `LIKE` cell copies (it
would be cut too) and cells in different universes (as their `u=` say); a `cut` with a
`TRCL` is written plain. A cell that writes `#keep` takes in what is cut: it is named in
the warnings, not changed. A geometry rewritten, plain or simplified, loses comments
inside it.

## Amendment, 2026-11-20: two steps, and a cheap first tier

Measured (`cut_expr` stages, A with m groups of two literals, B with n planes): the
normal-form path grew about 16 times for every two groups, 5 ms at 6 groups, 1.5 s at 10
and 26 s at 12 (4096 products), 95% of it in the redundancy test and the normal form, the
BDD proof under 6 ms, and the result was often no shorter. So:

- The viewer cuts in two edits: `cut_expand` writes `A` with `#B` expanded (De Morgan only),
  the check of what changed says whether the overlap is solved, and `cell_simplify` is a
  button, in the background, shown as a `diff -u`, accepted or discarded. Discarding keeps
  the expanded cut; Revert takes the whole cut back. "Plain cuts" is gone (the python
  `Deck.cut_plan` keeps `simplify=False`).
- `simplify_bool` tries a tree pass first (GEOUNED's idea in `BoolSequence.simplify`:
  a literal known from the products and sums around it is a constant below; no normal form),
  then the old path only for at most 256 products and under `Limits` (time, cancel).
  GEOUNED proves nothing; here the BDD proof stays, and the result is given only with it.
- The BDD variable order was not the problem (it is the order of first appearance).

## Amendment, 2026-11-20: what the viewer gives up, what it checks, what is proven

- The viewer writes `#B` expanded even when the algebra is no shorter, so cell A no
  longer follows later edits of cell B (the tie rule above stays for `cut_plan`). This
  was chosen: the expansion is what the user reviews and what Simplify works on.
- `A ∩ ¬B` and `B` are disjoint by construction, so the check after the cut cannot find
  them overlapping. What can go wrong is a cell that writes `#A`: it takes in `A ∩ B`
  and may overlap B. The verdict reports that as a new overlap (`CutExpand::users`).
- The proof is made on the card about to be written, read back from its text, not on the
  expression it was printed from, so a printing mistake cannot pass.
- The simplification keeps each other `#n` as a variable of its own (a form equal so is
  equal for any cell n) when that is as short, so those cells are still followed. Before,
  every `#n` was written out, and the result was seldom shorter than the `#n` it replaced.
- `cut_plan` and the two steps share their checks, so a cell cut by `#B` already is refused
  by both.


## Amendment, 2026-11-22: the second tier on the complement too

Measured (release, the second tier uncapped, A of m groups `(a:b)` cut by 16 planes): the
first tier gives the short form (2m+1 terms) in microseconds; the normal form has 2^m
products, all essential, and factoring does not bring the product of sums back, so the
second tier wrote 2^(m+1)−1 terms (511 at m = 8, 32 767 at m = 14), at a cost growing about
4 times per group, most of it in the redundancy test (14 s at m = 12, 58 s at m = 13). Memory
was not the limit (63 MB at 16 384 products). A search of 4 000 random expressions found
none still over 256 products after the first tier. More time, or cores, would not have
helped: a union of products is the wrong shape for MCNP cells, which are mostly
intersections of unions.

- The second tier also runs on the complement, negated back by De Morgan: an intersection
  of unions. The complement of m groups has m products, so what explodes one way is small
  the other. Each side has its own cap of 256 products; the shortest proven form wins, a
  tie keeping the earlier one, so no result is longer than before.
- Measured after: the groups case 3 ms up to m = 18; `(a:b1)…(a:b9)` with k groups,
  which neither tier shortened before, becomes `(a : b1 … b9)` with the groups (26 → 18
  terms at k = 4, 50 → 42 at k = 16) in 1–6 ms.
- A parallel second tier and a "thorough" Simplify without a time limit were considered
  and dropped: on these numbers they would spend minutes to confirm the first tier. To be
  looked at again if a real cell is still slow or not shortened.
