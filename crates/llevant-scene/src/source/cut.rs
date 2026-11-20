// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! Cutting a cell out of another one (the fix for an overlap): `A := A ∩ #B`,
//! written in the fewest terms the Boolean algebra gives
//! ([`algebra`](super::algebra)).

use super::algebra::{self, Cut, Limits};
use super::fields::CellChange;
use super::{geometry_words, CardKind, Edited, Source, Word};

/// What cutting cell `cut` out of cell `keep` would do, before anything is
/// written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CutPlan {
    /// The cell that changes.
    pub keep: i64,
    /// The cell taken out of it.
    pub cut: i64,
    /// The card of `keep` as it is.
    pub old_card: String,
    /// The card of `keep` after the cut (as `old_card` if there is nothing
    /// to write: see [`Cut::empty`]).
    pub new_card: String,
    /// The geometry, the terms before and after, the laws applied and
    /// anything to look at.
    pub result: Cut,
}

/// What cutting cell `cut` out of cell `keep` writes first (the cheap step):
/// the geometry of `keep` with `#cut` replaced by the complement of cell
/// `cut`, expanded by De Morgan only. Nothing is written until
/// [`Source::apply_card`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CutExpand {
    pub keep: i64,
    pub cut: i64,
    /// The card of `keep` as it is.
    pub old_card: String,
    /// The card with `#cut` added to the geometry as it is written, and the
    /// range of it that is new (for the first look: before expanding).
    pub plain_geometry: String,
    pub plain_mark: std::ops::Range<usize>,
    /// The card of `keep` as written (same as `old_card` when `empty`).
    pub new_card: String,
    /// Its geometry, and the range of it that is the expansion.
    pub geometry: String,
    pub mark: std::ops::Range<usize>,
    /// `keep` is inside `cut`: cutting would leave nothing.
    pub empty: bool,
    /// `#cut` could not be expanded and stays `#cut`.
    pub plain: bool,
    /// The cells that write `#keep` (they take in what is cut), and other
    /// things to look at.
    pub warnings: Vec<String>,
}

/// What simplifying the geometry of a cell gives, before it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Simplified {
    pub keep: i64,
    pub old_card: String,
    /// The card with the shorter geometry (as `old_card` if none is).
    pub new_card: String,
    /// Terms (surfaces and `#n`) before and after.
    pub terms_before: usize,
    pub terms_after: usize,
    /// A shorter geometry, proven equal, was found.
    pub simplified: bool,
    /// The laws applied, and what to look at.
    pub trace: Vec<String>,
    pub warnings: Vec<String>,
}

