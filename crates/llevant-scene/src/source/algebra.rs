// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! The Boolean algebra of cell geometries: a cell cut out of another
//! (`A ∩ #B`, [`expand_cut`], [`cut_expr`]) and a geometry in fewer terms
//! ([`shortest`], [`simplify_bool`]), proven equal ([`same`]).
//!
//! Only the algebra is used. Every `+s`/`-s` of a surface is a variable and
//! its negation (a macrobody facet `470.2` is a variable of its own, never
//! tied to the whole surface `470`), a `#n` kept is a variable too, and
//! nothing is assumed about where the surfaces are. Each step is an identity
//! valid for every value of the variables, so the result is the same region
//! in any geometry. A form is given only when reduced ordered BDDs
//! (canonical: the same function gets the same node) prove it equal, so the
//! guarantee does not rest on the steps being coded right; `Source` proves
//! the card again from its text.
//!
//! The steps of [`simplify_bool`]: negation normal form; the first tier
//! ([`tree`]: a literal known from the products and sums around it is a
//! constant below, `x·¬x = 0`, absorption; one visit of the expression);
//! then, for what has at most 256 products and within the [`Limits`], the
//! second tier on the expression and on its complement negated back (a union
//! of products and an intersection of unions): disjunctive normal form, all
//! prime implicants by iterated consensus, an irredundant cover, factoring.
//! The shortest proven form wins. [`shortest`] runs it with the other `#n`
//! kept and with them expanded. Why, and the measurements: ADR 0016.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

mod tree;

use migjorn::{Expr, SurfaceRef};

/// Most products the disjunctive normal form may reach.
const MAX_CUBES: usize = 4096;
/// As many, for the second tier of [`simplify_bool`] (it costs more than
/// the square of them): more than that is not tried.
const TIER2_CUBES: usize = 256;
/// Most cells a `#n` may be expanded through, one inside the other.
const MAX_DEPTH: usize = 64;
/// Most nodes of an expression after expanding the complements.
const MAX_NODES: usize = 100_000;
/// Most nodes of a BDD.
const MAX_BDD: usize = 2_000_000;
/// Most steps of one redundancy test.
const MAX_TAUTOLOGY: usize = 200_000;

/// What a variable of the algebra stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Atom {
    /// A surface, or a facet of a macrobody.
    Surface { id: i64, facet: Option<u8> },
    /// Outside cell `n` (`#n`), when `#n` is not expanded.
    Cell(i64),
}

/// An expression of the algebra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bool {
    True,
    False,
    /// The atom itself (`true`: positive sense, or `#n`) or its negation.
    Lit(Atom, bool),
    Not(Box<Bool>),
    And(Vec<Bool>),
    Or(Vec<Bool>),
}

/// A cut in one step, as [`CutPlan::result`](super::CutPlan::result) holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cut {
    /// The geometry of the cut cell, to write.
    pub geometry: String,
    /// The plain `A #B`, as [`Source::cell_cut`](super::Source::cell_cut) gives.
    pub plain: String,
    /// Terms (surfaces and `#n`) of `plain` and of `geometry`.
    pub terms_before: usize,
    pub terms_after: usize,
    /// The geometry is not the plain one: the algebra gave a shorter form.
    pub simplified: bool,
    /// `A ∩ ¬B` is empty (A is inside B): nothing to write.
    pub empty: bool,
    /// The laws applied, with how many times.
    pub trace: Vec<String>,
    /// Why a simplification was not done, or anything else to look at.
    pub warnings: Vec<String>,
}

