// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! Cutting a cell out of another one (the fix for an overlap): `A := A ∩ #B`,
//! written in the fewest terms the Boolean algebra gives
//! (the `algebra` module).

use std::collections::{HashMap, HashSet};

use super::algebra::{self, Cut, Limits};
use super::fields::CellChange;
use super::{geometry_words, CardKind, Edited, Source, Word};
use migjorn::Expr;

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
    /// [`Source::revision`] when it was planned: a plan is written only if
    /// the input has not changed since (what it was proven from included).
    pub revision: u64,
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
    /// [`Source::revision`] when it was planned (see [`CutPlan::revision`]).
    pub revision: u64,
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
    /// The cells that write `#keep` (they may take in what is cut), and other
    /// things to look at.
    pub warnings: Vec<String>,
    /// The cells that change with `keep` (by number): those that write
    /// `#keep`, and in turn those that write `#n` of one of them or copy one
    /// with `LIKE`. They may take in what is cut, and overlap `cut` after it.
    pub users: Vec<i64>,
}

/// What simplifying the geometry of a cell gives, before it is written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Simplified {
    pub keep: i64,
    pub old_card: String,
    /// [`Source::revision`] when it was planned (see [`CutPlan::revision`]).
    pub revision: u64,
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

/// The cells that change with the cell cut, each with the cell it changes
/// through (that cell itself for one that writes its `#n`).
type Users = Vec<(i64, i64)>;

impl Source {
    /// The checks every cut makes (see [`Source::cut_plan`]): the card of
    /// `keep`, its geometry, and the cells that change with it, each with
    /// the cell it changes through.
    fn cut_checks(&self, keep: i64, cut: i64) -> Result<(String, Expr, Users), String> {
        if keep == cut {
            return Err(format!("cell {keep} cannot be cut out of itself"));
        }
        let f = self
            .cell_fields(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?;
        let g = self
            .cell_fields(cut)
            .ok_or_else(|| format!("no cell {cut} in the input"))?;
        if let Some(why) = f.rewrite_refusal(keep, "cut") {
            return Err(why);
        }
        if g.like.is_some() {
            // It takes its universe (and more) from the cell it copies.
            return Err(format!("cell {cut} is a LIKE cell: cut by hand"));
        }
        let old_card = self
            .card(CardKind::Cell, keep)
            .ok_or_else(|| format!("cell {keep} is not in the input (in a READ file?)"))?
            .text;
        let tree = self.tree();
        if let Some(why) = placed_by_data_card(&tree, "cut") {
            return Err(why);
        }
        // The universes as numbers (`U=01` is universe 1, `U=-1` too).
        let universe = |n: i64| -> Result<i64, String> {
            let c = tree
                .cell(n)
                .ok_or_else(|| format!("no cell {n} in the input"))?;
            let u = c
                .universe()
                .map_err(|e| format!("cell {n}: {}", e.message))?;
            Ok(u.unwrap_or(0).abs())
        };
        if universe(keep)? != universe(cut)? {
            return Err(format!(
                "cells {keep} and {cut} are in different universes: they cannot overlap"
            ));
        }
        let a = tree
            .cell(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?
            .geometry_expr()
            .map_err(|e| format!("cell {keep}: {}", e.message))?;
        if algebra::already_cut(&a, cut) {
            return Err(format!("cell {keep} is cut by #{cut} already"));
        }
        // Each cell, what it stands on: its `LIKE` base and the cells it
        // writes `#n` of.
        let mut on: Vec<(i64, Vec<i64>)> = Vec::new();
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
            let mut deps = complemented(&tree, n);
            deps.extend(c.like());
            on.push((n, deps));
        }
        // The cells the cut changes: those that write `#keep`, and in turn
        // those that write `#n` of one of them or copy one with `LIKE`
        // (through two complements a cell grows again: all are listed).
        // Each with the cell it changes through (`keep` itself for a cell
        // that writes `#keep`).
        // Level by level from `keep` (in deck order within a level), over
        // an index of who stands on whom built once: linear in the deck and
        // its references even for a long chain.
        let mut users: Vec<(i64, i64)> = Vec::new();
        let mut rev: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, (_, deps)) in on.iter().enumerate() {
            for &d in deps {
                rev.entry(d).or_default().push(i);
            }
        }
        let mut changed: HashSet<i64> = HashSet::from([keep]);
        let mut found = vec![false; on.len()];
        let mut frontier = vec![keep];
        while !frontier.is_empty() {
            let mut level: Vec<usize> = frontier
                .iter()
                .filter_map(|d| rev.get(d))
                .flatten()
                .copied()
                .filter(|&i| !found[i])
                .collect();
            level.sort_unstable();
            level.dedup();
            frontier.clear();
            for &i in &level {
                found[i] = true;
                let (n, deps) = &on[i];
                if let Some(&via) = deps.iter().find(|d| changed.contains(d)) {
                    users.push((*n, via));
                    frontier.push(*n);
                }
            }
            changed.extend(frontier.iter().copied());
        }
        if let Some(via) = reaches(&tree, cut, keep) {
            return Err(format!(
                "cell {cut} depends on #{keep} (through cell {via}): writing #{cut} in cell \
                 {keep} would make them complement each other"
            ));
        }
        Ok((old_card, a, users))
    }

