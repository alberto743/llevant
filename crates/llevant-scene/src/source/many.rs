//! Many cards edited as one (decision 0012, phase 8): cards written
//! together, all or none, and numbers changed with every reference to them
//! ([`Source::renumber`]). Each gives the cards it changed, before and
//! after, so that one Undo takes it all back.

use std::collections::HashMap;

use rayon::prelude::*;

pub use migjorn::IdKind;

use super::{find, set_text, CardKind, CardLines, Edited, Source, MAX_COLUMNS};

/// A renumbering made by [`Source::renumber`].
#[derive(Clone, Debug)]
pub struct Renumbered {
    /// Every card it changed (definitions and the cards that refer to
    /// them), in the input's order.
    pub cards: Vec<Edited>,
    /// References it could not follow, left as written (`FM4: an id
    /// implied by 3M`).
    pub unfollowed: Vec<String>,
}

/// The highest number MCNP takes for a cell, surface, material, transform
/// or universe.
pub const HIGHEST_NUMBER: i64 = 99_999_999;

impl Source {
    /// Write each card as given (as [`Source::edit`] does one), all or
    /// none: on an error nothing has changed.
    pub fn edit_many(&self, cards: &[(CardKind, i64, String)]) -> Result<Vec<Edited>, String> {
        let mut slots = Vec::with_capacity(cards.len());
        {
            let tree = self.tree();
            for (kind, id, text) in cards {
                let what = format!("{} {id}", kind.name());
                if let Some((n, line)) = text
                    .lines()
                    .enumerate()
                    .find(|(_, l)| l.chars().count() > MAX_COLUMNS)
                {
                    return Err(format!(
                        "{what}: line {} has {} columns, over MCNP's {MAX_COLUMNS}",
                        n + 1,
                        line.chars().count()
                    ));
                }
                let (slot, old) = find(&tree, *kind, *id)
                    .ok_or_else(|| format!("{what} is not in the input (in a READ file?)"))?;
                let mut text = text.clone();
                if !text.ends_with('\n') {
                    text.push_str(if old.ends_with("\r\n") { "\r\n" } else { "\n" });
                }
                slots.push(Edited {
                    kind: *kind,
                    slot,
                    old,
                    new: text,
                    added: None,
                });
            }
        }
        self.write_all(&slots, false)?;
        Ok(slots)
    }

    /// Take edits back: each card written as it was, the last first.
    pub fn restore_all(&self, edits: &[Edited]) -> Result<(), String> {
        self.write_all(edits, true)
    }

    /// Make edits taken back again, in order.
    pub fn redo_all(&self, edits: &[Edited]) -> Result<(), String> {
        self.write_all(edits, false)
    }

    /// Write each card of `edits` as after (or, `back`, as before, the last
    /// first), under one lock, the lines counted again once; on an error
    /// the cards written are put back.
    fn write_all(&self, edits: &[Edited], back: bool) -> Result<(), String> {
        let order: Vec<&Edited> = if back {
            edits.iter().rev().collect()
        } else {
            edits.iter().collect()
        };
        let text = |e: &Edited, undo: bool| if back != undo { &e.old } else { &e.new }.clone();
        let mut tree = self.tree.write().unwrap_or_else(|e| e.into_inner());
        // A card added: taken out (found by its number), or added again.
        let write = |tree: &mut migjorn::Model, e: &Edited, undo: bool| -> Result<(), String> {
            match e.added {
                Some(id) if back != undo => {
                    let (slot, _) = find(tree, e.kind, id)
                        .ok_or_else(|| "the card added is gone".to_string())?;
                    tree.remove_card(slot);
                    Ok(())
                }
                Some(_) => add(tree, e.kind, &e.new).map(|_| ()),
                None => set_text(tree, e.kind, e.slot, &text(e, undo)),
            }
        };
        for (k, e) in order.iter().enumerate() {
            if let Err(err) = write(&mut tree, e, false) {
                for d in order[..k].iter().rev() {
                    if let Err(e2) = write(&mut tree, d, true) {
                        log::warn!("could not put a card back: {e2}");
                    }
                }
                return Err(format!("{} (slot {}): {err}", e.kind.name(), e.slot));
            }
        }
        let lines = CardLines::of(&tree);
        drop(tree);
        *self.lines.write().unwrap_or_else(|e| e.into_inner()) = lines;
        Ok(())
    }