/// How long the work may take, and who may stop it.
#[derive(Clone, Debug, Default)]
pub struct Limits {
    pub until: Option<Instant>,
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Limits {
    /// Until `d` from now.
    pub fn within(d: Duration) -> Self {
        Limits {
            until: Some(Instant::now() + d),
            cancel: None,
        }
    }

    /// Time is up, or it was cancelled.
    pub fn over(&self) -> bool {
        self.until.is_some_and(|t| Instant::now() >= t)
            || self
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

/// Counts of the laws applied.
struct Stats {
    de_morgan: usize,
    distributed: usize,
    contradictions: usize,
    absorbed: usize,
    propagated: usize,
    consensus: usize,
    redundant: usize,
    factored: usize,
    limits: Limits,
    /// Most products of the disjunctive normal form.
    max_cubes: usize,
    /// The limits stopped some work.
    stopped: bool,
}

impl Stats {
    fn new(limits: &Limits, max_cubes: usize) -> Self {
        Stats {
            de_morgan: 0,
            distributed: 0,
            contradictions: 0,
            absorbed: 0,
            propagated: 0,
            consensus: 0,
            redundant: 0,
            factored: 0,
            limits: limits.clone(),
            max_cubes,
            stopped: false,
        }
    }

    /// Time is up (and the work stops, with what it has).
    fn over(&mut self) -> bool {
        if self.limits.over() {
            self.stopped = true;
        }
        self.stopped
    }

    fn lines(&self) -> Vec<String> {
        [
            ("De Morgan", self.de_morgan),
            ("distributivity", self.distributed),
            ("x·¬x = 0", self.contradictions),
            ("a known literal replaced by 0 or 1", self.propagated),
            ("idempotence and absorption", self.absorbed),
            ("consensus", self.consensus),
            ("redundant term removed", self.redundant),
            ("factoring", self.factored),
        ]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(name, n)| format!("{name}: {n}"))
        .collect()
    }
}

fn atom_of(s: &SurfaceRef) -> Atom {
    Atom::Surface {
        id: s.id,
        facet: s.facet,
    }
}

/// What [`simplify_bool`] gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Simp {
    /// The shortest form proven equal to the original (`None`: none).
    pub expr: Option<Expr>,
    /// Terms of `expr`.
    pub terms: usize,
    /// The function is empty: no point is in the cell.
    pub empty: bool,
    /// The laws applied, with how many times.
    pub trace: Vec<String>,
    /// Why something was not done, or anything else to look at.
    pub warnings: Vec<String>,
}

/// The shortest form of `reference` that the laws of Boolean algebra give,
/// in two tiers: [`tree`] (about one visit of the expression, for any size),
/// then, only for what is small enough and within `limits`, disjunctive
/// normal form, all prime implicants, an irredundant cover and factoring,
/// of the expression and of its complement (negated back: an intersection
/// of unions).
/// A form is given only when it is proven equal to `reference` (reduced
/// ordered BDDs, so it does not rest on the laws being coded right); when
/// the proof is too large, or the limits stop it, there is no form and
/// `warnings` says why.
pub fn simplify_bool(reference: &Bool, limits: &Limits) -> Result<Simp, String> {
    let mut out = Simp {
        expr: None,
        terms: 0,
        empty: false,
        trace: Vec::new(),
        warnings: Vec::new(),
    };
    let mut stats = Stats::new(limits, MAX_CUBES);
    let normal = nnf(reference, false, &mut stats);
    let mut bdd = Bdd::new(limits);
    let Some(want) = bdd.build(reference) else {
        out.warnings
            .push("not simplified: too large to prove equal".into());
        return Ok(out);
    };

    // Tier 1.
    let t1 = tree::simplify(&normal, &mut stats);
    if t1 == Bool::False {
        if want != FALSE {
            return Err("internal error: an empty form is not proven empty".into());
        }
        out.empty = true;
        out.trace = stats.lines();
        return Ok(out);
    }
    let mut forms: Vec<Bool> = vec![t1.clone()];

    // Tier 2, from the first tier's result, as a union of products.
    let mut tier2 = Stats::new(limits, TIER2_CUBES);
    forms.extend(second_tier(&t1, &mut tier2));
    // And its dual: the complement as a union of products, negated back, so
    // an intersection of unions (MCNP's usual shape). A cell of m groups
    // `(a:b)` has 2^m products, its complement m: what explodes one way is
    // small the other, each under its own cap.
    let mut dual = Stats::new(limits, TIER2_CUBES);
    let complement = tree::simplify(&nnf(&t1, true, &mut dual), &mut dual);
    for g in second_tier(&complement, &mut dual) {
        forms.push(tree::simplify(&nnf(&g, true, &mut dual), &mut dual));
    }
    if tier2.stopped || dual.stopped {
        out.warnings
            .push("the time limit stopped the second tier of simplification".into());
    }
    for s in [&tier2, &dual] {
        stats.consensus += s.consensus;
        stats.redundant += s.redundant;
        stats.factored += s.factored;
        stats.distributed += s.distributed;
        stats.absorbed += s.absorbed;
        stats.contradictions += s.contradictions;
    }

    // The shortest first; the first one proven equal is the answer.
    let mut candidates: Vec<(Expr, usize)> = forms
        .iter()
        .filter_map(to_expr)
        .map(|e| {
            let n = terms(&e);
            (e, n)
        })
        .collect();
    candidates.sort_by_key(|c| c.1);
    // The shortest is the answer if it is proven equal; one that is not is a
    // mistake of the laws above (never given).
    if let Some((e, n)) = candidates.into_iter().next() {
        // The proof, on the form as MCNP will write it (the card made of it
        // is proven again from its text: `Source::cell_simplify`).
        let mut nodes = 0;
        let back = to_bool(&e, None, &mut Vec::new(), &mut nodes)
            .map_err(|why| format!("internal error: {why}"))?;
        return match bdd.build(&back) {
            Some(f) if f == want => {
                out.expr = Some(e);
                out.terms = n;
                out.trace = stats.lines();
                Ok(out)
            }
            Some(_) => Err("internal error: the simplified form differs".into()),
            None => {
                out.warnings
                    .push("not simplified: too large to prove equal".into());
                Ok(out)
            }
        };
    }
    out.warnings.push(if t1 == Bool::True {
        "not simplified: the result has no restriction left".into()
    } else {
        "not simplified: no shorter form can be written without the geometry of a cell \
         complemented"
            .into()
    });
    Ok(out)
}

/// The second tier on `b` (in negation normal form): its prime implicants,
/// an irredundant cover of them, factored and as a union of products; none
/// when its disjunctive normal form is over the cap of `stats`, or `b` is a
/// constant.
fn second_tier(b: &Bool, stats: &mut Stats) -> Vec<Bool> {
    match dnf(b, stats) {
        Some(mut cubes) if !cubes.is_empty() && cubes.iter().all(|c| !c.is_empty()) => {
            primes(&mut cubes, stats);
            irredundant(&mut cubes, stats);
            vec![
                factor(&cubes, stats),
                Bool::Or(cubes.iter().map(cube_bool).collect()),
            ]
        }
        _ => Vec::new(),
    }
}

/// Whether `got` is the same region as `want`, whatever the surfaces are:
/// first with every `#n` a variable of its own (equal so, equal for any
/// cell n), then with every `#n` expanded through `cells`. `None` when that
/// cannot be told (the proof is too large, or a `#n` cannot be expanded).
pub fn same(
    want: &Expr,
    got: &Expr,
    cells: &dyn Fn(i64) -> Option<Expr>,
    limits: &Limits,
) -> Option<bool> {
    let proof = |cells: Option<&dyn Fn(i64) -> Option<Expr>>| -> Option<bool> {
        if limits.over() {
            return None;
        }
        let (mut n, mut m) = (0, 0);
        let w = to_bool(want, cells, &mut Vec::new(), &mut n).ok()?;
        let g = to_bool(got, cells, &mut Vec::new(), &mut m).ok()?;
        let mut bdd = Bdd::new(limits);
        Some(bdd.build(&w)? == bdd.build(&g)?)
    };
    match proof(None) {
        Some(true) => Some(true),
        _ => proof(Some(cells)),
    }
}

/// `a` is cut by `#b` already: `#b` is one of the terms intersected.
pub fn already_cut(a: &Expr, b: i64) -> bool {
    plain(a, b) == *a
}

/// `e` with every `#b` in it replaced by `with`.
fn substitute(e: &Expr, b: i64, with: &Expr) -> Expr {
    match e {
        Expr::Complement(n) if *n == b => with.clone(),
        Expr::Surface(_) | Expr::Complement(_) => e.clone(),
        Expr::Not(x) => Expr::Not(Box::new(substitute(x, b, with))),
        Expr::And(v) => Expr::And(v.iter().map(|x| substitute(x, b, with)).collect()),
        Expr::Or(v) => Expr::Or(v.iter().map(|x| substitute(x, b, with)).collect()),
    }
}

fn has_complement(e: &Expr) -> bool {
    match e {
        Expr::Surface(_) => false,
        Expr::Complement(_) => true,
        Expr::Not(x) => has_complement(x),
        Expr::And(v) | Expr::Or(v) => v.iter().any(has_complement),
    }
}

/// The shortest form of `e` that [`simplify_bool`] proves equal, tried two
/// ways: with every `#n` kept as written (a variable of its own: the form
/// keeps following those cells), except `#open`, which is expanded; and
/// with every `#n` expanded through `cells`. The first wins a tie. `Err`
/// only for an internal error.
pub fn shortest(
    e: &Expr,
    open: Option<i64>,
    cells: &dyn Fn(i64) -> Option<Expr>,
    limits: &Limits,
) -> Result<Simp, String> {
    let none = |why: String| Simp {
        expr: None,
        terms: 0,
        empty: false,
        trace: Vec::new(),
        warnings: vec![format!("not simplified: {why}")],
    };
    let kept = match open {
        Some(b) => match cells(b) {
            Some(g) => substitute(e, b, &Expr::Not(Box::new(g))),
            None => {
                return Ok(none(format!(
                    "cell {b} cannot be expanded (TRCL, LIKE or absent)"
                )))
            }
        },
        None => e.clone(),
    };
    let mut nodes = 0;
    let first = match to_bool(&kept, None, &mut Vec::new(), &mut nodes) {
        Ok(r) => simplify_bool(&r, limits)?,
        Err(why) => none(why),
    };
    if first.empty || !has_complement(&kept) {
        return Ok(first);
    }
    let mut nodes = 0;
    let second = match to_bool(&kept, Some(cells), &mut Vec::new(), &mut nodes) {
        Ok(r) => simplify_bool(&r, limits)?,
        Err(why) => none(why),
    };
    Ok(match (&first.expr, &second.expr) {
        _ if second.empty => second,
        (Some(_), Some(_)) if second.terms < first.terms => second,
        (Some(_), _) => first,
        (None, Some(_)) => second,
        // Neither: why the full expansion failed says the most.
        (None, None) => {
            let mut s = second;
            s.warnings.extend(first.warnings);
            s.warnings.dedup();
            s
        }
    })
}

/// Cell `b` cut out of the cell whose geometry is `a`: `a ∩ #b`.
///
/// `cells` gives the geometry of any cell `#n` may name (including `b`), or
/// `None` for one that cannot be expanded (it has a `TRCL`, is a `LIKE` cell,
/// or does not exist). Then nothing is expanded, and the plain `A #B` is
/// given with the reason in `warnings`.
pub fn cut_expr(a: &Expr, b: i64, cells: &dyn Fn(i64) -> Option<Expr>) -> Result<Cut, String> {
    cut_expr_within(a, b, cells, &Limits::within(Duration::from_secs(10)))
}

/// As [`cut_expr`], stopped by `limits`.
pub fn cut_expr_within(
    a: &Expr,
    b: i64,
    cells: &dyn Fn(i64) -> Option<Expr>,
    limits: &Limits,
) -> Result<Cut, String> {
    let plain_expr = plain(a, b);
    let terms_before = terms(&plain_expr);
    let mut cut = Cut {
        geometry: plain_expr.to_string(),
        plain: plain_expr.to_string(),
        terms_before,
        terms_after: terms_before,
        simplified: false,
        empty: false,
        trace: Vec::new(),
        warnings: Vec::new(),
    };

    // `A ∩ ¬B`, `#B` expanded, the other `#n` kept if that is no longer.
    let both = Expr::And(vec![a.clone(), Expr::Complement(b)]);
    let s = shortest(&both, Some(b), cells, limits)?;
    cut.trace = s.trace;
    cut.warnings = s.warnings;
    cut.empty = s.empty;
    if let Some(e) = s.expr {
        if s.terms < terms_before {
            cut.geometry = e.to_string();
            cut.terms_after = s.terms;
            cut.simplified = true;
        } else {
            cut.warnings
                .push("the plain A #B is as short as the algebra gets".into());
        }
    }
    Ok(cut)
}

/// What cutting cell `b` out of cell `a` writes at first: `a` with `#b`
/// replaced by what it stands for, the complement of cell `b` (De Morgan,
/// and nothing else, so it is cheap at any size). `mark` is the range of
/// `geometry` that is new.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expanded {
    /// The geometry with `#b` as it is, and the range of it that is `#b`.
    pub plain_geometry: String,
    pub plain_mark: std::ops::Range<usize>,
    pub geometry: String,
    pub mark: std::ops::Range<usize>,
    /// `A` is inside `B`: nothing is left.
    pub empty: bool,
    /// `#b` could not be expanded (a `TRCL`, a `LIKE` or too large), so it
    /// is written as `#b`, and `warnings` says why.
    pub plain: bool,
    pub warnings: Vec<String>,
}