    /// Whether `card`, the card of cell `keep` about to be written, read back
    /// as MCNP reads it, is the region `want` (see [`algebra::same`]).
    fn proves(&self, keep: i64, card: &str, want: &Expr, limits: &Limits) -> Option<bool> {
        let got = card_geometry(card, keep).ok()?;
        let tree = self.tree();
        algebra::same(want, &got, &|n| expandable(&tree, n), limits)
    }

    /// The warnings for the cells that write `#keep`.
    fn user_warnings(keep: i64, cut: i64, users: &[(i64, i64)]) -> Vec<String> {
        users
            .iter()
            .map(|&(n, via)| {
                if via == keep {
                    format!(
                        "cell {n} writes #{keep}: its geometry may change (it may take in \
                         what cell {cut} has of cell {keep})"
                    )
                } else {
                    format!(
                        "cell {n} changes with cell {via} (through #{via} or LIKE {via}): it may \
                         take in what cell {cut} has of cell {keep}"
                    )
                }
            })
            .collect()
    }

    /// Step one of a cut: cell `cut` taken out of cell `keep` by writing
    /// the complement of `cut`'s geometry into `keep`'s, expanded by De
    /// Morgan (cheap at any size). The card is proven equal to `A ∩ ¬B` as
    /// it will be read back, or `#cut` is written as it is. The refusals are
    /// those of [`Source::cut_plan`].
    pub fn cut_expand(&self, keep: i64, cut: i64, limits: &Limits) -> Result<CutExpand, String> {
        let revision = self.revision();
        let (old_card, a, users) = self.cut_checks(keep, cut)?;
        let e = {
            let tree = self.tree();
            algebra::expand_cut(&a, cut, &|n| expandable(&tree, n), limits)?
        };
        let mut plan = CutExpand {
            keep,
            cut,
            old_card: old_card.clone(),
            revision,
            plain_geometry: e.plain_geometry,
            plain_mark: e.plain_mark,
            new_card: old_card.clone(),
            geometry: e.geometry,
            mark: e.mark,
            empty: e.empty,
            plain: e.plain,
            warnings: e.warnings,
            users: users.iter().map(|u| u.0).collect(),
        };
        plan.warnings.extend(Self::user_warnings(keep, cut, &users));
        if plan.empty {
            return self.unchanged_since(revision).map(|()| plan);
        }
        let want = Expr::And(vec![a, Expr::Complement(cut)]);
        plan.new_card = self.cell_with(keep, &[CellChange::Geometry(plan.geometry.clone())])?;
        match self.proves(keep, &plan.new_card, &want, limits) {
            Some(true) => {}
            Some(false) => return Err("internal error: the card written differs from A #B".into()),
            None if plan.plain => {}
            // Not proven: `#cut` as it is (that is `A #B` by definition).
            None => {
                plan.geometry = plan.plain_geometry.clone();
                plan.mark = plan.plain_mark.clone();
                plan.plain = true;
                plan.warnings.insert(
                    0,
                    format!("#{cut} is written as it is: too large to prove equal"),
                );
                plan.new_card =
                    self.cell_with(keep, &[CellChange::Geometry(plan.geometry.clone())])?;
            }
        }
        self.unchanged_since(revision)?;
        if let Some(w) = label_warning(keep, &plan.old_card, &plan.new_card) {
            plan.warnings.push(w);
        }
        Ok(plan)
    }