    /// Give numbers of `kind` new ones, `(old, new)`, in every card that
    /// uses them (the definitions, cell geometry, `#n`, `LIKE n BUT`,
    /// `U`/`FILL`, tallies… as migjorn follows them). An error, changing
    /// nothing, when an old number is not defined, a new one is in use
    /// (renumbering into numbers being freed is not taken, so that the
    /// edit can be taken back card by card), or out of MCNP's range. For
    /// universes (which no card defines) the caller checks which are in
    /// use.
    pub fn renumber(&self, kind: IdKind, pairs: &[(i64, i64)]) -> Result<Renumbered, String> {
        let name = match kind {
            IdKind::Cell => "cell",
            IdKind::Surface => "surface",
            IdKind::Material => "material",
            IdKind::Transform => "transform",
            IdKind::Universe => "universe",
            IdKind::Tally => "tally",
        };
        let card_kind = match kind {
            IdKind::Cell => Some(CardKind::Cell),
            IdKind::Surface => Some(CardKind::Surface),
            IdKind::Material => Some(CardKind::Material),
            IdKind::Transform => Some(CardKind::Transform),
            IdKind::Universe => None,
            IdKind::Tally => return Err("tallies are not renumbered here".into()),
        };
        let map: HashMap<i64, i64> = pairs.iter().copied().filter(|(a, b)| a != b).collect();
        if map.len() != pairs.iter().filter(|(a, b)| a != b).count() {
            return Err(format!("a {name} is given two new numbers"));
        }
        if map.is_empty() {
            return Ok(Renumbered {
                cards: Vec::new(),
                unfollowed: Vec::new(),
            });
        }
        let mut targets: Vec<i64> = map.values().copied().collect();
        targets.sort_unstable();
        if let Some(w) = targets.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("two {name}s would be numbered {}", w[0]));
        }
        // Material 0 is void, not a number a material can take: its cells
        // would keep their density, and `m1` would become `m0` (MCNP's card
        // of defaults for every material).
        if kind == IdKind::Material && targets.contains(&0) {
            return Err(
                "material 0 is void: a material cannot be numbered 0 (to make cells void, \
                 edit them)"
                    .into(),
            );
        }
        if let Some(&n) = targets
            .iter()
            .find(|&&n| !(1..=HIGHEST_NUMBER).contains(&n))
        {
            return Err(format!(
                "{name} number {n} is out of MCNP's range (1 to {HIGHEST_NUMBER})"
            ));
        }
        if kind == IdKind::Universe && map.contains_key(&0) {
            return Err("universe 0 is the real world: it keeps its number".into());
        }
        let mut tree = self.tree.write().unwrap_or_else(|e| e.into_inner());
        if let Some(ck) = card_kind {
            let mut olds: Vec<i64> = map.keys().copied().collect();
            olds.sort_unstable();
            if let Some(o) = olds.iter().find(|&&o| find(&tree, ck, o).is_none()) {
                return Err(format!("{name} {o} is not in the input (in a READ file?)"));
            }
            if let Some(t) = targets.iter().find(|&&t| find(&tree, ck, t).is_some()) {
                return Err(format!("{name} {t} is in use"));
            }
        }
        // The cards that can change (a number of the map written in them),
        // as they are: what is taken back.
        let before: Vec<(u32, String)> = {
            let cst = tree.cst();
            let cards: Vec<_> = cst.cards().collect();
            cards
                .par_iter()
                .filter(|c| mentions(c.text(), &map))
                .map(|c| (c.slot(), c.text().to_string()))
                .collect()
        };
        let unfollowed = tree.renumber(kind, |n| map.get(&n).copied().unwrap_or(n));
        let mut cards = Vec::new();
        let mut failed = None;
        for (slot, old) in before {
            let Some(now) = tree.cst().card(slot).map(|c| c.text().to_string()) else {
                continue;
            };
            if now == old {
                continue;
            }
            let kind = kind_at(&tree, slot);
            let mut new = now.clone();
            if new.lines().any(|l| l.chars().count() > MAX_COLUMNS) {
                new = super::fields::wrap_long_lines(&new);
                if let Err(e) = set_text(&mut tree, kind, slot, &new) {
                    failed = Some(format!("{} (slot {slot}): {e}", kind.name()));
                }
            }
            cards.push(Edited {
                kind,
                slot,
                old,
                new,
                added: None,
            });
        }
        if let Some(e) = failed {
            for c in cards.iter().rev() {
                let _ = set_text(&mut tree, c.kind, c.slot, &c.old);
            }
            return Err(e);
        }
        let lines = CardLines::of(&tree);
        drop(tree);
        *self.lines.write().unwrap_or_else(|e| e.into_inner()) = lines;
        Ok(Renumbered {
            cards,
            unfollowed: unfollowed.iter().map(|u| u.to_string()).collect(),
        })
    }
}