/// `a` ∩ ¬`b`, with `#b` expanded through `cells`, and whether nothing is
/// left. [`Source::cut_expand`](super::Source::cut_expand) proves the card
/// written from its text.
pub fn expand_cut(
    a: &Expr,
    b: i64,
    cells: &dyn Fn(i64) -> Option<Expr>,
    limits: &Limits,
) -> Result<Expanded, String> {
    let items = |e: &Expr| -> Vec<Expr> {
        match e {
            Expr::And(v) => v.clone(),
            other => vec![other.clone()],
        }
    };
    let join = |kept: &Expr, added: &Expr| -> Expanded {
        let k = Expr::And(items(kept)).to_string();
        let n = Expr::And(items(added)).to_string();
        // What is added starts after `k` and a space, in both.
        let start = k.len() + 1;
        let t = Expr::Complement(b).to_string();
        Expanded {
            plain_geometry: format!("{k} {t}"),
            plain_mark: start..start + t.len(),
            mark: start..start + n.len(),
            geometry: format!("{k} {n}"),
            empty: false,
            plain: false,
            warnings: Vec::new(),
        }
    };
    let mut nodes = 0;
    let both = Expr::And(vec![a.clone(), Expr::Complement(b)]);
    let reference = to_bool(&both, Some(cells), &mut Vec::new(), &mut nodes);
    let plain_form = |why: String| {
        let mut e = join(a, &Expr::Complement(b));
        e.plain = true;
        e.warnings.push(format!("#{b} is written as it is: {why}"));
        e
    };
    let reference = match reference {
        Ok(r) => r,
        Err(why) => return Ok(plain_form(why)),
    };
    let mut stats = Stats::new(limits, MAX_CUBES);
    // `#b` alone, expanded and with the negation pushed in.
    let mut nodes = 0;
    let inside = to_bool(
        &Expr::Complement(b),
        Some(cells),
        &mut Vec::new(),
        &mut nodes,
    )
    .map_err(|why| format!("internal error: {why}"))?;
    let neg = nnf(&inside, false, &mut stats);
    let Some(added) = to_expr(&neg) else {
        return Ok(plain_form("nothing to write".into()));
    };
    let mut out = join(a, &added);
    // Nothing left? (The text written is proven equal to `a ∩ ¬b` where it
    // is written, read back from the card: `Source::cut_expand`.)
    let mut bdd = Bdd::new(limits);
    match bdd.build(&reference) {
        Some(w) => out.empty = w == FALSE,
        None => return Ok(plain_form("too large to prove equal".into())),
    }
    Ok(out)
}

