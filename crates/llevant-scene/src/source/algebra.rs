// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! Cutting one cell out of another in the Boolean algebra of their geometry:
//! `A ∩ #B`, written in fewer terms when the laws of Boolean algebra allow.
//!
//! Only the algebra is used. Every `+s`/`-s` of a surface is a variable and
//! its negation (a macrobody facet `470.2` is a variable of its own, never
//! tied to the whole surface `470`), and nothing is assumed about where the
//! surfaces are. Each step below is an identity valid for every value of the
//! variables, so the result is the same region in any geometry. On top of
//! that [`cut_expr`] proves the result equal to the unsimplified `A ∩ ¬B`
//! with reduced ordered BDDs (canonical: the same function gets the same
//! node), so the guarantee does not rest on the steps being coded right. A
//! result that cannot be proven is never given.
//!
//! Steps: `#n` replaced by the negation of cell `n` (De Morgan, at any
//! depth), negation normal form, disjunctive normal form (distributivity,
//! `x·¬x = 0`, idempotence, absorption), all prime implicants by iterated
//! consensus, an irredundant cover of them, and factoring (distributivity
//! read backwards). The shortest of the forms is kept.

use std::collections::{BTreeMap, HashMap};

use migjorn::{Expr, SurfaceRef};

/// Most products the disjunctive normal form may reach.
const MAX_CUBES: usize = 4096;
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

/// What [`cut_expr`] gives.
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

/// Counts of the laws applied.
#[derive(Default)]
struct Stats {
    de_morgan: usize,
    distributed: usize,
    contradictions: usize,
    absorbed: usize,
    consensus: usize,
    redundant: usize,
    factored: usize,
}

impl Stats {
    fn lines(&self) -> Vec<String> {
        [
            ("De Morgan", self.de_morgan),
            ("distributivity", self.distributed),
            ("x·¬x = 0", self.contradictions),
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

/// Cell `b` cut out of the cell whose geometry is `a`: `a ∩ #b`.
///
/// `cells` gives the geometry of any cell `#n` may name (including `b`), or
/// `None` for one that cannot be expanded (it has a `TRCL`, is a `LIKE` cell,
/// or does not exist). Then nothing is expanded, and the plain `A #B` is
/// given with the reason in `warnings`.
pub fn cut_expr(a: &Expr, b: i64, cells: &dyn Fn(i64) -> Option<Expr>) -> Result<Cut, String> {
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

    // `A ∩ ¬B` with every `#n` replaced by the negation of cell n.
    let both = Expr::And(vec![a.clone(), Expr::Complement(b)]);
    let mut nodes = 0;
    let reference = match to_bool(&both, Some(cells), &mut Vec::new(), &mut nodes) {
        Ok(r) => r,
        Err(why) => {
            cut.warnings.push(format!("not simplified: {why}"));
            return Ok(cut);
        }
    };

    let mut stats = Stats::default();
    let nnf = nnf(&reference, false, &mut stats);
    let Some(mut cubes) = dnf(&nnf, &mut stats) else {
        cut.warnings
            .push("not simplified: too many terms to expand".into());
        return Ok(cut);
    };
    let mut bdd = Bdd::new();
    let want = match bdd.build(&reference) {
        Some(f) => f,
        None => {
            cut.warnings
                .push("not simplified: too large to prove equal".into());
            return Ok(cut);
        }
    };

    if cubes.is_empty() {
        if want != FALSE {
            return Err("internal error: an empty form is not proven empty".into());
        }
        cut.empty = true;
        cut.trace = stats.lines();
        return Ok(cut);
    }
    if cubes.iter().any(|c| c.is_empty()) {
        cut.warnings
            .push("not simplified: the result has no restriction left".into());
        return Ok(cut);
    }

    primes(&mut cubes, &mut stats);
    irredundant(&mut cubes, &mut stats);
    let flat = Bool::Or(cubes.iter().map(cube_bool).collect());
    let factored = factor(&cubes, &mut stats);

    let mut best: Option<(Expr, usize)> = None;
    for form in [factored, flat] {
        let Some(e) = to_expr(&form) else { continue };
        let n = terms(&e);
        if best.as_ref().is_none_or(|(_, m)| n < *m) {
            best = Some((e, n));
        }
    }
    let Some((e, n)) = best else {
        return Ok(cut);
    };

    // The proof: the form read back, expanded as the reference was, is the
    // same function.
    let mut nodes = 0;
    let back = to_bool(&e, Some(cells), &mut Vec::new(), &mut nodes)
        .map_err(|why| format!("internal error: {why}"))?;
    match bdd.build(&back) {
        Some(f) if f == want => {}
        Some(_) => return Err("internal error: the simplified form differs".into()),
        None => {
            cut.warnings
                .push("not simplified: too large to prove equal".into());
            return Ok(cut);
        }
    }

    cut.trace = stats.lines();
    if n < terms_before {
        cut.geometry = e.to_string();
        cut.terms_after = n;
        cut.simplified = true;
    } else {
        cut.warnings
            .push("the plain A #B is as short as the algebra gets".into());
    }
    Ok(cut)
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
fn absorb(cubes: &mut Vec<Cube>, stats: &mut Stats) {
    cubes.sort_by_key(|c| c.len());
    let before = cubes.len();
    let mut kept: Vec<Cube> = Vec::new();
    for c in cubes.drain(..) {
        if !kept.iter().any(|k| contains(&c, k)) {
            kept.push(c);
        }
    }
    stats.absorbed += before - kept.len();
    *cubes = kept;
}

/// Disjunctive normal form of an expression in negation normal form.
/// `None` when it passes [`MAX_CUBES`] products.
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
                if all.len() > MAX_CUBES {
                    return None;
                }
            }
            all
        }
        Bool::And(v) => {
            let mut acc = vec![Cube::new()];
            for x in v {
                let part = dnf(x, stats)?;
                let mut next = Vec::new();
                for c in &acc {
                    for d in &part {
                        match merge(c, d) {
                            Some(m) => next.push(m),
                            None => stats.contradictions += 1,
                        }
                    }
                }
                if acc.len() > 1 || part.len() > 1 {
                    stats.distributed += 1;
                }
                absorb(&mut next, stats);
                if next.len() > MAX_CUBES {
                    return None;
                }
                acc = next;
            }
            acc
        }
    };
    absorb(&mut cubes, stats);
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
    let most = MAX_CUBES / 4;
    loop {
        let mut added = Vec::new();
        'pairs: for i in 0..cubes.len() {
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
}

impl Bdd {
    fn new() -> Self {
        Bdd {
            nodes: vec![(u32::MAX, 0, 0), (u32::MAX, 1, 1)],
            unique: HashMap::new(),
            cache: HashMap::new(),
            vars: BTreeMap::new(),
        }
    }

    fn mk(&mut self, v: u32, lo: u32, hi: u32) -> Option<u32> {
        if lo == hi {
            return Some(lo);
        }
        if let Some(&n) = self.unique.get(&(v, lo, hi)) {
            return Some(n);
        }
        if self.nodes.len() >= MAX_BDD {
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
pub fn equivalent(a: &Bool, b: &Bool) -> Option<bool> {
    let mut bdd = Bdd::new();
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
        assert!(!cut.simplified);
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
        assert_eq!(cut.geometry, "1 -2 3");
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
        assert_eq!(cut.geometry, "5 470.1 -470.2");
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
}