    /// Step two of a cut: the geometry of cell `keep` as it is in the input
    /// in the fewest terms the Boolean algebra gives, proven equal to it
    /// (the card as it will be read back). The `#n` in it are kept when that
    /// is as short, else expanded (`algebra::shortest`). `limits` stop
    /// the long work; nothing is written (see [`Source::apply_simplified`]).
    pub fn cell_simplify(&self, keep: i64, limits: &Limits) -> Result<Simplified, String> {
        let revision = self.revision();
        // A `#n` expanded without the move a data card gives its cell would
        // be proven equal to the wrong region.
        if let Some(why) = placed_by_data_card(&self.tree(), "simplify") {
            return Err(why);
        }
        let old_card = self
            .card(CardKind::Cell, keep)
            .ok_or_else(|| format!("cell {keep} is not in the input (in a READ file?)"))?
            .text;
        let f = self
            .cell_fields(keep)
            .ok_or_else(|| format!("no cell {keep} in the input"))?;
        if let Some(why) = f.rewrite_refusal(keep, "simplify") {
            return Err(why);
        }
        let (a, s) = {
            let tree = self.tree();
            let a = tree
                .cell(keep)
                .ok_or_else(|| format!("no cell {keep} in the input"))?
                .geometry_expr()
                .map_err(|e| format!("cell {keep}: {}", e.message))?;
            let s = algebra::shortest(&a, None, &|n| expandable(&tree, n), limits)?;
            (a, s)
        };
        let terms_before = algebra::terms(&a);
        let mut r = Simplified {
            keep,
            old_card: old_card.clone(),
            revision,
            new_card: old_card,
            terms_before,
            terms_after: terms_before,
            simplified: false,
            trace: s.trace,
            warnings: s.warnings,
        };
        if s.empty {
            r.warnings
                .push("the geometry is empty: nothing to simplify".into());
            return self.unchanged_since(revision).map(|()| r);
        }
        match s.expr {
            Some(e) if s.terms < terms_before => {
                let card = self.cell_with(keep, &[CellChange::Geometry(e.to_string())])?;
                match self.proves(keep, &card, &a, limits) {
                    Some(true) => {
                        r.new_card = card;
                        r.terms_after = s.terms;
                        r.simplified = true;
                    }
                    Some(false) => {
                        return Err("internal error: the card written differs".into());
                    }
                    None => r
                        .warnings
                        .push("not simplified: the card is too large to prove equal".into()),
                }
            }
            Some(_) => r
                .warnings
                .push("the geometry is as short as the algebra gets".into()),
            None => {}
        }
        self.unchanged_since(revision)?;
        if r.simplified {
            if let Some(w) = label_warning(keep, &r.old_card, &r.new_card) {
                r.warnings.push(w);
            }
        }
        Ok(r)
    }

    /// An error if the input has changed since `revision`: a plan made
    /// from cards read at different times is not one.
    fn unchanged_since(&self, revision: u64) -> Result<(), String> {
        if self.revision() == revision {
            Ok(())
        } else {
            Err("the input changed while the cut was planned: plan it again".into())
        }
    }

    /// Writes the card `new_card` of cell `keep`, planned when the input was
    /// at `revision` (undone with [`Source::restore`]). An error, changing
    /// nothing, if anything in the input has changed since (plan again): the
    /// geometry a plan was proven from is in other cards too.
    pub fn apply_card(&self, keep: i64, revision: u64, new_card: &str) -> Result<Edited, String> {
        self.edit_at(CardKind::Cell, keep, new_card, Some(revision))
    }

    /// Writes an expansion made by [`Source::cut_expand`].
    pub fn apply_expand(&self, plan: &CutExpand) -> Result<Edited, String> {
        if plan.empty {
            return Err(nothing_left(plan.keep, plan.cut));
        }
        self.apply_card(plan.keep, plan.revision, &plan.new_card)
    }

    /// Writes a simplification made by [`Source::cell_simplify`].
    pub fn apply_simplified(&self, s: &Simplified) -> Result<Edited, String> {
        if !s.simplified {
            return Err("nothing shorter to write".into());
        }
        self.apply_card(s.keep, s.revision, &s.new_card)
    }