impl Source {
    /// The checks every cut makes (see [`Source::cut_plan`]): the card of
    /// `keep`, and the cells that write `#keep`.
    fn cut_checks(&self, keep: i64, cut: i64) -> Result<(String, Vec<i64>), String> {
        if keep == cut {
            return Err(format!("cell {keep} cannot be cut out of itself"));
        }
        let f = self
            .cell_fields(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?;
        let g = self
            .cell_fields(cut)
            .ok_or_else(|| format!("no cell {cut} in the input"))?;
        if f.like.is_some() {
            return Err(format!("cell {keep} is a LIKE cell: cut it by hand"));
        }
        if f.params.iter().any(|p| p.key == "trcl") {
            return Err(format!(
                "cell {keep} has a TRCL, which would move #{cut} with it: cut it by hand"
            ));
        }
        let universe = |fields: &super::fields::CellFields| {
            fields
                .params
                .iter()
                .find(|p| p.key == "u")
                .map(|p| p.value.trim_start_matches('-').to_string())
                .unwrap_or_else(|| "0".into())
        };
        if universe(&f) != universe(&g) {
            return Err(format!(
                "cells {keep} and {cut} are in different universes: they cannot overlap"
            ));
        }
        let old_card = self
            .card(CardKind::Cell, keep)
            .ok_or_else(|| format!("cell {keep} is not in the input (in a READ file?)"))?
            .text;
        let tree = self.tree();
        let mut users = Vec::new();
        for c in tree.cells() {
            let Some(n) = c.id() else { continue };
            if n == keep {
                continue;
            }
            if c.like() == Some(keep) {
                return Err(format!(
                    "cell {n} is LIKE {keep} BUT: it would be cut too; cut it by hand"
                ));
            }
            let complements = geometry_words(&c.geometry_text())
                .iter()
                .any(|w| matches!(w.kind, Word::Cell(m) if m == keep));
            if complements && !users.contains(&n) {
                users.push(n);
            }
        }
        Ok((old_card, users))
    }

    /// Step one of a cut: cell `cut` taken out of cell `keep` by writing
    /// the complement of `cut`'s geometry into `keep`'s, expanded by De
    /// Morgan (cheap, valid at any size, proven equal to `A ∩ ¬B`). The
    /// refusals are those of [`Source::cut_plan`]. A cell cut already by
    /// `#cut` is an error.
    pub fn cut_expand(&self, keep: i64, cut: i64, limits: &Limits) -> Result<CutExpand, String> {
        let (old_card, users) = self.cut_checks(keep, cut)?;
        let tree = self.tree();
        let a = tree
            .cell(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?
            .geometry_expr()
            .map_err(|e| format!("cell {keep}: {}", e.message))?;
        if matches!(&a, migjorn::Expr::And(v) if v.contains(&migjorn::Expr::Complement(cut)))
            || a == migjorn::Expr::Complement(cut)
        {
            return Err(format!("cell {keep} is cut by #{cut} already"));
        }
        let cells = |n: i64| expandable(&tree, n);
        let e = algebra::expand_cut(&a, cut, &cells, limits)?;
        drop(tree);
        let mut warnings = e.warnings.clone();
        for n in users {
            warnings.push(format!(
                "cell {n} writes #{keep}: it will take in what cell {cut} has of cell {keep}"
            ));
        }
        let new_card = if e.empty {
            old_card.clone()
        } else {
            self.cell_with(keep, &[CellChange::Geometry(e.geometry.clone())])?
        };
        Ok(CutExpand {
            keep,
            cut,
            old_card,
            plain_geometry: e.plain_geometry,
            plain_mark: e.plain_mark,
            new_card,
            geometry: e.geometry,
            mark: e.mark,
            empty: e.empty,
            plain: e.plain,
            warnings,
        })
    }

    /// Step two of a cut: the geometry of cell `keep` as it is in the input
    /// in the fewest terms the Boolean algebra gives, proven equal to it
    /// (every `#n` is expanded to do it, and the result has none). `limits`
    /// stop the long work; nothing is written (see
    /// [`Source::apply_simplified`]).
    pub fn cell_simplify(&self, keep: i64, limits: &Limits) -> Result<Simplified, String> {
        let old_card = self
            .card(CardKind::Cell, keep)
            .ok_or_else(|| format!("cell {keep} is not in the input (in a READ file?)"))?
            .text;
        let f = self
            .cell_fields(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?;
        if f.like.is_some() {
            return Err(format!("cell {keep} is a LIKE cell: simplify it by hand"));
        }
        if f.params.iter().any(|p| p.key == "trcl") {
            return Err(format!("cell {keep} has a TRCL: simplify it by hand"));
        }
        let tree = self.tree();
        let a = tree
            .cell(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?
            .geometry_expr()
            .map_err(|e| format!("cell {keep}: {}", e.message))?;
        let terms_before = algebra::terms(&a);
        let cells = |n: i64| expandable(&tree, n);
        let mut nodes = 0;
        let mut r = Simplified {
            keep,
            old_card: old_card.clone(),
            new_card: old_card.clone(),
            terms_before,
            terms_after: terms_before,
            simplified: false,
            trace: Vec::new(),
            warnings: Vec::new(),
        };
        let reference = match algebra::to_bool(&a, Some(&cells), &mut Vec::new(), &mut nodes) {
            Ok(b) => b,
            Err(why) => {
                r.warnings.push(format!("not simplified: {why}"));
                return Ok(r);
            }
        };
        let s = algebra::simplify_bool(&reference, limits)?;
        drop(tree);
        r.trace = s.trace;
        r.warnings = s.warnings;
        if s.empty {
            r.warnings
                .push("the geometry is empty: nothing to simplify".into());
            return Ok(r);
        }
        match s.expr {
            Some(e) if s.terms < terms_before => {
                r.new_card = self.cell_with(keep, &[CellChange::Geometry(e.to_string())])?;
                r.terms_after = s.terms;
                r.simplified = true;
            }
            Some(_) => r
                .warnings
                .push("the geometry is as short as the algebra gets".into()),
            None => {}
        }
        Ok(r)
    }

    /// Writes the card `new_card` of cell `keep`, planned when its card was
    /// `old_card` (undone with [`Source::restore`]). An error if the card
    /// is not `old_card` now (plan again).
    pub fn apply_card(&self, keep: i64, old_card: &str, new_card: &str) -> Result<Edited, String> {
        let now = self.card(CardKind::Cell, keep).map(|c| c.text);
        if now.as_deref() != Some(old_card) {
            return Err(format!(
                "cell {keep} has changed since the cut was planned: plan it again"
            ));
        }
        self.edit(CardKind::Cell, keep, new_card)
    }

    /// Writes an expansion made by [`Source::cut_expand`].
    pub fn apply_expand(&self, plan: &CutExpand) -> Result<Edited, String> {
        if plan.empty {
            return Err(format!(
                "cell {} lies inside cell {}: cutting would leave nothing",
                plan.keep, plan.cut
            ));
        }
        self.apply_card(plan.keep, &plan.old_card, &plan.new_card)
    }

    /// Writes a simplification made by [`Source::cell_simplify`].
    pub fn apply_simplified(&self, s: &Simplified) -> Result<Edited, String> {
        if !s.simplified {
            return Err("nothing shorter to write".into());
        }
        self.apply_card(s.keep, &s.old_card, &s.new_card)
    }

    /// What cutting cell `cut` out of cell `keep` gives: its geometry
    /// `A` becomes `A #B`, simplified by Boolean algebra when `simplify` (the
    /// result is proven equal to `A ∩ ¬B`, whatever the surfaces are; see
    /// [`algebra`](super::algebra)). Nothing is written: [`Source::apply_cut`]
    /// does.
    ///
    /// An error for a cell the input has not got, cells in different
    /// universes (`u=` on their cards), a `LIKE` cell, a `keep` with a `TRCL`
    /// (MCNP moves the surfaces of a `#n` by the cell's own `TRCL`) and a
    /// `keep` that a `LIKE` cell copies (that cell would be cut too). A `cut`
    /// with a `TRCL`, or `#n` of a cell that cannot be expanded, is written
    /// as `#n` and not simplified. A warning for each cell that writes
    /// `#keep`: it then takes in what is cut.
    pub fn cut_plan(&self, keep: i64, cut: i64, simplify: bool) -> Result<CutPlan, String> {
        if keep == cut {
            return Err(format!("cell {keep} cannot be cut out of itself"));
        }
        let f = self
            .cell_fields(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?;
        let g = self
            .cell_fields(cut)
            .ok_or_else(|| format!("no cell {cut} in the input"))?;
        if f.like.is_some() {
            return Err(format!("cell {keep} is a LIKE cell: cut it by hand"));
        }
        if f.params.iter().any(|p| p.key == "trcl") {
            return Err(format!(
                "cell {keep} has a TRCL, which would move #{cut} with it: cut it by hand"
            ));
        }
        let universe = |fields: &super::fields::CellFields| {
            fields
                .params
                .iter()
                .find(|p| p.key == "u")
                .map(|p| p.value.trim_start_matches('-').to_string())
                .unwrap_or_else(|| "0".into())
        };
        if universe(&f) != universe(&g) {
            return Err(format!(
                "cells {keep} and {cut} are in different universes: they cannot overlap"
            ));
        }

        let old_card = self
            .card(CardKind::Cell, keep)
            .ok_or_else(|| format!("cell {keep} is not in the input (in a READ file?)"))?
            .text;
        let (mut result, already, users) = {
            let tree = self.tree();
            let a = tree
                .cell(keep)
                .ok_or_else(|| format!("no cell {keep} in the input"))?
                .geometry_expr()
                .map_err(|e| format!("cell {keep}: {}", e.message))?;
            // The cells that copy `keep` or complement it change with it.
            let mut users = Vec::new();
            for c in tree.cells() {
                let Some(n) = c.id() else { continue };
                if n == keep {
                    continue;
                }
                if c.like() == Some(keep) {
                    return Err(format!(
                        "cell {n} is LIKE {keep} BUT: it would be cut too; cut it by hand"
                    ));
                }
                let complements = geometry_words(&c.geometry_text())
                    .iter()
                    .any(|w| matches!(w.kind, Word::Cell(m) if m == keep));
                if complements && !users.contains(&n) {
                    users.push(n);
                }
            }
            // A cell `#n` can be expanded through: its own geometry, no
            // `TRCL`, not a `LIKE`.
            let cells = |n: i64| {
                let c = tree.cell(n)?;
                if c.like().is_some()
                    || c.params()
                        .iter()
                        .any(|p| p.qualified_key().eq_ignore_ascii_case("trcl"))
                {
                    return None;
                }
                c.geometry_expr().ok()
            };
            let mut r = if simplify {
                algebra::cut_expr(&a, cut, &cells)?
            } else {
                let mut r = algebra::cut_expr(&a, cut, &|_| None)?;
                // Its "cannot be expanded" is the point of a plain cut.
                r.warnings.clear();
                r.trace.clear();
                r
            };
            // `A` has `#B` already: nothing to write (not even a shorter `A`).
            let already = r.plain == a.to_string();
            if already {
                r.geometry = r.plain.clone();
                r.terms_after = r.terms_before;
                r.simplified = false;
                r.warnings = vec![format!("cell {keep} is cut by #{cut} already")];
            }
            (r, already, users)
        };
        for n in users {
            result.warnings.push(format!(
                "cell {n} writes #{keep}: it will take in what cell {cut} has of cell {keep}"
            ));
        }
        let new_card = if result.empty || already {
            old_card.clone()
        } else {
            self.cell_with(keep, &[CellChange::Geometry(result.geometry.clone())])?
        };
        Ok(CutPlan {
            keep,
            cut,
            old_card,
            new_card,
            result,
        })
    }

    /// Writes a plan: the card of `plan.keep` replaced (undone with
    /// [`Source::restore`]). An error for a plan with nothing to write (the
    /// cell is inside the one cut out of it), and for one made when the card
    /// was not as it is now (plan again).
    pub fn apply_cut(&self, plan: &CutPlan) -> Result<Edited, String> {
        if plan.result.empty {
            return Err(format!(
                "cell {} lies inside cell {}: cutting would leave nothing",
                plan.keep, plan.cut
            ));
        }
        let now = self.card(CardKind::Cell, plan.keep).map(|c| c.text);
        if now.as_deref() != Some(plan.old_card.as_str()) {
            return Err(format!(
                "cell {} has changed since the cut was planned: plan it again",
                plan.keep
            ));
        }
        self.edit(CardKind::Cell, plan.keep, &plan.new_card)
    }
}

/// The geometry of cell `n` for a `#n` to expand through: its own, with no
/// `TRCL`, not a `LIKE`.
fn expandable(tree: &migjorn::Model, n: i64) -> Option<migjorn::Expr> {
    let c = tree.cell(n)?;
    if c.like().is_some()
        || c.params()
            .iter()
            .any(|p| p.qualified_key().eq_ignore_ascii_case("trcl"))
    {
        return None;
    }
    c.geometry_expr().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: &str = "cut
1 0 (-1:-2) 3 imp:n=1 $ the cell
2 0 -1 3 imp:n=1
3 0 -1 3 imp:n=1 u=4
4 0 -1 imp:n=1 trcl=(1 0 0)
5 like 4 but imp:n=2

1 so 5
2 so 9
3 px 0

";

    #[test]
    fn plain_and_simplified() {
        let s = Source::from_str(INPUT);
        let plain = s.cut_plan(1, 2, false).unwrap();
        assert_eq!(plain.new_card, "1 0 (-1:-2) 3 #2 imp:n=1 $ the cell\n");
        let p = s.cut_plan(1, 2, true).unwrap();
        assert_eq!(p.result.geometry, "-2 3 1");
        assert!(p.result.simplified);
        assert!(
            p.new_card.starts_with("1 0 -2 3 1 imp:n=1"),
            "{}",
            p.new_card
        );
        assert!(p.new_card.contains("$ the cell"), "{}", p.new_card);
        assert!(!p.result.trace.is_empty());
    }

    #[test]
    fn apply_edits_the_card_and_restore_undoes_it() {
        let s = Source::from_str(INPUT);
        let p = s.cut_plan(1, 2, true).unwrap();
        let e = s.apply_cut(&p).unwrap();
        assert!(s.to_source().contains("1 0 -2 3 1 imp:n=1"));
        s.restore(&e).unwrap();
        assert_eq!(s.to_source(), INPUT);
    }

    #[test]
    fn refusals() {
        let s = Source::from_str(INPUT);
        assert!(s.cut_plan(1, 1, true).is_err());
        assert!(s.cut_plan(1, 9, true).unwrap_err().contains("no cell 9"));
        assert!(s.cut_plan(1, 3, true).unwrap_err().contains("universes"));
        assert!(s.cut_plan(4, 1, true).unwrap_err().contains("TRCL"));
        assert!(s.cut_plan(5, 1, true).unwrap_err().contains("LIKE"));
    }

    #[test]
    fn a_cut_with_a_trcl_is_written_plain() {
        let s = Source::from_str(INPUT);
        let p = s.cut_plan(1, 4, true).unwrap();
        assert!(!p.result.simplified);
        assert!(p.new_card.contains("#4"), "{}", p.new_card);
        assert!(
            p.result.warnings[0].contains("cell 4"),
            "{:?}",
            p.result.warnings
        );
    }

    #[test]
    fn nothing_is_written_for_a_cell_inside_the_other() {
        let s = Source::from_str(INPUT);
        let p = s.cut_plan(2, 1, true);
        // Cell 2 (-1 3) is inside cell 1 ((-1:-2) 3): nothing left.
        let p = p.unwrap();
        assert!(p.result.empty);
        assert_eq!(p.new_card, p.old_card);
        assert!(s.apply_cut(&p).unwrap_err().contains("inside"));
    }

    #[test]
    fn nested_spheres_are_not_known_to_the_algebra() {
        // Sphere 1 is inside sphere 2, so cell 1 (-1 3) lies inside cell 2
        // (-2 3) and nothing of it is left. The algebra does not know where
        // surfaces are: -1 3 (2 : -3) is -1 3 2, as long as the plain cut,
        // which is kept.
        let s =
            Source::from_str("t\n1 0 -1 3 imp:n=1\n2 0 -2 3 imp:n=1\n\n1 so 5\n2 so 9\n3 px 0\n\n");
        let p = s.cut_plan(1, 2, true).unwrap();
        assert!(!p.result.empty);
        assert!(!p.result.simplified);
        assert_eq!(p.result.geometry, "-1 3 #2");
    }

    #[test]
    fn a_tie_keeps_the_plain_cut() {
        // (-2:-4) 3 outside -1 3 is 3 1 (-2:-4): four terms, as the plain cut
        // (the viewer's test makes this cut too, when it has a GPU).
        let s = Source::from_str(
            "t\n2 0 (-2:-4) 3 imp:n=1\n3 0 -1 3 imp:n=1\n\n1 so 5\n2 sx 4 5\n3 px 0\n4 so 9\n\n",
        );
        let p = s.cut_plan(2, 3, true).unwrap();
        assert!(!p.result.simplified);
        assert_eq!(p.new_card.trim_end(), "2 0 (-2:-4) 3 #3 imp:n=1");
    }

    #[test]
    fn a_stale_plan_is_refused() {
        let s = Source::from_str(INPUT);
        let first = s.cut_plan(1, 2, false).unwrap();
        let second = s.cut_plan(1, 4, false).unwrap();
        s.apply_cut(&first).unwrap();
        let err = s.apply_cut(&second).unwrap_err();
        assert!(err.contains("changed since"), "{err}");
        assert!(s.to_source().contains("#2"), "the first cut is kept");
    }

    #[test]
    fn a_cell_copied_by_like_is_refused() {
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 imp:n=1\n3 like 1 but imp:n=2\n\n1 so 5\n2 sx 4 5\n\n",
        );
        let err = s.cut_plan(1, 2, true).unwrap_err();
        assert!(err.contains("cell 3 is LIKE 1"), "{err}");
        assert!(s.cut_plan(2, 1, true).is_ok());
    }

    #[test]
    fn cells_complementing_the_cut_one_are_named() {
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 imp:n=1\n3 0 #1 #2 -3 imp:n=1\n4 0 3 imp:n=0\n\n\
             1 so 5\n2 sx 4 5\n3 so 50\n\n",
        );
        let p = s.cut_plan(1, 2, true).unwrap();
        assert!(
            p.result
                .warnings
                .iter()
                .any(|w| w.contains("cell 3 writes #1")),
            "{:?}",
            p.result.warnings
        );
    }

    #[test]
    fn a_cell_cut_already_is_left_as_it_is() {
        let s = Source::from_str("t\n1 0 -1 #2 imp:n=1\n2 0 -2 imp:n=1\n\n1 so 5\n2 sx 4 5\n\n");
        for simplify in [false, true] {
            let p = s.cut_plan(1, 2, simplify).unwrap();
            assert_eq!(p.new_card, p.old_card);
            assert!(!p.result.simplified);
            assert!(
                p.result.warnings[0].contains("already"),
                "{:?}",
                p.result.warnings
            );
        }
    }

    #[test]
    fn expand_writes_the_complement_and_marks_it() {
        let s = Source::from_str(INPUT);
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        // Cell 2 is `-1 3`: its complement is `(1:-3)`.
        assert_eq!(e.geometry, "(-1:-2) 3 (1:-3)");
        assert_eq!(&e.geometry[e.mark.clone()], "(1:-3)");
        assert_eq!(e.plain_geometry, "(-1:-2) 3 #2");
        assert_eq!(&e.plain_geometry[e.plain_mark.clone()], "#2");
        assert!(e.new_card.contains("(-1:-2) 3 (1:-3)"), "{}", e.new_card);
        assert!(e.new_card.contains("$ the cell"));
        assert!(!e.empty && !e.plain);
        // Written, it is one edit that restore undoes.
        let ed = s.apply_expand(&e).unwrap();
        assert!(s.to_source().contains("(-1:-2) 3 (1:-3)"));
        s.restore(&ed).unwrap();
        assert_eq!(s.to_source(), INPUT);
    }

    #[test]
    fn expand_refusals_and_special_cases() {
        let s = Source::from_str(INPUT);
        let l = Limits::default();
        assert!(s.cut_expand(1, 1, &l).is_err());
        assert!(s.cut_expand(1, 3, &l).unwrap_err().contains("universes"));
        assert!(s.cut_expand(4, 1, &l).unwrap_err().contains("TRCL"));
        // A cut with a TRCL stays `#4`.
        let e = s.cut_expand(1, 4, &l).unwrap();
        assert!(e.plain);
        assert_eq!(&e.geometry[e.mark.clone()], "#4");
        // Inside the other: nothing is left, nothing is written.
        let e = s.cut_expand(2, 1, &l).unwrap();
        assert!(e.empty);
        assert_eq!(e.new_card, e.old_card);
        assert!(s.apply_expand(&e).unwrap_err().contains("inside"));
        // Cut already.
        let t = Source::from_str("t\n1 0 -1 #2 imp:n=1\n2 0 -2 imp:n=1\n\n1 so 5\n2 sx 4 5\n\n");
        assert!(t.cut_expand(1, 2, &l).unwrap_err().contains("already"));
    }

    #[test]
    fn expand_names_the_cells_that_write_the_cut_one() {
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 imp:n=1\n3 0 #1 #2 -3 imp:n=1\n4 0 3 imp:n=0\n\n\
             1 so 5\n2 sx 4 5\n3 so 50\n\n",
        );
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        assert!(e.warnings.iter().any(|w| w.contains("cell 3 writes #1")));
    }

    #[test]
    fn simplify_shortens_the_geometry_written_and_proves_it() {
        let s = Source::from_str(INPUT);
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        s.apply_expand(&e).unwrap();
        // (-1:-2) 3 (1:-3) = (-1:-2) 3 1 = -2 3 1.
        let r = s.cell_simplify(1, &Limits::default()).unwrap();
        assert!(r.simplified, "{:?}", r.warnings);
        assert!(r.terms_after < r.terms_before);
        assert!(r.new_card.contains("-2 3 1"), "{}", r.new_card);
        assert!(r.new_card.contains("$ the cell"));
        let ed = s.apply_simplified(&r).unwrap();
        assert!(s.to_source().contains("1 0 -2 3 1 imp:n=1"));
        s.restore(&ed).unwrap();
        // A second time, from the shorter card: nothing to do.
        let again = s.cell_simplify(1, &Limits::default()).unwrap();
        assert!(again.simplified, "(the card was restored)");
    }

    #[test]
    fn simplify_of_a_short_geometry_is_a_no_op_and_a_stale_card_is_refused() {
        let s = Source::from_str(INPUT);
        let r = s.cell_simplify(2, &Limits::default()).unwrap();
        assert!(!r.simplified);
        assert_eq!(r.new_card, r.old_card);
        assert!(s.apply_simplified(&r).is_err());
        // Planned, then the card changes.
        let s = Source::from_str(INPUT);
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        let r = {
            s.apply_expand(&e).unwrap();
            s.cell_simplify(1, &Limits::default()).unwrap()
        };
        s.edit(CardKind::Cell, 1, "1 0 -1 imp:n=1\n").unwrap();
        assert!(s
            .apply_simplified(&r)
            .unwrap_err()
            .contains("changed since"));
    }
}
