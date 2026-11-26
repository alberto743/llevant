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

/// A cut that the algebra shortens: cell 1 cut by cell 2 is `1 -2 3`.
const UNION: &str = "union
1 0 (-1:-2) 3 imp:n=1
2 0 -1 3 imp:n=1
3 0 #2 #1 -4 imp:n=0
4 0 4 imp:n=0

1 so 5
2 so 9
3 px 0
4 so 50

";

/// The viewer's two steps on a cut that the second shortens: the
/// simplification is written, and labels every point as the expansion.
#[test]
fn the_second_step_shortens_and_changes_no_point() {
    let pts = grid();
    let s = Source::from_str(UNION);
    let e = s.cut_expand(1, 2, &Limits::default()).unwrap();
    s.apply_expand(&e).unwrap();
    let expanded = who(&s.to_source(), &pts);
    let r = s.cell_simplify(1, &Limits::default()).unwrap();
    assert!(r.simplified, "{r:?}");
    s.apply_simplified(&r).unwrap();
    assert_ne!(
        s.cell(1).unwrap().text,
        e.new_card,
        "a shorter card written"
    );
    assert_eq!(who(&s.to_source(), &pts), expanded, "{}", r.new_card);
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

/// A cell with a union that the algebra does shorten (`(-1:-2) 3` outside
/// `-1 3` is `-2 3 1`): the model still labels every point as with the
/// plain cut.
#[test]
fn a_shortened_cut_labels_every_point_as_the_plain_one() {
    let pts = grid();
    let a = Source::from_str(UNION);
    let b = Source::from_str(UNION);
    let plain = a.cut_plan(1, 2, false).unwrap();
    let simple = b.cut_plan(1, 2, true).unwrap();
    a.apply_cut(&plain).unwrap();
    b.apply_cut(&simple).unwrap();
    // The shortening is the point: a plain card compared with itself would
    // pass this test without the algebra having done anything.
    assert!(simple.result.simplified, "{:?}", simple.result);
    assert!(simple.result.terms_after < plain.result.terms_after);
    assert_eq!(who(&a.to_source(), &pts), who(&b.to_source(), &pts));
}

/// The simplification is proven on surface senses true or false: off the
/// surfaces it labels every point as the longer form, on one it may not.
/// `1 (-1:2) -3` and `1 2 -3` are the same region, but at (0, -1, 0), on
/// surface 1 where `2` is false, the first is on-surface (unknown) and the
/// second is outside (`docs/contracts.md`, "Cut").
#[test]
fn a_simplified_card_may_label_a_point_on_a_surface_otherwise() {
    let deck = |g: &str| {
        format!(
            "t\n1 0 {g} imp:n=1\n2 0 #1 -9 imp:n=1\n3 0 9 imp:n=0\n\n\
             1 px 0\n2 py 0\n3 so 10\n9 so 50\n\n"
        )
    };
    let label = |g: &str, p: Vec3| {
        let m = Model::from_text(&deck(g), &Options::default());
        let found = reference::classify(&m, &[p], Depth::All);
        found.labels[found.ids[0] as usize].status
    };
    let on = [0.0, -1.0, 0.0];
    assert_eq!(label("1 (-1:2) -3", on), Status::OnSurface);
    assert_ne!(label("1 2 -3", on), Status::OnSurface);
    // Off the surfaces, alike.
    for p in [[1.0, 1.0, 0.0], [1.0, -1.0, 0.0], [-1.0, 1.0, 0.0]] {
        assert_eq!(label("1 (-1:2) -3", p), label("1 2 -3", p), "{p:?}");
    }
}

/// Every pair of overlapping cells of the overlap zoo
/// (`tools/mcnp-oracle/models/overlaps.i`), cut as the viewer cuts (`#B`
/// expanded, then simplified) or refused with its reason: each card written
/// fits MCNP's columns, and where the two overlapped before (sampled in the
/// frame the cells are placed in), the cell not cut is left alone after the
/// expansion and after the simplification.
#[test]
fn the_overlap_zoo_is_cut_or_refused() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models/overlaps.i"
    );
    let text = std::fs::read_to_string(path).unwrap();
    // (keep, cut, refused with): the clusters of the zoo, both ways round.
    let cases: &[(i64, i64, Option<&str>)] = &[
        (101, 102, None),
        (102, 101, None),
        (111, 112, None),
        (112, 111, None),
        (121, 122, None),
        (131, 132, None),
        (132, 133, None),
        (133, 131, None),
        (141, 142, None),
        (142, 141, None),
        (152, 153, None),
        (153, 152, None),
        (161, 162, None),
        (162, 161, None),
        (171, 172, None),
        (172, 173, None),
        (173, 174, None),
        (175, 172, None),
        (181, 182, Some("TRCL")),
        (182, 181, Some("LIKE")),
        (183, 182, Some("LIKE")),
        (184, 185, None),
        (185, 184, None),
        (191, 192, None),
        (185, 191, Some("universes")),
        (201, 202, None),
        (202, 201, None),
        (211, 212, None),
        (212, 211, None),
        (222, 223, Some("complement each other")),
        (223, 222, Some("already")),
        (224, 221, None),
    ];
    let fits = |card: &str| card.lines().all(|l| l.chars().count() <= 128);
    // Where cells meet: their boxes, in the frame their universe is placed
    // in (universe 1 fills cell 184 moved by (800, 40, 0)).
    let shift = |n: i64| -> Vec3 {
        if (191..=193).contains(&n) {
            [800.0, 40.0, 0.0]
        } else {
            [0.0; 3]
        }
    };
    let overlap_points = |m: &Model, keep: i64, cut: i64| -> Vec<Vec3> {
        let (a, b) = (m.cell_index(keep).unwrap(), m.cell_index(cut).unwrap());
        let (ba, bb, d) = (m.cell(a).bbox, m.cell(b).bbox, shift(keep));
        let lo: Vec3 = std::array::from_fn(|i| ba.lo[i].max(bb.lo[i]) + d[i]);
        let hi: Vec3 = std::array::from_fn(|i| ba.hi[i].min(bb.hi[i]) + d[i]);
        let n = 24;
        let mut pts = Vec::new();
        for i in 0..n {
            for j in 0..n {
                for k in 0..n {
                    let f = |t: usize, d: usize| {
                        lo[d] + (t as f64 + 0.371) / n as f64 * (hi[d] - lo[d])
                    };
                    pts.push([f(i, 0), f(j, 1), f(k, 2)]);
                }
            }
        }
        let found = reference::classify(m, &pts, Depth::All);
        pts.iter()
            .zip(&found.ids)
            .filter(|&(_, &id)| {
                let l = &found.labels[id as usize];
                l.status == Status::Overlap && l.involved.contains(&a) && l.involved.contains(&b)
            })
            .map(|(p, _)| *p)
            .collect()
    };
    // At the points where they overlapped: B is still there, A no longer
    // (B, or a cell filled with what holds it, is on the point's path, or
    // is one of the cells of a problem there).
    let left_to_b = |s: &Source, keep: i64, cut: i64, pts: &[Vec3], step: &str| {
        let m = Model::from_text(&s.to_source(), &Options::default());
        let (a, b) = (m.cell_index(keep).unwrap(), m.cell_index(cut).unwrap());
        let found = reference::classify(&m, pts, Depth::All);
        for (p, &id) in pts.iter().zip(&found.ids) {
            let l = &found.labels[id as usize];
            let cells: Vec<u32> = match l.status {
                // B may be filled: the cell that holds the point is below it.
                Status::Inside => l.path.iter().map(|s| s.cell).collect(),
                _ => l
                    .path
                    .iter()
                    .map(|s| s.cell)
                    .chain(l.involved.iter().copied())
                    .collect(),
            };
            assert!(
                cells.contains(&b) && !cells.contains(&a),
                "{keep} #{cut} ({step}): at {p:?}, {:?} {:?}\n{}",
                l.status,
                cells,
                s.cell(keep).unwrap().text
            );
        }
    };
    let before = Model::from_text(&text, &Options::default());
    for &(keep, cut, refused) in cases {
        let s = Source::from_str(&text);
        let e = s.cut_expand(keep, cut, &Limits::default());
        let e = match (refused, e) {
            (Some(why), Err(err)) => {
                assert!(err.contains(why), "{keep} #{cut}: {err}");
                continue;
            }
            (Some(why), Ok(_)) => panic!("{keep} #{cut}: not refused ({why})"),
            (None, Err(err)) => panic!("{keep} #{cut}: {err}"),
            (None, Ok(e)) => e,
        };
        let pts = overlap_points(&before, keep, cut);
        assert!(!pts.is_empty(), "{keep} #{cut}: they do not overlap before");
        if e.empty {
            continue;
        }
        assert!(fits(&e.new_card), "{keep} #{cut}:\n{}", e.new_card);
        s.apply_expand(&e).unwrap();
        left_to_b(&s, keep, cut, &pts, "expanded");
        let r = s.cell_simplify(keep, &Limits::default()).unwrap();
        if r.simplified {
            assert!(fits(&r.new_card), "{keep} #{cut}:\n{}", r.new_card);
            s.apply_simplified(&r).unwrap();
            left_to_b(&s, keep, cut, &pts, "simplified");
        }
    }
}