impl Source {
    /// Add a card of `kind` (a cell, surface, material or transform) at the
    /// end of its block, as written. For a cell, each data card that gives
    /// a value per cell (`IMP:N`, `VOL`, `U`…) gets one more (`1` for an
    /// importance, `j` for the others), so that the cells keep theirs. The
    /// cards changed, the one added last; an error, changing nothing, when
    /// its number is in use, it does not read, or the values per cell are
    /// in vertical format.
    pub fn add_card(&self, kind: CardKind, text: &str) -> Result<Vec<Edited>, String> {
        if let Some((n, line)) = text
            .lines()
            .enumerate()
            .find(|(_, l)| l.chars().count() > MAX_COLUMNS)
        {
            return Err(format!(
                "line {} has {} columns, over MCNP's {MAX_COLUMNS}",
                n + 1,
                line.chars().count()
            ));
        }
        let mut text = text.trim_end().to_string();
        text.push('\n');
        let mut tree = self.tree.write().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        // The values per cell, one more each.
        if kind == CardKind::Cell {
            let cards: Vec<(u32, Option<String>, String)> = tree
                .per_cell_cards()
                .iter()
                .map(|d| {
                    (
                        d.slot(),
                        d.mnemonic()
                            .map(|m| m.trim_start_matches('*').to_ascii_lowercase()),
                        d.text().to_string(),
                    )
                })
                .collect();
            for (slot, name, old) in cards {
                let Some(name) = name else {
                    return Err(
                        "the input gives values per cell in vertical format: add the new cell's by hand"
                            .into(),
                    );
                };
                let entry = if name.starts_with("imp") { "1" } else { "j" };
                let new = super::fields::wrap_long_lines(&with_entry(&old, entry));
                out.push(Edited {
                    kind: CardKind::Data,
                    slot,
                    old,
                    new,
                    added: None,
                });
            }
        }
        let slot = add(&mut tree, kind, &text)?;
        let id = match kind {
            CardKind::Cell => tree.cell_at(slot).and_then(|c| c.id()),
            CardKind::Surface => tree.surface_at(slot).and_then(|c| c.id()),
            CardKind::Material => tree.material_at(slot).and_then(|c| c.id()),
            CardKind::Transform => tree.transform_at(slot).and_then(|c| c.id()),
            CardKind::Data => None,
        };
        // Taking the card out again drops an entry from the values per
        // cell (migjorn's remove_card): they are written back as they were.
        let take_out = |tree: &mut migjorn::Model, out: &[Edited]| {
            tree.remove_card(slot);
            for d in out.iter().rev() {
                let _ = set_text(tree, d.kind, d.slot, &d.old);
            }
        };
        let taken = id
            .and_then(|id| find(&tree, kind, id))
            .is_some_and(|(s, _)| s != slot);
        if id.is_none() || taken {
            take_out(&mut tree, &out);
            return Err(match id {
                Some(id) => format!("{} {id} is in use", kind.name()),
                None => format!("it does not read as a {} card with a number", kind.name()),
            });
        }
        // The values per cell, written now that the cell reads.
        for e in &out {
            if let Err(err) = set_text(&mut tree, e.kind, e.slot, &e.new) {
                take_out(&mut tree, &out);
                return Err(format!("a card of values per cell: {err}"));
            }
        }
        let new = tree
            .cst()
            .card(slot)
            .map(|c| c.text().to_string())
            .unwrap_or(text);
        out.push(Edited {
            kind,
            slot,
            old: String::new(),
            new,
            added: id,
        });
        let lines = CardLines::of(&tree);
        drop(tree);
        *self.lines.write().unwrap_or_else(|e| e.into_inner()) = lines;
        Ok(out)
    }

    /// The next free number of `kind` above the highest in use (a cell,
    /// surface, material or transform).
    pub fn next_free(&self, kind: CardKind) -> i64 {
        let tree = self.tree();
        let top = match kind {
            CardKind::Cell => tree.cells().filter_map(|c| c.id()).max(),
            CardKind::Surface => tree.surfaces().filter_map(|c| c.id()).max(),
            CardKind::Material => tree.materials().filter_map(|c| c.id()).max(),
            CardKind::Transform => tree.transforms().filter_map(|c| c.id()).max(),
            CardKind::Data => None,
        };
        top.unwrap_or(0) + 1
    }
}

