// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! The first, cheap tier of simplification: on the tree of the expression,
//! with no normal form, as a person would do it. It is in the spirit of
//! GEOUNED's `BoolSequence.simplify`, but not taken from it: that one expands
//! on each variable (Shannon) and may use geometric constraints; this one
//! propagates what the enclosing products and sums fix, and uses none.
//!
//! Within `x·(…)` the `x` is true, within `x+(…)` it is false, at any depth.
//! So a literal met again below is a constant, and the constants are
//! propagated (`x·(x+y) = x`, `x·(¬x+y) = x·y`, `x+(¬x·y) = x+y`). On top:
//! same operators flattened, equal terms joined, `x·¬x = 0`, `x+¬x = 1`, and
//! absorption of whole products or sums (`ab + abc = ab`).
//!
//! Every step is an identity valid for any value of the variables, and each
//! pass costs about one visit of every node, so this tier is for any size;
//! the result is still proven equal to the original by the caller.

use std::collections::{BTreeMap, HashMap};

use super::{Atom, Bool, Stats};

/// Most passes: each one gives a form no larger than the last.
const PASSES: usize = 16;
/// Most terms of a sum or product compared in pairs for absorption.
const ABSORB_MOST: usize = 2000;

/// `b` (in negation normal form: no `Not`) simplified.
pub fn simplify(b: &Bool, stats: &mut Stats) -> Bool {
    let mut cur = b.clone();
    for _ in 0..PASSES {
        let mut known = HashMap::new();
        let next = pass(&cur, &mut known, stats);
        let same = next == cur;
        cur = next;
        if same || stats.over() {
            break;
        }
    }
    cur
}

/// One visit. `known`: the atoms that are known, by the products and sums
/// around, to be true (`true`) or false.
fn pass(b: &Bool, known: &mut HashMap<Atom, bool>, stats: &mut Stats) -> Bool {
    let (items, and) = match b {
        Bool::And(v) => (v, true),
        Bool::Or(v) => (v, false),
        Bool::Lit(a, p) => {
            return match known.get(a) {
                Some(v) => {
                    stats.propagated += 1;
                    constant(v == p)
                }
                None => b.clone(),
            }
        }
        _ => return b.clone(),
    };
    // What ends the operator at once, and what it ignores.
    let (zero, one) = (constant(!and), constant(and));
    let mut flat: Vec<&Bool> = Vec::new();
    flatten(items, and, &mut flat);

    // The literals first: they are what the others are simplified by.
    let mut lits: BTreeMap<Atom, bool> = BTreeMap::new();
    for it in &flat {
        if let Bool::Lit(a, p) = it {
            match known.get(a) {
                Some(v) if v == p => {
                    stats.propagated += 1;
                    if !and {
                        return zero;
                    }
                    continue;
                }
                Some(_) => {
                    stats.propagated += 1;
                    if and {
                        return zero;
                    }
                    continue;
                }
                None => {}
            }
            match lits.insert(*a, *p) {
                Some(q) if q != *p => {
                    stats.contradictions += 1;
                    return zero;
                }
                Some(_) => stats.absorbed += 1,
                None => {}
            }
        }
    }
    // Under `and`, the literals are true for the terms; under `or`, false.
    let mut set: Vec<Atom> = Vec::new();
    for (a, p) in &lits {
        if known.insert(*a, *p == and).is_none() {
            set.push(*a);
        }
    }
    // The terms stay in the order they were written (the result is shown as
    // a diff of the cell as it was).
    let mut out: Vec<Bool> = Vec::new();
    let mut emitted: Vec<Atom> = Vec::new();
    let mut result = None;
    for it in &flat {
        if let Bool::Lit(a, _) = it {
            if let Some(p) = lits.get(a) {
                if !emitted.contains(a) {
                    emitted.push(*a);
                    out.push(Bool::Lit(*a, *p));
                }
            }
            continue;
        }
        let r = pass(it, known, stats);
        if r == zero {
            result = Some(zero.clone());
            break;
        }
        if r == one {
            continue;
        }
        // (A simplified term may have become the same operator.)
        let mut again = Vec::new();
        flatten(std::slice::from_ref(&r), and, &mut again);
        for x in again {
            if !out.contains(x) {
                out.push(x.clone());
            } else {
                stats.absorbed += 1;
            }
        }
    }
    for a in set {
        known.remove(&a);
    }
    if let Some(z) = result {
        return z;
    }
    absorb(&mut out, and, stats);
    match out.len() {
        0 => one,
        1 => out.pop().unwrap_or(one),
        _ if and => Bool::And(out),
        _ => Bool::Or(out),
    }
}

fn constant(v: bool) -> Bool {
    if v {
        Bool::True
    } else {
        Bool::False
    }
}

/// The terms of an `and` (or `or`) with its own kind inside merged.
fn flatten<'a>(items: &'a [Bool], and: bool, out: &mut Vec<&'a Bool>) {
    for it in items {
        match it {
            Bool::And(v) if and => flatten(v, and, out),
            Bool::Or(v) if !and => flatten(v, and, out),
            _ => out.push(it),
        }
    }
}

/// The literals of a term of the other operator, if it holds only literals
/// (and a lone literal counts: `a` is the sum, or product, of `a`).
fn literals(b: &Bool, and: bool) -> Option<BTreeMap<Atom, bool>> {
    match b {
        Bool::Lit(a, p) => Some(BTreeMap::from([(*a, *p)])),
        Bool::And(v) if !and => lits_of(v),
        Bool::Or(v) if and => lits_of(v),
        _ => None,
    }
}

fn lits_of(v: &[Bool]) -> Option<BTreeMap<Atom, bool>> {
    v.iter()
        .map(|x| match x {
            Bool::Lit(a, p) => Some((*a, *p)),
            _ => None,
        })
        .collect()
}

/// `ab + abc = ab` and `(a+b)(a+b+c) = (a+b)`: of the terms made of literals
/// only, those that have all the literals of another are dropped.
fn absorb(out: &mut Vec<Bool>, and: bool, stats: &mut Stats) {
    if out.len() < 2 || out.len() > ABSORB_MOST {
        return;
    }
    let sets: Vec<Option<BTreeMap<Atom, bool>>> = out.iter().map(|b| literals(b, and)).collect();
    let mut drop = vec![false; out.len()];
    for i in 0..out.len() {
        let Some(si) = &sets[i] else { continue };
        for j in 0..out.len() {
            if i == j || drop[j] {
                continue;
            }
            let Some(sj) = &sets[j] else { continue };
            // `j` has every literal of `i`, and more (or the same, once).
            let has = si.iter().all(|(k, v)| sj.get(k) == Some(v));
            if has && (sj.len() > si.len() || j > i) {
                drop[j] = true;
                stats.absorbed += 1;
            }
        }
    }
    let mut k = 0;
    out.retain(|_| {
        k += 1;
        !drop[k - 1]
    });
}
