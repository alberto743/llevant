//! Cell geometry expressions: lower migjorn's expression tree to a compact
//! program of `u32` words evaluated with short-circuit
//! and three-valued (Kleene) logic.
//!
//! Program encoding, in prefix order:
//! - leaf: bit 31 clear; bit 30 set when the leaf is true on the surface's
//!   positive side; bits 0..30 the surface index.
//! - node: bit 31 set; bit 30 clear for AND, set for OR; bits 0..30 the
//!   number of words that follow it (its box word and its children), so a
//!   decided node skips the rest. The word after the header is a box word:
//!   `NO_BOX`, or `NODE | i` for entry `i` of the model's node boxes.
//!
//! Complements are pushed down to the leaves at compile time (De Morgan), so
//! there is no NOT in a program.

use migjorn::{Expr, SurfaceRef};

use crate::bbox::Aabb;
use crate::transform::Affine;
use crate::vec3::Vec3;

pub const NODE: u32 = 1 << 31;
pub const OR: u32 = 1 << 30;
pub const POS: u32 = 1 << 30;
pub const MASK: u32 = (1 << 30) - 1;
/// Box word of a node without boxes.
pub const NO_BOX: u32 = u32::MAX;

/// How a surface id maps onto compiled surfaces.
#[derive(Clone, Copy, Debug)]
pub enum SurfSlot {
    Single(u32),
    /// A macrobody: facets `first .. first + count`.
    Macro {
        first: u32,
        count: u8,
    },
}

/// What the lowering needs from the rest of the model.
pub trait Resolve {
    fn surface(&self, id: i64) -> Result<SurfSlot, String>;
    /// The geometry of cell `id`, for `#id`, and its TRCL (main frame → the
    /// frame its geometry is written in), if any.
    fn cell_expr(&self, id: i64) -> Result<(Expr, Option<Affine>), String>;
}

/// Lower an expression to a program, appending to `out`. With `tr` (a
/// TRCL), every leaf refers to its surface moved by `tr`: the leaf is written
/// with the untransformed surface index and its position recorded in
/// `pending`, for the caller to swap in a moved copy.
pub fn lower(
    ast: &Expr,
    r: &dyn Resolve,
    tr: Option<Affine>,
    out: &mut Vec<u32>,
    pending: &mut Vec<(usize, Affine)>,
) -> Result<(), String> {
    let mut stack = Vec::new();
    Lower {
        r,
        out,
        stack: &mut stack,
        tr,
        pending,
    }
    .node(ast, false, None)
}

struct Lower<'a> {
    r: &'a dyn Resolve,
    out: &'a mut Vec<u32>,
    /// Cells being expanded through `#n`, to catch cycles.
    stack: &'a mut Vec<i64>,
    tr: Option<Affine>,
    pending: &'a mut Vec<(usize, Affine)>,
}

