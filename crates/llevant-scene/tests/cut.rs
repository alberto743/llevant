// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! Cutting a cell out of another fixes an overlap and changes nothing else:
//! the decks are compiled before and after and classified by brute force.

use llevant::{reference, Depth, Model, Options, Status, Vec3};
use llevant_scene::source::{Limits, Source};

/// Two spheres that overlap, a void around them.
const OVERLAP: &str = "overlap
1 0 -1 imp:n=1
2 0 -2 imp:n=1
3 0 1 2 -3 imp:n=0
4 0 3 imp:n=0

1 so 5
2 sx 4 5
3 so 50

";

fn grid() -> Vec<Vec3> {
    let mut pts = Vec::new();
    for i in 0..40 {
        for j in 0..24 {
            for k in 0..24 {
                // Off the symmetric grid, so no point is on a surface.
                pts.push([
                    -12.0 + i as f64 * 0.6173 + 0.013,
                    -7.0 + j as f64 * 0.6071 + 0.007,
                    -7.0 + k as f64 * 0.5987 + 0.003,
                ]);
            }
        }
    }
    pts
}

/// The cell number holding each point, or the status if it is not in one.
fn who(text: &str, pts: &[Vec3]) -> Vec<Result<i64, Status>> {
    let m = Model::from_text(text, &Options::default());
    let found = reference::classify(&m, pts, Depth::All);
    found
        .ids
        .iter()
        .map(|&id| {
            let l = &found.labels[id as usize];
            match l.status {
                Status::Inside => Ok(m.cell_id(l.path.last().unwrap().cell)),
                s => Err(s),
            }
        })
        .collect()
}

#[test]
fn cutting_removes_the_overlap_and_nothing_else() {
    let pts = grid();
    let before = who(OVERLAP, &pts);
    assert!(before.contains(&Err(Status::Overlap)));

    for (keep, cut) in [(2, 1), (1, 2)] {
        let s = Source::from_str(OVERLAP);
        let plan = s.cut_plan(keep, cut, true).unwrap();
        s.apply_cut(&plan).unwrap();
        let after = who(&s.to_source(), &pts);
        assert!(
            !after.contains(&Err(Status::Overlap)),
            "cut {cut} from {keep}: {}",
            plan.new_card
        );
        for (b, a) in before.iter().zip(&after) {
            match b {
                // Points of one cell only keep their cell; the overlap goes to
                // the cell that was not cut.
                Err(Status::Overlap) => assert_eq!(*a, Ok(cut)),
                other => assert_eq!(a, other),
            }
        }
    }
}

#[test]
fn simplified_and_plain_cuts_label_every_point_alike() {
    let pts = grid();
    let a = Source::from_str(OVERLAP);
    let b = Source::from_str(OVERLAP);
    let plain = a.cut_plan(2, 1, false).unwrap();
    let simple = b.cut_plan(2, 1, true).unwrap();
    a.apply_cut(&plain).unwrap();
    b.apply_cut(&simple).unwrap();
    assert_eq!(who(&a.to_source(), &pts), who(&b.to_source(), &pts));
}

/// The viewer's two steps (`#B` expanded, then simplified) fix the overlap
/// as the one-step cut does, and the simplification labels every point as
/// the expansion.
#[test]
fn the_two_steps_fix_the_overlap_and_nothing_else() {
    let pts = grid();
    let before = who(OVERLAP, &pts);
    for (keep, cut) in [(2, 1), (1, 2)] {
        let s = Source::from_str(OVERLAP);
        let e = s.cut_expand(keep, cut, &Limits::default()).unwrap();
        s.apply_expand(&e).unwrap();
        let expanded = who(&s.to_source(), &pts);
        for (b, a) in before.iter().zip(&expanded) {
            match b {
                Err(Status::Overlap) => assert_eq!(*a, Ok(cut)),
                other => assert_eq!(a, other),
            }
        }
        let r = s.cell_simplify(keep, &Limits::default()).unwrap();
        if r.simplified {
            s.apply_simplified(&r).unwrap();
        }
        assert_eq!(who(&s.to_source(), &pts), expanded, "{}", r.new_card);
    }
}

/// A cell that writes `#keep` takes in what is cut out of `keep`: here it
/// then overlaps the cut cell (what the viewer's verdict looks for).
#[test]
fn a_cell_writing_the_complement_of_the_cut_one_may_overlap_after() {
    const DECK: &str = "users
1 0 -1 imp:n=1
2 0 -2 imp:n=1
3 0 #1 (2:-1) -3 imp:n=1
4 0 3 imp:n=0

1 so 5
2 sx 4 5
3 so 50

";
    let pts = grid();
    let before = who(DECK, &pts);
    let s = Source::from_str(DECK);
    let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
    assert_eq!(e.users, vec![3]);
    s.apply_expand(&e).unwrap();
    let after = who(&s.to_source(), &pts);
    // The overlap of 1 and 2 is now one of 3 and 2, in the same place.
    assert_eq!(before, after);
    assert!(after.contains(&Err(Status::Overlap)));
}

/// A cell with a union and a nested complement: the algebra shortens it and
/// the model still labels every point as with the plain cut.
#[test]
fn a_shortened_cut_labels_every_point_as_the_plain_one() {
    const DECK: &str = "union
1 0 -1 : -2 imp:n=1
2 0 -1 -3 imp:n=1
3 0 #2 #1 -4 imp:n=0
4 0 4 imp:n=0

1 so 5
2 sx 4 5
3 px 0
4 so 50

";
    let pts = grid();
    let a = Source::from_str(DECK);
    let b = Source::from_str(DECK);
    let plain = a.cut_plan(1, 2, false).unwrap();
    let simple = b.cut_plan(1, 2, true).unwrap();
    a.apply_cut(&plain).unwrap();
    b.apply_cut(&simple).unwrap();
    assert!(simple.result.terms_after <= plain.result.terms_after);
    assert_eq!(who(&a.to_source(), &pts), who(&b.to_source(), &pts));
}
