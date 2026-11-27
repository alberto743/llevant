//! Cards of the input edited in the viewer: each edit to take back, and the
//! cards changed, to carry into the sources an assembled input is made from
//! (an edit to the assembled file is lost at the next assembly).
//!
//! The text as edited is the input's [`Source`](llevant_scene::source::Source)
//! (migjorn's tree, edited in place); an edit keeps only its cards, before
//! and after: one card, or many (cells edited together, a renumbering and
//! every card that refers to the numbers), taken back as one.

use llevant_scene::source::{lines, Edited};

/// One edit: what it was (`cell 5`, `material of 12 cells`), and each
/// card it changed, where it is, before and after.
#[derive(Clone, Debug)]
pub struct Change {
    pub what: String,
    pub cards: Vec<Edited>,
    /// What it could not do (references a renumbering left as written).
    pub notes: Vec<String>,
}

impl Change {
    /// An edit of one card, numbered `id` before.
    pub fn one(id: i64, edit: Edited) -> Change {
        Change {
            what: format!("{} {id}", edit.kind.name()),
            cards: vec![edit],
            notes: Vec::new(),
        }
    }

    /// Whether it can have changed the cells' labels: a cell card whose
    /// number or `$` comments differ, or one added or taken out.
    pub fn touches_labels(&self) -> bool {
        // The `$` comments of a card's text, and its first word (the number).
        let labels = |t: &str| -> (Option<String>, Vec<String>) {
            let number = t
                .lines()
                .find(|l| !lines::is_blank(l) && !lines::is_comment(l))
                .and_then(|l| l.split_whitespace().next())
                .map(str::to_string);
            let comments = t
                .lines()
                .filter_map(|l| l.split_once('$').map(|(_, c)| c.trim().to_string()))
                .collect();
            (number, comments)
        };
        self.cards.iter().any(|c| {
            c.kind == llevant_scene::source::CardKind::Cell && labels(&c.old) != labels(&c.new)
        })
    }

    /// Whether it changed a `TRn` card (which FMESH cards can use).
    pub fn touches_transforms(&self) -> bool {
        self.cards
            .iter()
            .any(|c| c.kind == llevant_scene::source::CardKind::Transform)
    }
}

/// The edits of the open input.
pub struct InputEdits {
    undo: Vec<Change>,
    /// `undo.len()` when the file last got the text (`None`: no state of
    /// the history is the file's any more, an edit was made after taking
    /// back edits that were saved).
    saved: Option<usize>,
}

impl Default for InputEdits {
    fn default() -> InputEdits {
        InputEdits {
            undo: Vec::new(),
            saved: Some(0),
        }
    }
}

impl InputEdits {
    /// An edit landed.
    pub fn push(&mut self, change: Change) {
        if self.saved.is_some_and(|s| s > self.undo.len()) {
            self.saved = None;
        }
        self.undo.push(change);
    }

    /// The last edit, if any.
    pub fn last(&self) -> Option<&Change> {
        self.undo.last()
    }

    /// The last edit was taken back.
    pub fn pop(&mut self) {
        self.undo.pop();
    }

    /// The file now holds the text as edited after `edits` edits.
    pub fn saved_at(&mut self, edits: usize) {
        self.saved = Some(edits);
    }

    /// The number of edits made.
    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// Whether the text differs from what was last saved.
    pub fn unsaved(&self) -> bool {
        self.saved != Some(self.undo.len())
    }

    /// The cards changed, in order, as text to copy into the sources: each
    /// card before and after.
    pub fn report(&self) -> String {
        let mut s = String::new();
        for c in &self.undo {
            for (k, e) in c.cards.iter().enumerate() {
                let what = if c.cards.len() == 1 {
                    c.what.clone()
                } else {
                    format!("{} ({} of {})", c.what, k + 1, c.cards.len())
                };
                s.push_str(&format!(
                    "c --- {what} before:\n{}c --- after:\n{}\n",
                    with_newline(&e.old),
                    with_newline(&e.new)
                ));
            }
        }
        s
    }
}

fn with_newline(t: &str) -> String {
    if t.ends_with('\n') {
        t.to_string()
    } else {
        format!("{t}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant_scene::source::CardKind;

    fn card(id: i64) -> Change {
        Change::one(
            id,
            Edited {
                kind: CardKind::Surface,
                slot: id as u32,
                old: format!("{id} so 5\n"),
                new: format!("{id} so 6\n"),
                added: None,
            },
        )
    }

    #[test]
    fn edits_that_touch_labels() {
        let cell = |old: &str, new: &str| {
            Change::one(
                1,
                Edited {
                    kind: CardKind::Cell,
                    slot: 0,
                    old: old.into(),
                    new: new.into(),
                    added: None,
                },
            )
        };
        let same = "c header\n1 3 -7.8 -1 imp:n=1 $ steel\n";
        assert!(!cell(same, &same.replace("-7.8", "-7.9")).touches_labels());
        assert!(!cell(same, &same.replace("header", "other header")).touches_labels());
        assert!(cell(same, &same.replace("steel", "lead")).touches_labels());
        assert!(cell(same, &same.replace("1 3", "7 3")).touches_labels());
        assert!(cell("", same).touches_labels());
        assert!(!card(4).touches_labels());
    }

    #[test]
    fn edits_undo_and_saving() {
        let mut e = InputEdits::default();
        assert!(!e.unsaved() && e.is_empty());
        e.push(card(1));
        e.push(card(2));
        assert!(e.unsaved());
        e.saved_at(e.len());
        assert!(!e.unsaved());
        assert_eq!(e.last().unwrap().what, "surface 2");
        e.pop();
        assert!(e.unsaved(), "the file has edit 2, the text not");
        e.push(card(3));
        assert!(e.unsaved());
        e.pop();
        e.pop();
        assert!(e.is_empty() && e.unsaved(), "the file has edit 2");
        e.push(card(3));
        let mut two = card(4);
        two.what = "renumbered".into();
        two.cards.push(card(5).cards[0].clone());
        e.push(two);
        let r = e.report();
        assert!(
            r.contains("c --- renumbered (2 of 2) before:\n5 so 5\n"),
            "{r}"
        );
        assert!(
            r.contains("surface 3 before:\n3 so 5\nc --- after:\n3 so 6\n"),
            "{r}"
        );
    }
}