    /// What cutting cell `cut` out of cell `keep` gives, in one step: its
    /// geometry `A` becomes `A #B`, simplified by Boolean algebra when
    /// `simplify` (`#B` expanded, the other `#n` kept when that is as short;
    /// the card is proven equal to `A ∩ ¬B` from its text, whatever the
    /// surfaces are; the `algebra` module), all within 10 s. The
    /// plain `A #B` is kept on a tie. Nothing is written: [`Source::apply_cut`]
    /// does.
    ///
    /// An error for a cell the input has not got, cells in different
    /// universes (`u=` on their cards), a `LIKE` cell, a `keep` with a `TRCL`
    /// (MCNP moves the surfaces of a `#n` by the cell's own `TRCL`), a `keep`
    /// that a `LIKE` cell copies (that cell would be cut too), a `keep` cut by
    /// `#cut` already, a `cut` that writes `#keep` (directly or through other
    /// cells) and any input with `U` or `TRCL` on a data card. A `cut` with a
    /// `TRCL`, or `#n` of a cell that cannot be expanded, is written as `#n`
    /// and not simplified. A warning for each cell that writes `#keep`: it
    /// then takes in what is cut.
    pub fn cut_plan(&self, keep: i64, cut: i64, simplify: bool) -> Result<CutPlan, String> {
        let revision = self.revision();
        let limits = Limits::within(PLAN_LIMIT);
        let (old_card, a, users) = self.cut_checks(keep, cut)?;
        let mut result = if simplify {
            let tree = self.tree();
            algebra::cut_expr(&a, cut, &|n| expandable(&tree, n), &limits)?
        } else {
            let mut r = algebra::cut_expr(&a, cut, &|_| None, &limits)?;
            // Its "cannot be expanded" is the point of a plain cut.
            r.warnings.clear();
            r.trace.clear();
            r
        };
        result
            .warnings
            .extend(Self::user_warnings(keep, cut, &users));
        let mut new_card = old_card.clone();
        if !result.empty {
            new_card = self.cell_with(keep, &[CellChange::Geometry(result.geometry.clone())])?;
            let want = Expr::And(vec![a, Expr::Complement(cut)]);
            match self.proves(keep, &new_card, &want, &limits) {
                Some(true) => {}
                Some(false) => {
                    return Err("internal error: the card written differs from A #B".into())
                }
                None if !result.simplified => {}
                None => {
                    result.geometry = result.plain.clone();
                    result.terms_after = result.terms_before;
                    result.simplified = false;
                    result
                        .warnings
                        .push("not simplified: the card is too large to prove equal".into());
                    new_card =
                        self.cell_with(keep, &[CellChange::Geometry(result.geometry.clone())])?;
                }
            }
        }
        self.unchanged_since(revision)?;
        if let Some(w) = label_warning(keep, &old_card, &new_card) {
            result.warnings.push(w);
        }
        Ok(CutPlan {
            keep,
            cut,
            old_card,
            revision,
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
            return Err(nothing_left(plan.keep, plan.cut));
        }
        self.apply_card(plan.keep, plan.revision, &plan.new_card)
    }
}

/// The longest [`Source::cut_plan`] may take, simplification and proof
/// together.
const PLAN_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Why a cut of `cut` out of `keep` is not written: nothing is left.
pub fn nothing_left(keep: i64, cut: i64) -> String {
    format!("cell {keep} lies inside cell {cut}: cutting would leave nothing")
}

/// `card` alone in an input of its own, as MCNP reads it.
fn parse_card(card: &str) -> migjorn::Model {
    migjorn::Model::parse(&format!("t\n{}\n\n1 so 1\n\n", card.trim_end()))
}

/// The `$` comments (labels) of the card of cell `keep`.
fn card_labels(card: &str, keep: i64) -> Vec<String> {
    parse_card(card)
        .cell(keep)
        .map(|c| c.comments().into_iter().map(String::from).collect())
        .unwrap_or_default()
}

/// A warning if the card written does not keep the `$` comments of the
/// card it replaces (a label too long for a line of its own becomes comment
/// lines, which no rule `$ label` selects the cell by).
fn label_warning(keep: i64, old_card: &str, new_card: &str) -> Option<String> {
    (card_labels(old_card, keep) != card_labels(new_card, keep)).then(|| {
        format!(
            "a label of cell {keep} does not fit on its card any more: it becomes comment lines"
        )
    })
}

/// The geometry of cell `keep` in `card`, as MCNP reads it (the card alone
/// in an input of its own; a `#n` stays `#n`).
fn card_geometry(card: &str, keep: i64) -> Result<Expr, String> {
    parse_card(card)
        .cell(keep)
        .ok_or_else(|| format!("cell {keep}: the card does not read back"))?
        .geometry_expr()
        .map_err(|e| format!("cell {keep}: {}", e.message))
}

/// Why an input that gives `U` or `TRCL` on a data card is not cut or
/// simplified (`what`): llevant reads them from the cell cards alone, so
/// the checks would miss a universe, and a `#n` would be expanded without
/// its move. A vertical-format card (`#` then the column names) has no
/// name: its columns are read from its line that starts with `#` (comment
/// lines above it are part of its text). A `READ` card anywhere is refused
/// too: the cards it brings in are not read, so they could give `U` or
/// `TRCL`, or write `#n` of the cell changed.
fn placed_by_data_card(tree: &migjorn::Model, what: &str) -> Option<String> {
    if let Some(r) = tree.read_files().first() {
        return Some(format!(
            "the input reads cards from another file (`READ FILE={}`), which is not read: \
             {what} by hand",
            r.path
        ));
    }
    let moves = |n: &str| {
        let n = n.trim_start_matches('*');
        let n = n.split(':').next().unwrap_or(n);
        n.eq_ignore_ascii_case("u") || n.eq_ignore_ascii_case("trcl")
    };
    let name = tree.cell_parameter_cards().find_map(|c| match c.name() {
        Some(n) => moves(n).then(|| n.to_string()),
        None => {
            let head = c
                .text()
                .lines()
                .find_map(|l| l.trim_start().strip_prefix('#'))?;
            let head = head.split('$').next().unwrap_or("");
            head.split_whitespace()
                .find(|w| moves(w))
                .map(|w| format!("# {w}"))
        }
    })?;
    Some(format!(
        "the input gives cell parameters on a `{name}` data card, which is not read: \
         {what} by hand"
    ))
}

/// The first cell, from `from` on through the cells each writes `#n` of,
/// that writes `#target` (`from` itself counts): the cell that would make
/// `target` and `from` complement each other.
fn reaches(tree: &migjorn::Model, from: i64, target: i64) -> Option<i64> {
    let mut seen = vec![from];
    let mut todo = vec![from];
    while let Some(n) = todo.pop() {
        let Some(c) = tree.cell(n) else { continue };
        // A `LIKE m BUT` cell has the geometry of m: it writes what m writes.
        if let Some(m) = c.like() {
            if m == target {
                return Some(n);
            }
            if !seen.contains(&m) {
                seen.push(m);
                todo.push(m);
            }
        }
        for m in complemented(tree, n) {
            if m == target {
                return Some(n);
            }
            if !seen.contains(&m) {
                seen.push(m);
                todo.push(m);
            }
        }
    }
    None
}

/// The cells cell `n` writes `#m` of, as MCNP reads its geometry (a `#` in a
/// `$` comment is none, one split from its number by a comment is one); read
/// from its words if the geometry does not parse.
fn complemented(tree: &migjorn::Model, n: i64) -> Vec<i64> {
    fn walk(e: &Expr, out: &mut Vec<i64>) {
        match e {
            Expr::Surface(_) => {}
            Expr::Complement(m) => {
                if !out.contains(m) {
                    out.push(*m);
                }
            }
            Expr::Not(x) => walk(x, out),
            Expr::And(v) | Expr::Or(v) => v.iter().for_each(|x| walk(x, out)),
        }
    }
    let Some(c) = tree.cell(n) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match c.geometry_expr() {
        Ok(e) => walk(&e, &mut out),
        Err(_) => {
            for w in geometry_words(&c.geometry_text()) {
                if let Word::Cell(m) = w.kind {
                    if !out.contains(&m) {
                        out.push(m);
                    }
                }
            }
        }
    }
    out
}

/// The geometry of cell `n` for a `#n` to expand through: its own, with no
/// `TRCL`, not a `LIKE`.
fn expandable(tree: &migjorn::Model, n: i64) -> Option<Expr> {
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
    fn a_cut_that_would_make_cells_complement_each_other_is_refused() {
        // Direct: cell 2 writes #1.
        let s = Source::from_str("t\n1 0 -1 imp:n=1\n2 0 #1 : -2 imp:n=1\n\n1 so 5\n2 so 9\n\n");
        let err = s.cut_plan(1, 2, true).unwrap_err();
        assert!(err.contains("complement each other"), "{err}");
        assert!(s.cut_expand(1, 2, &Limits::default()).is_err());
        // Indirect: cell 2 writes #3, cell 3 writes #1.
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 #3 -2 imp:n=1\n3 0 #1 -3 imp:n=1\n\n1 so 5\n2 so 9\n3 so 7\n\n",
        );
        assert!(s.cut_plan(1, 2, false).is_err());
        // Cell 2 writes #3, which does not write #1: fine.
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 #3 -2 imp:n=1\n3 0 -3 imp:n=1\n\n1 so 5\n2 so 9\n3 so 7\n\n",
        );
        assert!(s.cut_plan(1, 2, false).is_ok());
        // Through a LIKE: cell 2 writes #3, cell 3 is LIKE 4, cell 4 writes #1.
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 #3 -2 imp:n=1\n3 like 4 but imp:n=2\n4 0 #1 -3 imp:n=1\n\n\
             1 so 5\n2 so 9\n3 so 7\n\n",
        );
        let err = s.cut_plan(1, 2, false).unwrap_err();
        assert!(err.contains("complement each other"), "{err}");
    }

    /// Two inputs loaded alike, each edited once: a plan made from one is
    /// not written into the other.
    #[test]
    fn a_plan_is_bound_to_the_input_it_was_made_from() {
        let a = Source::from_str(INPUT);
        let b = Source::from_str(&INPUT.replace("2 0 -1 3", "2 0 -2 3"));
        assert_ne!(a.revision(), b.revision());
        let plan = a.cut_expand(1, 2, &Limits::default()).unwrap();
        let err = b.apply_expand(&plan).unwrap_err();
        assert!(err.contains("changed since"), "{err}");
        assert!(a.apply_expand(&plan).is_ok());
    }

    #[test]
    fn a_like_cell_cannot_be_cut_out_of_another() {
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 like 3 but imp:n=2\n3 0 -2 imp:n=1\n\n1 so 5\n2 sx 4 5\n\n",
        );
        let err = s.cut_plan(1, 2, true).unwrap_err();
        assert!(err.contains("cell 2 is a LIKE cell"), "{err}");
    }

    #[test]
    fn a_plan_is_refused_when_a_cell_it_was_proven_from_changed() {
        // Cell 2 is edited after the plan: cell 1's card is as it was.
        let s = Source::from_str(INPUT);
        let plan = s.cut_expand(1, 2, &Limits::default()).unwrap();
        let card = s.card(CardKind::Cell, 2).unwrap().text;
        s.edit(CardKind::Cell, 2, &card.replace("-1", "-3"))
            .unwrap();
        let err = s.apply_expand(&plan).unwrap_err();
        assert!(err.contains("changed since"), "{err}");
        assert_eq!(s.card(CardKind::Cell, 1).unwrap().text, plan.old_card);
    }

    #[test]
    fn a_label_that_cannot_stay_a_label_is_warned_of() {
        // 118 characters: still its own `$` line; 125: comment lines.
        for (n, warned) in [(118usize, false), (125, true)] {
            let g = vec!["3"; 40].join(" ");
            let s = Source::from_str(&format!(
                "t\n1 0 -1 {g} imp:n=1 $ {}\n2 0 -2 imp:n=1\n\n1 so 5\n2 sx 4 5\n3 px 0\n\n",
                "x".repeat(n)
            ));
            let p = s.cut_plan(1, 2, false).unwrap();
            let said = p
                .result
                .warnings
                .iter()
                .any(|w| w.contains("a label of cell 1"));
            assert_eq!(said, warned, "{n}: {:?}\n{}", p.result.warnings, p.new_card);
            let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
            assert_eq!(
                e.warnings.iter().any(|w| w.contains("a label of cell 1")),
                warned
            );
        }
    }

    #[test]
    fn a_complement_that_cannot_be_expanded_does_not_stop_the_cut_one() {
        // Cell 3 has a TRCL, so `#3` stays as it is; `#2` is still expanded.
        let s = Source::from_str(
            "t\n1 0 -1 #3 imp:n=1\n2 0 -2 imp:n=1\n3 0 -3 imp:n=1 trcl=(1 0 0)\n\n\
             1 so 5\n2 sx 4 5\n3 so 3\n\n",
        );
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        assert!(!e.plain, "{:?}", e.warnings);
        assert_eq!(e.geometry, "-1 #3 2");
        assert_eq!(e.new_card.trim_end(), "1 0 -1 #3 2 imp:n=1");
        // The cut cell itself with a TRCL: `#2` as it is.
        let s = Source::from_str(
            "t\n1 0 -1 #3 imp:n=1\n2 0 -2 imp:n=1 trcl=(1 0 0)\n3 0 -3 imp:n=1\n\n\
             1 so 5\n2 sx 4 5\n3 so 3\n\n",
        );
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        assert!(e.plain);
        assert_eq!(e.geometry, "-1 #3 #2");
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

    /// A `LIKE m BUT` copy of a cell that writes `#keep` writes it too: it
    /// is one of the cells named, and one the viewer's verdict looks at.
    #[test]
    fn a_like_copy_of_a_cell_writing_the_complement_is_named() {
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 imp:n=1\n3 0 #1 -3 imp:n=1\n5 like 3 but imp:n=2\n\
             6 like 5 but imp:n=1\n4 0 3 imp:n=0\n\n1 so 5\n2 sx 4 5\n3 so 50\n\n",
        );
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        let mut users = e.users.clone();
        users.sort_unstable();
        assert_eq!(users, [3, 5, 6]);
    }

    /// Through other complements: C = #A in a sphere, D = #C, E = #D. When
    /// A shrinks C grows, D shrinks and E grows again: all three are named,
    /// E as changing through D.
    #[test]
    fn cells_that_change_through_other_complements_are_named() {
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 imp:n=1\n3 0 #1 -3 imp:n=1\n4 0 #3 -3 imp:n=1\n\
             5 0 #4 -1 imp:n=1\n9 0 3 imp:n=0\n\n1 so 5\n2 sx 4 5\n3 so 50\n\n",
        );
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        let mut users = e.users.clone();
        users.sort_unstable();
        assert_eq!(users, [3, 4, 5]);
        assert!(
            e.warnings
                .iter()
                .any(|w| w.starts_with("cell 5 changes with cell 4")),
            "{:?}",
            e.warnings
        );
        assert!(e.warnings.iter().any(|w| w.starts_with("cell 3 writes #1")));
    }

    /// A `#n` is read as MCNP reads the geometry: one in a `$` comment is
    /// none (no user, no cycle), one split from its number by a comment is
    /// one.
    #[test]
    fn a_complement_in_a_comment_is_not_one() {
        // Cell 3 only mentions #1 in a comment.
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 imp:n=1 $ was #1\n3 0 1 -3 imp:n=1 $ see #1\n\
             9 0 3 imp:n=0\n\n1 so 5\n2 sx 4 5\n3 so 50\n\n",
        );
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        assert!(e.users.is_empty(), "{:?}", e.users);
        // Cell 2 writes `#` then a comment, and `1` on the next line: `#1`.
        let s = Source::from_str(
            "t\n1 0 -1 imp:n=1\n2 0 -2 # $ note\n     1 imp:n=1\n9 0 2 imp:n=0\n\n\
             1 so 5\n2 sx 4 5\n\n",
        );
        let err = s.cut_expand(1, 2, &Limits::default()).unwrap_err();
        assert!(err.contains("complement each other"), "{err}");
    }

    /// Universes are compared as numbers: `U=01` is universe 1.
    #[test]
    fn universes_are_compared_as_numbers() {
        let s = Source::from_str(
            "t\n1 0 -1 u=01 imp:n=1\n2 0 -2 u=1 imp:n=1\n3 0 -3 fill=1 imp:n=1\n\
             9 0 3 imp:n=0\n\n1 so 5\n2 sx 4 5\n3 so 50\n\n",
        );
        assert!(s.cut_expand(1, 2, &Limits::default()).is_ok());
    }

    /// `U` (or `TRCL`) on a data card is not read: a cut, whose checks read
    /// the cell cards, is refused rather than made across universes.
    #[test]
    fn a_cut_is_refused_when_cells_are_placed_by_a_data_card() {
        let deck = "t\n1 0 -1\n2 0 -2\n3 0 1 2 -3\n4 0 3\n\n1 so 5\n2 sx 4 5\n3 so 50\n\n\
                    imp:n 1 1 1 0\n";
        assert!(
            Source::from_str(deck).cut_plan(1, 2, false).is_ok(),
            "IMP is fine"
        );
        let err = Source::from_str(&format!("{deck}u 1 0 0 0\n"))
            .cut_plan(1, 2, false)
            .unwrap_err();
        assert!(err.contains("data card"), "{err}");
        // In vertical format: a `TRCL` column moves cell 1.
        let vertical = format!("{deck}#  trcl  imp:p\n1  (5 0 0)  1\n2  j  1\n3  j  1\n4  j  0\n");
        let err = Source::from_str(&vertical)
            .cut_plan(1, 2, false)
            .unwrap_err();
        assert!(err.contains("data card"), "{err}");
        // An `IMP` column alone is fine.
        let imp = format!("{deck}#  imp:p\n1  1\n2  1\n3  1\n4  0\n");
        assert!(Source::from_str(&imp).cut_plan(1, 2, false).is_ok());
        // The same with a comment line above the `#` line (part of the
        // card's text).
        let vertical = format!(
            "{deck}c placement\n#  trcl  imp:p\n1  (5 0 0)  1\n2  j  1\n3  j  1\n4  j  0\n"
        );
        let err = Source::from_str(&vertical)
            .cut_plan(1, 2, false)
            .unwrap_err();
        assert!(err.contains("data card"), "{err}");
        let imp = format!("{deck}c placement\n#  imp:p\n1  1\n2  1\n3  1\n4  0\n");
        assert!(Source::from_str(&imp).cut_plan(1, 2, false).is_ok());
        // A `READ` card in the data block could bring in `U 1 0 0 0`.
        let err = Source::from_str(&format!("{deck}read file=placement.i\n"))
            .cut_plan(1, 2, false)
            .unwrap_err();
        assert!(err.contains("READ FILE=placement.i"), "{err}");
    }

    /// Simplify refuses it too: cell 2, moved by a data card, would be
    /// expanded where it is not, and `-1 #2` written as `-1`.
    #[test]
    fn a_simplification_is_refused_when_cells_are_placed_by_a_data_card() {
        let deck = "t\n1 0 -1 #2\n2 0 1 -2\n3 0 2\n\n1 px 0\n2 so 50\n\n";
        let l = Limits::default();
        assert!(Source::from_str(deck).cell_simplify(1, &l).is_ok());
        let moved = format!("{deck}#  trcl\n1  j\n2  (-10 0 0)\n3  j\n");
        let err = Source::from_str(&moved).cell_simplify(1, &l).unwrap_err();
        assert!(err.contains("data card"), "{err}");
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
    fn a_cell_cut_already_is_refused_by_both_steps() {
        let s = Source::from_str("t\n1 0 -1 #2 imp:n=1\n2 0 -2 imp:n=1\n\n1 so 5\n2 sx 4 5\n\n");
        for simplify in [false, true] {
            let err = s.cut_plan(1, 2, simplify).unwrap_err();
            assert!(err.contains("already"), "{err}");
        }
        let err = s.cut_expand(1, 2, &Limits::default()).unwrap_err();
        assert!(err.contains("already"), "{err}");
    }

    #[test]
    fn the_card_written_is_proven_from_its_text() {
        let s = Source::from_str(INPUT);
        let want = Expr::And(vec![
            card_geometry(&s.card(CardKind::Cell, 1).unwrap().text, 1).unwrap(),
            Expr::Complement(2),
        ]);
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        assert_eq!(
            s.proves(1, &e.new_card, &want, &Limits::default()),
            Some(true)
        );
        // A card that reads otherwise (a union where the text lost its
        // parentheses) is told apart.
        let wrong = "1 0 -1:-2 3 1:-3 imp:n=1\n";
        assert_eq!(s.proves(1, wrong, &want, &Limits::default()), Some(false));
    }

    #[test]
    fn a_long_cut_fits_mcnp_columns() {
        // B of 40 five-digit planes: its complement is a union of 40 terms,
        // which migjorn prints without a space; A has a long comment.
        let planes: Vec<String> = (0..40).map(|k| format!("-{}", 10001 + k)).collect();
        let rows: Vec<String> = planes.chunks(10).map(|c| c.join(" ")).collect();
        let mut deck = format!(
            "t\n1 0 -1 imp:n=1 $ the cell that the other one overlaps, by far\n\
             2 0 {} imp:n=1\n\n1 so 500\n",
            rows.join("\n      ")
        );
        for k in 0..40 {
            deck += &format!("{} px {k}\n", 10001 + k);
        }
        deck += "\n";
        let fits = |card: &str| {
            card.lines()
                .all(|l| l.chars().count() <= crate::source::MAX_COLUMNS)
        };
        let s = Source::from_str(&deck);
        let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
        assert!(!e.plain, "{:?}", e.warnings);
        assert!(fits(&e.new_card), "{}", e.new_card);
        assert!(e.new_card.contains("by far"), "{}", e.new_card);
        s.apply_expand(&e).unwrap();
        let r = s.cell_simplify(1, &Limits::default()).unwrap();
        assert!(fits(&r.new_card), "{}", r.new_card);
    }

    #[test]
    fn simplify_keeps_the_other_complements() {
        // Cell 1 writes #5 and has the expansion of a cut in it: the
        // shorter form keeps #5 (it still follows cell 5).
        let s = Source::from_str(
            "t\n1 0 (-1:-2) 3 (1:-3) #5 imp:n=1\n2 0 -1 3 imp:n=1\n5 0 -4 -6 7 imp:n=1\n\n\
             1 so 5\n2 so 9\n3 px 0\n4 so 2\n6 py 0\n7 pz 0\n\n",
        );
        let r = s.cell_simplify(1, &Limits::default()).unwrap();
        assert!(r.simplified, "{:?}", r.warnings);
        assert!(r.new_card.contains("#5"), "{}", r.new_card);
        // -2 3 1 #5: the expanded #5 would be three terms more.
        assert_eq!(r.terms_after, 4, "{}", r.new_card);
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
        assert_eq!(e.users, vec![3]);
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
        // Restored, the expanded card is back, and simplifies again.
        let again = s.cell_simplify(1, &Limits::default()).unwrap();
        assert!(again.simplified);
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