/// Add a card of `kind` at the end of its block; its slot.
fn add(tree: &mut migjorn::Model, kind: CardKind, text: &str) -> Result<u32, String> {
    let r = match kind {
        CardKind::Cell => tree.add_cell(text).map(|c| c.slot()),
        CardKind::Surface => tree.add_surface(text).map(|c| c.slot()),
        CardKind::Material => tree.add_material(text).map(|c| c.slot()),
        CardKind::Transform => tree.add_transform(text).map(|c| c.slot()),
        CardKind::Data => tree.add_data_card(text).map(|c| c.slot()),
    };
    r.map_err(|e| e.to_string())
}

/// Card `text` with one more value, `entry`, at its end (before a `$`
/// comment on its last line).
fn with_entry(text: &str, entry: &str) -> String {
    let body = text.trim_end_matches(['\n', '\r']);
    let end = &text[body.len()..];
    let end = if end.is_empty() { "\n" } else { end };
    let start = body.rfind('\n').map_or(0, |k| k + 1);
    let last = &body[start..];
    let (code, note) = match last.find('$') {
        Some(k) => (last[..k].trim_end(), format!(" {}", &last[k..])),
        None => (last.trim_end(), String::new()),
    };
    format!("{}{code} {entry}{note}{end}", &body[..start])
}