/// `a #b`: `#b` added to the intersection (nothing added if it is there).
fn plain(a: &Expr, b: i64) -> Expr {
    let c = Expr::Complement(b);
    match a {
        Expr::And(items) if items.contains(&c) => a.clone(),
        Expr::And(items) => {
            let mut v = items.clone();
            v.push(c);
            Expr::And(v)
        }
        _ if *a == c => a.clone(),
        _ => Expr::And(vec![a.clone(), c]),
    }
}

/// Terms (surfaces and complements) of an expression.
pub fn terms(e: &Expr) -> usize {
    match e {
        Expr::Surface(_) | Expr::Complement(_) => 1,
        Expr::Not(x) => terms(x),
        Expr::And(v) | Expr::Or(v) => v.iter().map(terms).sum(),
    }
}

/// The expression as algebra. With `cells`, a `#n` becomes the negation of
/// cell n's geometry (an `Err` if it cannot); without, it is an atom.
pub fn to_bool(
    e: &Expr,
    cells: Option<&dyn Fn(i64) -> Option<Expr>>,
    stack: &mut Vec<i64>,
    nodes: &mut usize,
) -> Result<Bool, String> {
    *nodes += 1;
    if *nodes > MAX_NODES {
        return Err("the cells complemented are too large".into());
    }
    Ok(match e {
        Expr::Surface(s) => Bool::Lit(atom_of(s), !s.negative),
        Expr::Complement(n) => match cells {
            None => Bool::Lit(Atom::Cell(*n), true),
            Some(cells) => {
                if stack.contains(n) {
                    return Err(format!("cell {n} complements itself"));
                }
                if stack.len() >= MAX_DEPTH {
                    return Err("the complements are nested too deep".into());
                }
                let inner = cells(*n)
                    .ok_or_else(|| format!("cell {n} cannot be expanded (TRCL, LIKE or absent)"))?;
                stack.push(*n);
                let b = to_bool(&inner, Some(cells), stack, nodes)?;
                stack.pop();
                Bool::Not(Box::new(b))
            }
        },
        Expr::Not(x) => Bool::Not(Box::new(to_bool(x, cells, stack, nodes)?)),
        Expr::And(v) => Bool::And(
            v.iter()
                .map(|x| to_bool(x, cells, stack, nodes))
                .collect::<Result<_, _>>()?,
        ),
        Expr::Or(v) => Bool::Or(
            v.iter()
                .map(|x| to_bool(x, cells, stack, nodes))
                .collect::<Result<_, _>>()?,
        ),
    })
}

/// The expression as MCNP writes it, if it can be: `None` for a form with a
/// negated `#n` (there is no way to write "inside cell n" without its geometry)
/// or with nothing in it.
pub fn to_expr(b: &Bool) -> Option<Expr> {
    fn many(v: &[Bool], and: bool) -> Option<Expr> {
        let mut out = Vec::new();
        for x in v {
            match to_expr(x)? {
                Expr::And(inner) if and => out.extend(inner),
                Expr::Or(inner) if !and => out.extend(inner),
                e => out.push(e),
            }
        }
        match out.len() {
            0 => None,
            1 => out.pop(),
            _ if and => Some(Expr::And(out)),
            _ => Some(Expr::Or(out)),
        }
    }
    match b {
        Bool::True | Bool::False => None,
        Bool::Lit(Atom::Surface { id, facet }, pos) => Some(Expr::Surface(SurfaceRef {
            id: *id,
            facet: *facet,
            negative: !pos,
        })),
        Bool::Lit(Atom::Cell(n), true) => Some(Expr::Complement(*n)),
        Bool::Lit(Atom::Cell(_), false) => None,
        Bool::Not(x) => Some(Expr::Not(Box::new(to_expr(x)?))),
        Bool::And(v) => many(v, true),
        Bool::Or(v) => many(v, false),
    }
}

/// Negations pushed to the atoms (De Morgan, double negation).
fn nnf(b: &Bool, neg: bool, stats: &mut Stats) -> Bool {
    match b {
        Bool::True => {
            if neg {
                Bool::False
            } else {
                Bool::True
            }
        }
        Bool::False => {
            if neg {
                Bool::True
            } else {
                Bool::False
            }
        }
        Bool::Lit(a, p) => Bool::Lit(*a, *p != neg),
        Bool::Not(x) => nnf(x, !neg, stats),
        Bool::And(v) | Bool::Or(v) => {
            let and = matches!(b, Bool::And(_)) != neg;
            if neg {
                stats.de_morgan += 1;
            }
            let items = v.iter().map(|x| nnf(x, neg, stats)).collect();
            if and {
                Bool::And(items)
            } else {
                Bool::Or(items)
            }
        }
    }
}

/// A product of literals: atom to sense. The empty cube is "true".
type Cube = BTreeMap<Atom, bool>;

fn cube_bool(c: &Cube) -> Bool {
    Bool::And(c.iter().map(|(a, p)| Bool::Lit(*a, *p)).collect())
}

/// `a` implies `b` as products: every literal of `b` is in `a`.
fn contains(a: &Cube, b: &Cube) -> bool {
    b.iter().all(|(k, v)| a.get(k) == Some(v))
}

/// Idempotence and absorption (`x + xy = x`): no cube kept holds another.
/// `false` if the limits stopped it (the cubes are then left as they were).
fn absorb(cubes: &mut Vec<Cube>, stats: &mut Stats) -> bool {
    cubes.sort_by_key(|c| c.len());
    let before = cubes.len();
    let mut kept: Vec<Cube> = Vec::new();
    for (i, c) in cubes.iter().enumerate() {
        if i % 64 == 0 && stats.over() {
            return false;
        }
        if !kept.iter().any(|k| contains(c, k)) {
            kept.push(c.clone());
        }
    }
    stats.absorbed += before - kept.len();
    *cubes = kept;
    true
}

/// How many times `max_cubes` the products of one step may be before they
/// are absorbed.
const RAW_FACTOR: usize = 4;