impl Lower<'_> {
    /// Emit `ast` (negated if `neg`). `parent` is the kind of the enclosing
    /// node (`Some(false)` AND, `Some(true)` OR): a child of the same kind is
    /// spliced into it instead of nested.
    fn node(&mut self, ast: &Expr, neg: bool, parent: Option<bool>) -> Result<(), String> {
        match ast {
            Expr::Surface(s) => self.surf(*s, neg, parent),
            Expr::Not(inner) => self.node(inner, !neg, parent),
            Expr::Complement(id) => {
                if self.stack.contains(id) {
                    return Err(format!("cyclic complement through cell {id}"));
                }
                let (inner, trcl) = self.r.cell_expr(*id)?;
                // The complemented cell's own TRCL applies first, then ours.
                let saved = self.tr;
                self.tr = match (trcl, saved) {
                    (Some(n), Some(c)) => Some(n.compose(&c)),
                    (n, c) => n.or(c),
                };
                self.stack.push(*id);
                let res = self.node(&inner, !neg, parent);
                self.stack.pop();
                self.tr = saved;
                res
            }
            Expr::And(kids) => self.group(kids, neg, parent, false),
            Expr::Or(kids) => self.group(kids, neg, parent, true),
        }
    }

    fn group(
        &mut self,
        kids: &[Expr],
        neg: bool,
        parent: Option<bool>,
        or: bool,
    ) -> Result<(), String> {
        let is_or = or != neg;
        if kids.len() == 1 {
            return self.node(&kids[0], neg, parent);
        }
        if parent == Some(is_or) {
            for k in kids {
                self.node(k, neg, parent)?;
            }
            return Ok(());
        }
        let head = self.out.len();
        self.out.push(0);
        self.out.push(NO_BOX);
        for k in kids {
            self.node(k, neg, Some(is_or))?;
        }
        self.close(head, is_or)
    }

    fn close(&mut self, head: usize, is_or: bool) -> Result<(), String> {
        let len = self.out.len() - head - 1;
        if len > MASK as usize {
            return Err("geometry expression too large".into());
        }
        self.out[head] = NODE | if is_or { OR } else { 0 } | len as u32;
        Ok(())
    }

    fn surf(&mut self, s: SurfaceRef, neg: bool, parent: Option<bool>) -> Result<(), String> {
        let slot = self.r.surface(s.id)?;
        // The leaf is true on the negative side for `-s`, positive for `s`.
        let want_pos = s.negative == neg;
        match (slot, s.facet) {
            (SurfSlot::Single(i), None) => self.leaf(i, want_pos),
            (SurfSlot::Single(_), Some(f)) => {
                return Err(format!("surface {} is not a macrobody (facet {f})", s.id))
            }
            (SurfSlot::Macro { first, count }, Some(f)) => {
                if f == 0 || f > count {
                    return Err(format!("macrobody {} has no facet {f}", s.id));
                }
                self.leaf(first + f as u32 - 1, want_pos)
            }
            (SurfSlot::Macro { first, count }, None) => {
                // Inside the body = every facet negative; outside = any positive.
                let is_or = want_pos;
                if count == 1 {
                    self.leaf(first, want_pos);
                } else if parent == Some(is_or) {
                    for i in 0..count as u32 {
                        self.leaf(first + i, want_pos);
                    }
                } else {
                    let head = self.out.len();
                    self.out.push(0);
                    self.out.push(NO_BOX);
                    for i in 0..count as u32 {
                        self.leaf(first + i, want_pos);
                    }
                    self.close(head, is_or)?;
                }
            }
        }
        Ok(())
    }

    fn leaf(&mut self, surface: u32, want_pos: bool) {
        if let Some(t) = self.tr {
            self.pending.push((self.out.len(), t));
        }
        self.out.push(surface | if want_pos { POS } else { 0 });
    }
}

/// Kleene truth value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    False,
    True,
    Unknown,
}

/// Evaluate a whole program at `p`; `leaf(surface, want_pos)` gives each
/// leaf's value. With `boxes`, a node whose may-be-true box excludes `p` is
/// false and one whose may-be-false box excludes it is true, without looking
/// at its children. That is exact only when `p` lies in the outer box the
/// node boxes were computed in (the cell's region of interest).
#[inline]
pub fn eval(
    code: &[u32],
    p: Vec3,
    boxes: Option<&[[Aabb; 2]]>,
    leaf: &mut impl FnMut(u32, bool) -> Tri,
) -> Tri {
    if code.is_empty() {
        return Tri::False;
    }
    let mut pos = 0;
    eval_at(code, &mut pos, p, boxes, leaf)
}

fn eval_at(
    code: &[u32],
    pos: &mut usize,
    p: Vec3,
    boxes: Option<&[[Aabb; 2]]>,
    leaf: &mut impl FnMut(u32, bool) -> Tri,
) -> Tri {
    let w = code[*pos];
    *pos += 1;
    if w & NODE == 0 {
        return leaf(w & MASK, w & POS != 0);
    }
    let end = *pos + (w & MASK) as usize;
    let bw = code[*pos];
    *pos += 1;
    if let (Some(bx), true) = (boxes, bw != NO_BOX) {
        let [t, f] = &bx[(bw & !NODE) as usize];
        if !t.contains(p) {
            *pos = end;
            return Tri::False;
        }
        if !f.contains(p) {
            *pos = end;
            return Tri::True;
        }
    }
    // AND stops at the first false, OR at the first true.
    let (stop, other) = if w & OR != 0 {
        (Tri::True, Tri::False)
    } else {
        (Tri::False, Tri::True)
    };
    let mut acc = other;
    while *pos < end {
        match eval_at(code, pos, p, boxes, leaf) {
            v if v == stop => {
                *pos = end;
                return stop;
            }
            Tri::Unknown => acc = Tri::Unknown,
            _ => {}
        }
    }
    acc
}