/// Whether `text` has a run of digits that is a number of `map` (every
/// card a renumbering changes has).
fn mentions(text: &str, map: &HashMap<i64, i64>) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if text[start..i]
                .parse::<i64>()
                .is_ok_and(|n| map.contains_key(&n))
            {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// The kind of the card at `slot`.
fn kind_at(tree: &migjorn::Model, slot: u32) -> CardKind {
    if tree.cell_at(slot).is_some() {
        CardKind::Cell
    } else if tree.surface_at(slot).is_some() {
        CardKind::Surface
    } else if tree.material_at(slot).is_some() {
        CardKind::Material
    } else if tree.transform_at(slot).is_some() {
        CardKind::Transform
    } else {
        CardKind::Data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT: &str = "renumber
1 1 -2.7 -1 imp:n=1 $ the ball
2 0 1 -2 #1 imp:n=1
3 like 2 but imp:n=2
9 0 2 imp:n=0

1 so 5
2 so 50

m1 13027 1
f4:n 1 2
sd4 1 1
";

    #[test]
    fn numbers_changed_everywhere_and_taken_back() {
        let src = Source::from_str(INPUT);
        let r = src.renumber(IdKind::Cell, &[(1, 101), (2, 102)]).unwrap();
        let text = src.to_source();
        assert!(text.contains("101 1 -2.7 -1 imp:n=1 $ the ball"), "{text}");
        assert!(text.contains("102 0 1 -2 #101 imp:n=1"), "{text}");
        assert!(text.contains("3 like 102 but"), "{text}");
        assert!(text.contains("f4:n 101 102"), "{text}");
        assert!(src.cell(101).is_some() && src.cell(1).is_none());
        let kinds: Vec<CardKind> = r.cards.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                CardKind::Cell,
                CardKind::Cell,
                CardKind::Cell,
                CardKind::Data
            ]
        );
        src.restore_all(&r.cards).unwrap();
        assert_eq!(src.to_source(), INPUT);
        assert!(src.cell(1).is_some() && src.cell(101).is_none());
        src.redo_all(&r.cards).unwrap();
        assert!(src.to_source().contains("102 0 1 -2 #101"));

        // Surfaces: the senses kept.
        let src = Source::from_str(INPUT);
        src.renumber(IdKind::Surface, &[(1, 7)]).unwrap();
        let text = src.to_source();
        assert!(text.contains("1 1 -2.7 -7 imp:n=1"), "{text}");
        assert!(text.contains("2 0 7 -2 #1"), "{text}");
        assert!(text.contains("\n7 so 5"), "{text}");

        // Materials and universes.
        let src = Source::from_str(INPUT);
        src.renumber(IdKind::Material, &[(1, 5)]).unwrap();
        let text = src.to_source();
        assert!(
            text.contains("1 5 -2.7 -1") && text.contains("m5 13027 1"),
            "{text}"
        );
        let src = Source::from_str(
            "u\n1 0 -1 fill=4 imp:n=1\n2 0 -1 u=4 imp:n=1\n9 0 1 imp:n=0\n\n1 so 5\n\n",
        );
        let r = src.renumber(IdKind::Universe, &[(4, 40)]).unwrap();
        let text = src.to_source();
        assert!(text.contains("fill=40") && text.contains("u=40"), "{text}");
        assert_eq!(r.cards.len(), 2);
    }

    #[test]
    fn cards_added_and_taken_out() {
        let input = "added
1 0 -1
2 0 1 -2
9 0 2

1 so 5
2 so 50

imp:n 1 1 0 $ last is outside
vol 4j
m1 1001 1
";
        let src = Source::from_str(input);
        assert_eq!(src.next_free(CardKind::Cell), 10);
        assert_eq!(src.next_free(CardKind::Material), 2);
        let e = src.add_card(CardKind::Cell, "10 0 -1 (2").unwrap_err();
        assert!(!e.is_empty(), "{e}");
        assert_eq!(src.to_source(), input, "nothing changed");
        assert!(src
            .add_card(CardKind::Cell, "2 0 -1")
            .unwrap_err()
            .contains("in use"));
        assert_eq!(src.to_source(), input, "nothing changed");
        let done = src.add_card(CardKind::Cell, "10 0 -2 1 $ new").unwrap();
        let text = src.to_source();
        assert!(text.contains("9 0 2\n10 0 -2 1 $ new\n"), "{text}");
        assert!(text.contains("imp:n 1 1 0 1 $ last is outside\n"), "{text}");
        assert!(text.contains("vol 4j j\n"), "{text}");
        assert_eq!(done.len(), 3);
        assert_eq!(done[2].added, Some(10));
        src.restore_all(&done).unwrap();
        assert_eq!(src.to_source(), input, "taken out");
        src.redo_all(&done).unwrap();
        assert!(src.cell(10).is_some());
        src.restore_all(&done).unwrap();
        assert_eq!(src.to_source(), input, "and out again");

        let done = src.add_card(CardKind::Surface, "3 pz 7").unwrap();
        assert!(src.to_source().contains("2 so 50\n3 pz 7\n"));
        src.restore_all(&done).unwrap();
        let done = src.add_card(CardKind::Material, "m2 8016 1").unwrap();
        assert!(src.to_source().contains("m2 8016 1\n"));
        src.restore_all(&done).unwrap();
        assert_eq!(src.to_source(), input);
    }

    #[test]
    fn renumbering_refused() {
        let src = Source::from_str(INPUT);
        let refused = |kind, pairs: &[(i64, i64)]| src.renumber(kind, pairs).unwrap_err();
        assert!(refused(IdKind::Cell, &[(1, 2)]).contains("in use"));
        assert!(refused(IdKind::Cell, &[(5, 6)]).contains("not in the input"));
        assert!(refused(IdKind::Cell, &[(1, 50), (2, 50)]).contains("two cells"));
        assert!(refused(IdKind::Cell, &[(1, 0)]).contains("range"));
        assert!(refused(IdKind::Universe, &[(0, 4)]).contains("real world"));
        // Material 0 is void: `1 1 -2.7` would become `1 0 -2.7`, `m1` `m0`.
        assert!(refused(IdKind::Material, &[(1, 0)]).contains("void"));
        assert_eq!(src.to_source(), INPUT, "nothing changed");
        assert!(src
            .renumber(IdKind::Cell, &[(1, 1)])
            .unwrap()
            .cards
            .is_empty());
    }

    #[test]
    fn many_cards_all_or_none() {
        let src = Source::from_str(INPUT);
        let done = src
            .edit_many(&[
                (CardKind::Cell, 1, "1 1 -3 -1 imp:n=1 $ the ball".into()),
                (CardKind::Surface, 2, "2 so 60".into()),
            ])
            .unwrap();
        assert_eq!(done.len(), 2);
        assert!(src.to_source().contains("2 so 60\n"));
        src.restore_all(&done).unwrap();
        assert_eq!(src.to_source(), INPUT);
        let e = src
            .edit_many(&[
                (CardKind::Surface, 2, "2 so 60".into()),
                (CardKind::Surface, 1, "not a surface".into()),
            ])
            .unwrap_err();
        assert!(e.contains("surface"), "{e}");
        assert_eq!(src.to_source(), INPUT, "the first put back");
        assert!(src
            .edit_many(&[(CardKind::Cell, 77, "77 0 1".into())])
            .is_err());
    }
}