/// Disjunctive normal form of an expression in negation normal form.
/// `None` when it passes `stats.max_cubes` products, or the limits.
fn dnf(b: &Bool, stats: &mut Stats) -> Option<Vec<Cube>> {
    let mut cubes = match b {
        Bool::True => vec![Cube::new()],
        Bool::False => Vec::new(),
        Bool::Lit(a, p) => vec![Cube::from([(*a, *p)])],
        Bool::Not(_) => unreachable!("negations are at the atoms"),
        Bool::Or(v) => {
            let mut all = Vec::new();
            for x in v {
                all.extend(dnf(x, stats)?);
                if all.len() > stats.max_cubes {
                    return None;
                }
            }
            all
        }
        Bool::And(v) => {
            let mut acc = vec![Cube::new()];
            for x in v {
                if stats.over() {
                    return None;
                }
                let part = dnf(x, stats)?;
                let mut next = Vec::new();
                for c in &acc {
                    for d in &part {
                        match merge(c, d) {
                            Some(m) => next.push(m),
                            None => stats.contradictions += 1,
                        }
                    }
                    // The products before absorbing may pass the cap (they
                    // shrink), but not by so much that absorbing them is
                    // the work the cap is there to stop.
                    if next.len() > stats.max_cubes * RAW_FACTOR || stats.over() {
                        return None;
                    }
                }
                if acc.len() > 1 || part.len() > 1 {
                    stats.distributed += 1;
                }
                if !absorb(&mut next, stats) {
                    return None;
                }
                if next.len() > stats.max_cubes {
                    return None;
                }
                acc = next;
            }
            acc
        }
    };
    if !absorb(&mut cubes, stats) {
        return None;
    }
    Some(cubes)
}

/// The product of two cubes, `None` if they hold a literal and its negation.
fn merge(a: &Cube, b: &Cube) -> Option<Cube> {
    let mut m = a.clone();
    for (k, v) in b {
        if *m.entry(*k).or_insert(*v) != *v {
            return None;
        }
    }
    Some(m)
}

/// The consensus of two cubes: they differ in the sense of exactly one atom.
fn consensus(a: &Cube, b: &Cube) -> Option<Cube> {
    let mut opposed = None;
    for (k, v) in a {
        if b.get(k).is_some_and(|w| w != v) {
            if opposed.is_some() {
                return None;
            }
            opposed = Some(*k);
        }
    }
    let k = opposed?;
    let mut m = a.clone();
    m.remove(&k);
    for (j, v) in b {
        if *j != k && *m.entry(*j).or_insert(*v) != *v {
            return None;
        }
    }
    Some(m)
}

/// All prime implicants, by iterated consensus and absorption. Stops, with
/// the cubes it has (still the same function), if there get to be too many.
fn primes(cubes: &mut Vec<Cube>, stats: &mut Stats) {
    let most = stats.max_cubes / 4;
    loop {
        let mut added = Vec::new();
        'pairs: for i in 0..cubes.len() {
            if stats.over() {
                return;
            }
            for j in i + 1..cubes.len() {
                if let Some(c) = consensus(&cubes[i], &cubes[j]) {
                    if !cubes.iter().chain(&added).any(|k| contains(&c, k)) {
                        added.push(c);
                        // Bounded within a round too: each new cube is
                        // compared with all the others.
                        if cubes.len() + added.len() > most {
                            break 'pairs;
                        }
                    }
                }
            }
        }
        if added.is_empty() {
            return;
        }
        stats.consensus += added.len();
        cubes.extend(added);
        absorb(cubes, stats);
        if cubes.len() > most {
            return;
        }
    }
}

/// Is the union of `cubes` everything? (Shannon expansion; `None` when it
/// takes more than `budget` steps.)
fn tautology(cubes: &[Cube], budget: &mut usize) -> Option<bool> {
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    if cubes.iter().any(|c| c.is_empty()) {
        return Some(true);
    }
    if cubes.is_empty() {
        return Some(false);
    }
    let mut count: BTreeMap<Atom, usize> = BTreeMap::new();
    for c in cubes {
        for k in c.keys() {
            *count.entry(*k).or_default() += 1;
        }
    }
    let (&x, _) = count.iter().max_by_key(|(_, n)| **n)?;
    for sense in [true, false] {
        let part: Vec<Cube> = cubes
            .iter()
            .filter(|c| c.get(&x) != Some(&!sense))
            .map(|c| {
                let mut c = c.clone();
                c.remove(&x);
                c
            })
            .collect();
        if !tautology(&part, budget)? {
            return Some(false);
        }
    }
    Some(true)
}

/// Cubes the others cover are dropped, longest first (a test that is too
/// long keeps its cube: the function is the same either way).
fn irredundant(cubes: &mut Vec<Cube>, stats: &mut Stats) {
    let mut order: Vec<usize> = (0..cubes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(cubes[i].len()));
    let mut keep = vec![true; cubes.len()];
    for i in order {
        if stats.over() {
            break;
        }
        // What is left of the other cubes inside cube i.
        let inside: Vec<Cube> = cubes
            .iter()
            .enumerate()
            .filter(|&(j, c)| j != i && keep[j] && merge(c, &cubes[i]).is_some())
            .map(|(_, c)| {
                let mut c = c.clone();
                c.retain(|k, _| !cubes[i].contains_key(k));
                c
            })
            .collect();
        let mut budget = MAX_TAUTOLOGY;
        if tautology(&inside, &mut budget) == Some(true) {
            keep[i] = false;
            stats.redundant += 1;
        }
    }
    let mut k = keep.into_iter();
    cubes.retain(|_| k.next().unwrap_or(true));
}

/// Factoring: the literal in most cubes is taken out (`xa + xb + c =
/// x(a + b) + c`), again in each part.
fn factor(cubes: &[Cube], stats: &mut Stats) -> Bool {
    match cubes {
        [] => return Bool::False,
        [c] => return cube_bool(c),
        _ => {}
    }
    let mut count: BTreeMap<(Atom, bool), usize> = BTreeMap::new();
    for c in cubes {
        for (k, v) in c {
            *count.entry((*k, *v)).or_default() += 1;
        }
    }
    let Some((&(x, sense), &n)) = count.iter().max_by_key(|(_, n)| **n) else {
        return Bool::False;
    };
    if n < 2 {
        return Bool::Or(cubes.iter().map(cube_bool).collect());
    }
    stats.factored += 1;
    let (with, rest): (Vec<&Cube>, Vec<&Cube>) =
        cubes.iter().partition(|c| c.get(&x) == Some(&sense));
    let with: Vec<Cube> = with
        .into_iter()
        .map(|c| {
            let mut c = c.clone();
            c.remove(&x);
            c
        })
        .collect();
    let rest: Vec<Cube> = rest.into_iter().cloned().collect();
    let term = Bool::And(vec![Bool::Lit(x, sense), factor(&with, stats)]);
    if rest.is_empty() {
        term
    } else {
        Bool::Or(vec![term, factor(&rest, stats)])
    }
}

const FALSE: u32 = 0;
const TRUE: u32 = 1;

/// Reduced ordered binary decision diagrams: the same function is always the
/// same node, so two expressions are equal exactly when their nodes are.
struct Bdd {
    /// (variable, low, high); entries 0 and 1 are the constants.
    nodes: Vec<(u32, u32, u32)>,
    unique: HashMap<(u32, u32, u32), u32>,
    cache: HashMap<(u32, u32, u32), u32>,
    vars: BTreeMap<Atom, u32>,
    limits: Limits,
    /// Calls of `ite` that did work (the limits are looked at every so many).
    steps: u32,
}