/// Evaluate a program over a whole box: `True` or `False` only if [`eval`]
/// gives that value at every point of `b`, given a `leaf` that decides each
/// leaf over the whole box. Node boxes prune as in [`eval`], so `b` must lie
/// inside the region they were built for.
pub fn eval_box(
    code: &[u32],
    b: &Aabb,
    boxes: &[[Aabb; 2]],
    leaf: &mut impl FnMut(u32, bool) -> Tri,
) -> Tri {
    if code.is_empty() {
        return Tri::False;
    }
    let mut pos = 0;
    eval_box_at(code, &mut pos, b, boxes, leaf)
}

fn eval_box_at(
    code: &[u32],
    pos: &mut usize,
    b: &Aabb,
    boxes: &[[Aabb; 2]],
    leaf: &mut impl FnMut(u32, bool) -> Tri,
) -> Tri {
    let w = code[*pos];
    *pos += 1;
    if w & NODE == 0 {
        return leaf(w & MASK, w & POS != 0);
    }
    let end = *pos + (w & MASK) as usize;
    let bw = code[*pos];
    *pos += 1;
    if bw != NO_BOX {
        // `eval` returns False outside the may-be-true box and True outside
        // the may-be-false box.
        let [t, f] = &boxes[(bw & !NODE) as usize];
        if !t.overlaps(b) {
            *pos = end;
            return Tri::False;
        }
        if !f.overlaps(b) {
            *pos = end;
            return Tri::True;
        }
    }
    let (stop, other) = if w & OR != 0 {
        (Tri::True, Tri::False)
    } else {
        (Tri::False, Tri::True)
    };
    let mut acc = other;
    while *pos < end {
        match eval_box_at(code, pos, b, boxes, leaf) {
            v if v == stop => {
                *pos = end;
                return stop;
            }
            Tri::Unknown => acc = Tri::Unknown,
            _ => {}
        }
    }
    acc
}

/// Specialize a program to a box: append to `out` a program that [`eval`]
/// gives the same value as `code` at every point of `b`, or return the
/// value when it is the same everywhere in `b` (`True` or `False`; nothing
/// is appended then). `leaf` decides each leaf over the whole box, as for
/// [`eval_box`]: a child decided over the box is dropped from its node (an
/// AND's true children, an OR's false ones) or decides it, and a node left
/// with one child is replaced by it. Box words are kept, so the result is
/// evaluated with the same node boxes, which `b` must lie inside the region
/// of.
pub fn specialize(
    code: &[u32],
    b: &Aabb,
    boxes: &[[Aabb; 2]],
    leaf: &mut impl FnMut(u32, bool) -> Tri,
    out: &mut Vec<u32>,
) -> Option<Tri> {
    if code.is_empty() {
        return Some(Tri::False);
    }
    let mut pos = 0;
    specialize_at(code, &mut pos, b, boxes, leaf, out)
}

fn specialize_at(
    code: &[u32],
    pos: &mut usize,
    b: &Aabb,
    boxes: &[[Aabb; 2]],
    leaf: &mut impl FnMut(u32, bool) -> Tri,
    out: &mut Vec<u32>,
) -> Option<Tri> {
    let w = code[*pos];
    *pos += 1;
    if w & NODE == 0 {
        return match leaf(w & MASK, w & POS != 0) {
            Tri::Unknown => {
                out.push(w);
                None
            }
            v => Some(v),
        };
    }
    let end = *pos + (w & MASK) as usize;
    let bw = code[*pos];
    *pos += 1;
    if bw != NO_BOX {
        let [t, f] = &boxes[(bw & !NODE) as usize];
        if !t.overlaps(b) {
            *pos = end;
            return Some(Tri::False);
        }
        if !f.overlaps(b) {
            *pos = end;
            return Some(Tri::True);
        }
    }
    let (stop, other) = if w & OR != 0 {
        (Tri::True, Tri::False)
    } else {
        (Tri::False, Tri::True)
    };
    let head = out.len();
    out.push(0);
    out.push(bw);
    let mut kids = 0;
    while *pos < end {
        match specialize_at(code, pos, b, boxes, leaf, out) {
            Some(v) if v == stop => {
                out.truncate(head);
                *pos = end;
                return Some(stop);
            }
            // Decided the other way: left out.
            Some(_) => {}
            None => kids += 1,
        }
    }
    match kids {
        0 => {
            out.truncate(head);
            Some(other)
        }
        1 => {
            out.drain(head..head + 2);
            None
        }
        _ => {
            let len = out.len() - head - 1;
            out[head] = (w & (NODE | OR)) | len as u32;
            None
        }
    }
}

