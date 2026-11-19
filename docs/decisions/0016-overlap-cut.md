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