impl Bdd {
    fn new(limits: &Limits) -> Self {
        Bdd {
            nodes: vec![(u32::MAX, 0, 0), (u32::MAX, 1, 1)],
            unique: HashMap::new(),
            cache: HashMap::new(),
            vars: BTreeMap::new(),
            limits: limits.clone(),
            steps: 0,
        }
    }

    fn mk(&mut self, v: u32, lo: u32, hi: u32) -> Option<u32> {
        if lo == hi {
            return Some(lo);
        }
        if let Some(&n) = self.unique.get(&(v, lo, hi)) {
            return Some(n);
        }
        if self.nodes.len() >= MAX_BDD
            || (self.nodes.len().is_multiple_of(4096) && self.limits.over())
        {
            return None;
        }
        let n = self.nodes.len() as u32;
        self.nodes.push((v, lo, hi));
        self.unique.insert((v, lo, hi), n);
        Some(n)
    }

    fn var_of(&self, f: u32) -> u32 {
        self.nodes[f as usize].0
    }

    fn cofactors(&self, f: u32, v: u32) -> (u32, u32) {
        let (w, lo, hi) = self.nodes[f as usize];
        if w == v {
            (lo, hi)
        } else {
            (f, f)
        }
    }

    /// if `f` then `g` else `h`.
    fn ite(&mut self, f: u32, g: u32, h: u32) -> Option<u32> {
        if f == TRUE {
            return Some(g);
        }
        if f == FALSE {
            return Some(h);
        }
        if g == h {
            return Some(g);
        }
        if g == TRUE && h == FALSE {
            return Some(f);
        }
        if let Some(&r) = self.cache.get(&(f, g, h)) {
            return Some(r);
        }
        self.steps = self.steps.wrapping_add(1);
        if self.steps.is_multiple_of(1024) && self.limits.over() {
            return None;
        }
        let v = self.var_of(f).min(self.var_of(g)).min(self.var_of(h));
        let (f0, f1) = self.cofactors(f, v);
        let (g0, g1) = self.cofactors(g, v);
        let (h0, h1) = self.cofactors(h, v);
        let lo = self.ite(f0, g0, h0)?;
        let hi = self.ite(f1, g1, h1)?;
        let r = self.mk(v, lo, hi)?;
        self.cache.insert((f, g, h), r);
        Some(r)
    }

    fn build(&mut self, b: &Bool) -> Option<u32> {
        match b {
            Bool::True => Some(TRUE),
            Bool::False => Some(FALSE),
            Bool::Lit(a, pos) => {
                let next = self.vars.len() as u32;
                let v = *self.vars.entry(*a).or_insert(next);
                if *pos {
                    self.mk(v, FALSE, TRUE)
                } else {
                    self.mk(v, TRUE, FALSE)
                }
            }
            Bool::Not(x) => {
                let f = self.build(x)?;
                self.ite(f, FALSE, TRUE)
            }
            Bool::And(v) => {
                let mut acc = TRUE;
                for x in v {
                    let f = self.build(x)?;
                    acc = self.ite(acc, f, FALSE)?;
                }
                Some(acc)
            }
            Bool::Or(v) => {
                let mut acc = FALSE;
                for x in v {
                    let f = self.build(x)?;
                    acc = self.ite(acc, TRUE, f)?;
                }
                Some(acc)
            }
        }
    }
}