/// Length in words of the program (a leaf or a node) starting at `pos`.
#[inline]
pub fn program_len(code: &[u32], pos: usize) -> usize {
    let w = code[pos];
    if w & NODE == 0 {
        1
    } else {
        1 + (w & MASK) as usize
    }
}

/// The program starting at `pos` of `code`.
#[inline]
pub fn program_at(code: &[u32], pos: usize) -> &[u32] {
    &code[pos..pos + program_len(code, pos)]
}

/// Visit the surfaces of the leaves that can matter where `crosses(box)`:
/// a node is skipped when `crosses` is false for its may-be-true box (it is
/// false wherever `crosses` looks) or for its may-be-false box (true there),
/// as [`eval`] would decide it without its children.
pub fn leaves_where(
    code: &[u32],
    boxes: &[[Aabb; 2]],
    crosses: &mut impl FnMut(&Aabb) -> bool,
    f: &mut impl FnMut(u32),
) {
    let mut pos = 0;
    while pos < code.len() {
        let w = code[pos];
        if w & NODE == 0 {
            f(w & MASK);
            pos += 1;
            continue;
        }
        let end = pos + 1 + (w & MASK) as usize;
        let bw = code[pos + 1];
        if bw != NO_BOX {
            let [t, fb] = &boxes[(bw & !NODE) as usize];
            if !crosses(t) || !crosses(fb) {
                pos = end;
                continue;
            }
        }
        pos += 2;
    }
}

/// The complement of a program (De Morgan: AND and OR swap, every leaf
/// flips its side). Node boxes do not carry over: box words are `NO_BOX`.
pub fn negate(code: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(code.len());
    let mut pos = 0;
    while pos < code.len() {
        let w = code[pos];
        if w & NODE == 0 {
            out.push(w ^ POS);
            pos += 1;
        } else {
            out.push(w ^ OR);
            out.push(NO_BOX);
            pos += 2;
        }
    }
    out
}