/// Are the two expressions the same function of their atoms? `None` if the
/// diagrams get too large to tell.
#[cfg(test)]
fn equivalent(a: &Bool, b: &Bool) -> Option<bool> {
    let mut bdd = Bdd::new(&Limits::default());
    let fa = bdd.build(a)?;
    let fb = bdd.build(b)?;
    Some(fa == fb)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(geometry: &str) -> Expr {
        let m = migjorn::Model::parse(&format!("t\n1 0 {geometry} imp:n=1\n\n1 SO 5\n\n"));
        let e = m
            .cells()
            .next()
            .unwrap()
            .geometry_expr()
            .unwrap_or_else(|e| panic!("{geometry}: {e:?}"));
        e
    }

    fn surface(id: i64, negative: bool) -> Expr {
        Expr::Surface(SurfaceRef {
            id,
            facet: None,
            negative,
        })
    }

    /// Cells by number, as text.
    fn world<'a>(cells: &'a [(i64, &'a str)]) -> impl Fn(i64) -> Option<Expr> + 'a {
        move |n| cells.iter().find(|c| c.0 == n).map(|c| parse(c.1))
    }

    /// Value of an expression for an assignment of the surfaces (bit k of
    /// `bits` is the positive sense of surface k+1).
    fn eval(e: &Expr, bits: u32, cells: &dyn Fn(i64) -> Option<Expr>) -> bool {
        match e {
            Expr::Surface(s) => ((bits >> (s.id - 1)) & 1 == 1) != s.negative,
            Expr::Complement(n) => !eval(&cells(*n).unwrap(), bits, cells),
            Expr::Not(x) => !eval(x, bits, cells),
            Expr::And(v) => v.iter().all(|x| eval(x, bits, cells)),
            Expr::Or(v) => v.iter().any(|x| eval(x, bits, cells)),
        }
    }

    #[test]
    fn plain_cut_adds_the_complement() {
        let cut = cut_expr(&parse("-1 : 2"), 7, &|_| None).unwrap();
        assert_eq!(cut.geometry, "(-1:2) #7");
        assert!(cut.terms_after <= cut.terms_before, "{cut:?}");
        assert!(cut.warnings[0].contains("cell 7"), "{:?}", cut.warnings);
    }

    #[test]
    fn already_cut_is_left_alone() {
        let a = parse("-1 #7");
        assert_eq!(plain(&a, 7), a);
        assert_eq!(plain(&parse("#7"), 7), parse("#7"));
    }

    #[test]
    fn contradictions_and_absorption_shorten_the_cut() {
        // (-1 : -2) 3 (1 : -3): -1·1 and 3·-3 are 0, what is left is 1 -2 3.
        let cells = [(2, "-1 3")];
        let cut = cut_expr(&parse("(-1:-2) 3"), 2, &world(&cells)).unwrap();
        assert_eq!(cut.plain, "(-1:-2) 3 #2");
        assert_eq!(cut.geometry, "-2 3 1");
        assert!(cut.simplified);
        assert!(cut.terms_after < cut.terms_before);
        assert!(!cut.trace.is_empty());
    }

    #[test]
    fn a_cell_inside_the_other_leaves_nothing() {
        let cells = [(2, "-1")];
        let cut = cut_expr(&parse("-1 2"), 2, &world(&cells)).unwrap();
        assert!(cut.empty);
    }

    #[test]
    fn disjoint_cells_leave_the_cell_as_it_is() {
        // B = -1 -2, A = 1 3: nothing of A is in B, A ∩ ¬B = A.
        let cells = [(2, "-1 -2")];
        let cut = cut_expr(&parse("1 3"), 2, &world(&cells)).unwrap();
        assert_eq!(cut.geometry, "1 3");
        assert!(cut.simplified);
    }

    #[test]
    fn complements_inside_complements_expand() {
        let cells = [(2, "-1 #3"), (3, "2")];
        let w = world(&cells);
        let a = parse("-1 -3");
        let cut = cut_expr(&a, 2, &w).unwrap();
        let got = parse(&cut.geometry);
        for bits in 0..8 {
            let want = eval(&a, bits, &w) && !eval(&w(2).unwrap(), bits, &w);
            assert_eq!(eval(&got, bits, &w), want, "{bits:03b}: {}", cut.geometry);
        }
    }

    #[test]
    fn cycles_and_missing_cells_fall_back_to_the_plain_cut() {
        let cells = [(2, "-1 #2")];
        let cut = cut_expr(&parse("-1"), 2, &world(&cells)).unwrap();
        assert_eq!(cut.geometry, "-1 #2");
        assert!(cut.warnings[0].contains("itself"), "{:?}", cut.warnings);
    }

    #[test]
    fn facets_are_variables_of_their_own() {
        // 470.1 and 470.2 are not each other's negation: 470.2 and -470.2 are,
        // and (470.2 : 470.1) 5 minus (470.2 5) is 5 470.1 -470.2.
        let cells = [(2, "470.2 5")];
        let cut = cut_expr(&parse("(470.2 : 470.1) 5"), 2, &world(&cells)).unwrap();
        assert_eq!(cut.geometry, "470.1 5 -470.2");
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    fn random(r: &mut Rng, depth: u32, cells: &[i64]) -> Expr {
        let leaf = depth == 0 || r.next(4) == 0;
        if leaf {
            if !cells.is_empty() && r.next(5) == 0 {
                return Expr::Complement(cells[r.next(cells.len() as u64) as usize]);
            }
            return surface(1 + r.next(5) as i64, r.next(2) == 0);
        }
        let n = 2 + r.next(2) as usize;
        let items: Vec<Expr> = (0..n).map(|_| random(r, depth - 1, cells)).collect();
        match r.next(4) {
            0 => Expr::Or(items),
            1 => Expr::Not(Box::new(Expr::And(items))),
            _ => Expr::And(items),
        }
    }

    #[test]
    fn the_cut_is_a_and_not_b_for_every_assignment() {
        let mut r = Rng(0x9e37_79b9_7f4a_7c15);
        let mut simplified = 0;
        for case in 0..400 {
            // Cell 20 is below cell 21 in the chain of complements.
            let c20 = random(&mut r, 3, &[]);
            let c21 = random(&mut r, 3, &[20]);
            let b = random(&mut r, 3, &[20]);
            let a = random(&mut r, 3, &[20, 21]);
            let table = [(20, c20), (21, c21), (22, b)];
            let cells = |n: i64| table.iter().find(|c| c.0 == n).map(|c| c.1.clone());
            let cut = cut_expr(&a, 22, &cells).unwrap();
            // What the text written reads back as.
            let m =
                migjorn::Model::parse(&format!("t\n1 0 {} imp:n=1\n\n1 SO 5\n\n", cut.geometry));
            if cut.empty {
                for bits in 0..32 {
                    // All of A is in B.
                    assert!(
                        !eval(&a, bits, &cells) || eval(&cells(22).unwrap(), bits, &cells),
                        "case {case}"
                    );
                }
                continue;
            }
            let got = m.cells().next().unwrap().geometry_expr().unwrap();
            simplified += cut.simplified as usize;
            for bits in 0..32 {
                let want = eval(&a, bits, &cells) && !eval(&cells(22).unwrap(), bits, &cells);
                assert_eq!(
                    eval(&got, bits, &cells),
                    want,
                    "case {case}: {}",
                    cut.geometry
                );
            }
            assert!(cut.terms_after <= cut.terms_before);
        }
        assert!(simplified > 20, "only {simplified} cuts were shortened");
    }

    #[test]
    fn the_proof_rejects_a_wrong_form() {
        let x = Bool::Lit(Atom::Surface { id: 1, facet: None }, true);
        let y = Bool::Lit(Atom::Surface { id: 2, facet: None }, true);
        let both = Bool::And(vec![x.clone(), y.clone()]);
        let either = Bool::Or(vec![x.clone(), y.clone()]);
        assert_eq!(equivalent(&both, &either), Some(false));
        let not_not = Bool::Not(Box::new(Bool::Not(Box::new(both.clone()))));
        assert_eq!(equivalent(&both, &not_not), Some(true));
        // De Morgan.
        let lhs = Bool::Not(Box::new(both));
        let rhs = Bool::Or(vec![
            Bool::Lit(Atom::Surface { id: 1, facet: None }, false),
            Bool::Lit(Atom::Surface { id: 2, facet: None }, false),
        ]);
        assert_eq!(equivalent(&lhs, &rhs), Some(true));
    }

    /// A pseudo-random expression in negation normal form over `atoms`
    /// surfaces, `depth` levels deep.
    fn step(seed: &mut u64, n: u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed % n
    }

    fn random_nnf(seed: &mut u64, atoms: i64, depth: u32) -> Bool {
        if depth == 0 || step(seed, 4) == 0 {
            return Bool::Lit(
                Atom::Surface {
                    id: 1 + step(seed, atoms as u64) as i64,
                    facet: None,
                },
                step(seed, 2) == 0,
            );
        }
        let n = 2 + step(seed, 3) as usize;
        let v: Vec<Bool> = (0..n).map(|_| random_nnf(seed, atoms, depth - 1)).collect();
        if step(seed, 2) == 0 {
            Bool::And(v)
        } else {
            Bool::Or(v)
        }
    }

    #[test]
    fn the_tree_tier_keeps_the_function() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut shorter = 0;
        for k in 0..400 {
            let b = random_nnf(&mut seed, 5, 1 + k % 4);
            let mut stats = Stats::new(&Limits::default(), MAX_CUBES);
            let t = tree::simplify(&b, &mut stats);
            assert_eq!(equivalent(&b, &t), Some(true), "{b:?}\n{t:?}");
            let size = |x: &Bool| format!("{x:?}").len();
            assert!(size(&t) <= size(&b), "not longer: {b:?} -> {t:?}");
            shorter += (size(&t) < size(&b)) as usize;
        }
        assert!(shorter > 50, "the tier does something: {shorter}");
    }

    #[test]
    fn known_literals_are_propagated() {
        // x(x + y) = x; x(¬x + y) = xy; x + ¬x y = x + y.
        let x = Atom::Surface { id: 1, facet: None };
        let y = Atom::Surface { id: 2, facet: None };
        let l = |a, p| Bool::Lit(a, p);
        let mut st = Stats::new(&Limits::default(), MAX_CUBES);
        let one = Bool::And(vec![l(x, true), Bool::Or(vec![l(x, true), l(y, true)])]);
        assert_eq!(
            tree::simplify(&one, &mut st),
            Bool::And(vec![l(x, true)]).clone().into_single()
        );
        let two = Bool::And(vec![l(x, true), Bool::Or(vec![l(x, false), l(y, true)])]);
        assert_eq!(
            tree::simplify(&two, &mut st),
            Bool::And(vec![l(x, true), l(y, true)])
        );
        let three = Bool::Or(vec![l(x, true), Bool::And(vec![l(x, false), l(y, true)])]);
        assert_eq!(
            tree::simplify(&three, &mut st),
            Bool::Or(vec![l(x, true), l(y, true)])
        );
    }

    impl Bool {
        fn into_single(self) -> Bool {
            match self {
                Bool::And(mut v) if v.len() == 1 => v.remove(0),
                other => other,
            }
        }
    }

    #[test]
    fn a_big_cut_is_done_at_once() {
        // 14 groups ANDed, cut by 14 planes: its disjunctive normal form has
        // thousands of products (it took seconds): now at once, and proven.
        let mut a = String::new();
        for k in 0..14 {
            a += &format!("(-{}:-{}) ", 10 + 2 * k, 11 + 2 * k);
        }
        let mut b = String::new();
        for k in 0..16 {
            b += &format!("{}{} ", if k % 3 == 0 { "-" } else { "" }, 10 + k);
        }
        let m = migjorn::Model::parse(&format!(
            "t\n1 0 {a}1 imp:n=1\n2 0 {b}1 imp:n=1\n\n1 so 5\n\n"
        ));
        let a = m.cell(1).unwrap().geometry_expr().unwrap();
        let cells = |n: i64| m.cell(n).and_then(|c| c.geometry_expr().ok());
        let t = std::time::Instant::now();
        let cut = cut_expr_within(&a, 2, &cells, &Limits::within(Duration::from_secs(20))).unwrap();
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert!(cut.terms_after <= cut.terms_before, "{cut:?}");
    }

    /// `(a:b1)…(a:b9)` and k groups `(c:d)`: as a union of products it has
    /// over 256 of them (the second tier gives up), but its complement has
    /// few, and negated back it is `(a : b1 … b9)` and the groups.
    #[test]
    fn the_dual_form_shortens_an_intersection_of_unions() {
        let k = 8;
        let a = Atom::Surface { id: 1, facet: None };
        let lit = |id: i64| Bool::Lit(Atom::Surface { id, facet: None }, true);
        let mut items: Vec<Bool> = (0..9)
            .map(|j| Bool::Or(vec![Bool::Lit(a, true), lit(10 + j)]))
            .collect();
        items.extend((0..k).map(|j| Bool::Or(vec![lit(30 + 2 * j), lit(31 + 2 * j)])));
        let f = Bool::And(items);
        let before = 18 + 2 * k as usize;
        let s = simplify_bool(&f, &Limits::within(Duration::from_secs(20))).unwrap();
        let e = s.expr.expect("a form");
        assert_eq!(s.terms, 10 + 2 * k as usize, "{e} (from {before} terms)");
        let mut nodes = 0;
        let back = to_bool(&e, None, &mut Vec::new(), &mut nodes).unwrap();
        assert_eq!(equivalent(&f, &back), Some(true));
    }

    /// A cell of m groups cut by planes: the first tier's short form stays,
    /// and the dual does not make it slower.
    #[test]
    fn groups_cut_by_planes_stay_short_and_fast() {
        let m = 12;
        let mut a = String::new();
        for k in 0..m {
            a += &format!("(-{}:-{}) ", 10 + 2 * k, 11 + 2 * k);
        }
        let mut b = String::new();
        for k in 0..16 {
            b += &format!("{}{} ", if k % 3 == 0 { "-" } else { "" }, 10 + k);
        }
        let model = migjorn::Model::parse(&format!(
            "t\n1 0 {a}1 imp:n=1\n2 0 {b}1 imp:n=1\n\n1 so 5\n\n"
        ));
        let ea = model.cell(1).unwrap().geometry_expr().unwrap();
        let cells = |n: i64| model.cell(n).and_then(|c| c.geometry_expr().ok());
        let t = std::time::Instant::now();
        let cut =
            cut_expr_within(&ea, 2, &cells, &Limits::within(Duration::from_secs(20))).unwrap();
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        assert!(cut.terms_after <= 2 * m + 1, "{}", cut.geometry);
    }

    #[test]
    fn cancelling_stops_the_work() {
        let stop = Arc::new(AtomicBool::new(true));
        let limits = Limits {
            until: None,
            cancel: Some(stop),
        };
        let mut seed = 7u64;
        let b = random_nnf(&mut seed, 6, 4);
        // Stopped before it began: the tier-1 form, or no form, never a wrong one.
        let s = simplify_bool(&b, &limits);
        assert!(s.is_ok() || s.is_err_and(|e| !e.contains("internal")));
    }

    /// A proof that creates a handful of nodes still looks at the limits.
    #[test]
    fn a_cancelled_proof_gives_no_answer() {
        let stop = Arc::new(AtomicBool::new(true));
        let limits = Limits {
            until: None,
            cancel: Some(stop),
        };
        let e = parse("-1 2");
        assert_eq!(same(&e, &e, &|_| None, &limits), None);
        assert_eq!(same(&e, &e, &|_| None, &Limits::default()), Some(true));
    }

    /// Two unions of 256 atoms each, with none shared, are 65 536 products:
    /// the second tier gives up before absorbing them, at once.
    #[test]
    fn a_huge_product_of_unions_is_given_up_at_once() {
        let lit = |id: i64| Bool::Lit(Atom::Surface { id, facet: None }, true);
        let union = |from: i64| Bool::Or((from..from + 256).map(lit).collect());
        let f = Bool::And(vec![union(1), union(1000)]);
        let t = std::time::Instant::now();
        let mut stats = Stats::new(&Limits::default(), MAX_CUBES);
        assert!(dnf(&f, &mut stats).is_none());
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    }
}