/// Visit every leaf `(surface, want_pos)` of a program. (Header and box
/// words both have bit 31 set.)
pub fn leaves(code: &[u32]) -> impl Iterator<Item = (u32, bool)> + '_ {
    code.iter()
        .filter(|w| *w & NODE == 0)
        .map(|w| (w & MASK, w & POS != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Expr {
        let m = migjorn::parse(&format!("t\n1 0 {s} imp:n=1\n\n1 so 1\n\n"));
        m.cell(1).unwrap().geometry_expr().unwrap()
    }

    struct R;
    impl Resolve for R {
        fn surface(&self, id: i64) -> Result<SurfSlot, String> {
            if id == 9 {
                Ok(SurfSlot::Macro {
                    first: 90,
                    count: 3,
                })
            } else {
                Ok(SurfSlot::Single(id as u32))
            }
        }
        fn cell_expr(&self, id: i64) -> Result<(Expr, Option<Affine>), String> {
            if id == 7 {
                Ok((parse("-1 2"), None))
            } else {
                Err("no".into())
            }
        }
    }

    fn compile(s: &str) -> Vec<u32> {
        let mut out = Vec::new();
        lower(&parse(s), &R, None, &mut out, &mut Vec::new()).unwrap();
        out
    }

    /// Evaluate with surfaces whose positive side is given by `pos[id]`.
    fn run(code: &[u32], pos: &[u32]) -> Tri {
        eval(code, [0.0; 3], None, &mut |s, want_pos| {
            if pos.contains(&s) == want_pos {
                Tri::True
            } else {
                Tri::False
            }
        })
    }

    #[test]
    fn precedence() {
        let c = compile("-1 2 : 3");
        // (-1 AND 2) OR 3
        assert_eq!(run(&c, &[2]), Tri::True);
        assert_eq!(run(&c, &[3, 1]), Tri::True);
        assert_eq!(run(&c, &[1]), Tri::False);
        assert_eq!(c[0] & (NODE | OR), NODE | OR);
    }

    #[test]
    fn complements_and_macrobodies() {
        // #7 = NOT(-1 2) = 1 : -2
        let c = compile("#7");
        assert_eq!(run(&c, &[1, 2]), Tri::True);
        assert_eq!(run(&c, &[2]), Tri::False);
        let c = compile("#(-1 2) 4");
        assert_eq!(run(&c, &[1, 4]), Tri::True);
        assert_eq!(run(&c, &[2, 4]), Tri::False);
        // -9: inside all facets; 9.2: positive side of facet 2 only.
        let c = compile("-9");
        assert_eq!(run(&c, &[]), Tri::True);
        assert_eq!(run(&c, &[91]), Tri::False);
        let c = compile("9.2");
        assert_eq!(c, vec![91 | POS]);
    }

    #[test]
    fn flattening() {
        let c = compile("(-1 (2 3)) -4");
        assert_eq!(c.len(), 6);
        let c = compile("((1 : 2) : 3)");
        assert_eq!(c.len(), 5);
    }

    /// Surface `s` is the plane `x[s % 3] = s / 3` (positive side above).
    fn plane_side(s: u32, x: f64) -> Tri {
        let d = x - (s / 3) as f64;
        if d.abs() < 1e-9 {
            Tri::Unknown
        } else if d > 0.0 {
            Tri::True
        } else {
            Tri::False
        }
    }

    #[test]
    fn specialized_programs_agree_in_their_box() {
        let progs = [
            "-1 2 : 3 -4",
            "(1 : -2) (3 : 4 : -5) -7",
            "#(1 -2 (3 : -9)) 4",
            "-9 : 1 2 3",
            "((1 2) : (3 4)) ((-5 6) : -7)",
        ];
        let mut r = 0x2545f4914f6cdd1du64;
        let mut f = move || {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            (r >> 11) as f64 / (1u64 << 53) as f64
        };
        let (mut constant, mut shorter) = (0, 0);
        for src in progs {
            let code = compile(src);
            for _ in 0..300 {
                let lo = [0, 1, 2].map(|_| -1.0 + 33.0 * f());
                let hi = [0, 1, 2].map(|i| lo[i] + 12.0 * f() * f());
                let b = Aabb { lo, hi };
                let over = |s: u32, want_pos: bool| {
                    let a = (s % 3) as usize;
                    let (x, y) = (plane_side(s, b.lo[a]), plane_side(s, b.hi[a]));
                    let v = if x == y && x != Tri::Unknown {
                        x
                    } else {
                        Tri::Unknown
                    };
                    match (v, want_pos) {
                        (Tri::Unknown, _) => Tri::Unknown,
                        (Tri::True, true) | (Tri::False, false) => Tri::True,
                        _ => Tri::False,
                    }
                };
                let mut res = Vec::new();
                let c = specialize(&code, &b, &[], &mut |s, w| over(s, w), &mut res);
                constant += c.is_some() as usize;
                shorter += (c.is_none() && res.len() < code.len()) as usize;
                if c.is_none() {
                    assert_eq!(program_len(&res, 0), res.len(), "{src}");
                }
                for _ in 0..40 {
                    let p = [0, 1, 2].map(|i| b.lo[i] + f() * (b.hi[i] - b.lo[i]));
                    let at = |s: u32, want_pos: bool| match (
                        plane_side(s, p[(s % 3) as usize]),
                        want_pos,
                    ) {
                        (Tri::Unknown, _) => Tri::Unknown,
                        (Tri::True, true) | (Tri::False, false) => Tri::True,
                        _ => Tri::False,
                    };
                    let full = eval(&code, p, None, &mut |s, w| at(s, w));
                    let part = match c {
                        Some(v) => v,
                        None => eval(&res, p, None, &mut |s, w| at(s, w)),
                    };
                    assert_eq!(full, part, "{src} in {b:?} at {p:?}");
                }
            }
        }
        assert!(constant > 100 && shorter > 50, "{constant} {shorter}");
    }

    #[test]
    fn negation_is_the_complement() {
        let c = compile("(-1 2) : (3 -9)");
        let n = negate(&c);
        for mask in 0..16u32 {
            let pos: Vec<u32> = [1, 2, 3, 9]
                .iter()
                .enumerate()
                .filter(|(i, _)| mask >> i & 1 == 1)
                .map(|(_, &s)| s)
                .collect();
            let (a, b) = (run(&c, &pos), run(&n, &pos));
            assert_ne!(a, b, "{pos:?}");
        }
        // Unknown stays unknown.
        let v = eval(&n, [0.0; 3], None, &mut |_, _| Tri::Unknown);
        assert_eq!(v, Tri::Unknown);
    }

    #[test]
    fn kleene() {
        let c = compile("-1 2");
        let v = eval(&c, [0.0; 3], None, &mut |s, _| {
            if s == 1 {
                Tri::Unknown
            } else {
                Tri::False
            }
        });
        assert_eq!(v, Tri::False);
        let v = eval(&c, [0.0; 3], None, &mut |s, _| {
            if s == 1 {
                Tri::Unknown
            } else {
                Tri::True
            }
        });
        assert_eq!(v, Tri::Unknown);
    }
}
