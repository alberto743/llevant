//! A card's text cut into words that know what they name, for the
//! inspector to colour and link: in a cell card, the cell's number, its
//! material and density, the surfaces of its geometry (with their sense),
//! complemented cells, operators, and the keywords after it with the
//! universes (`U=`, `FILL=`) and transforms (`TRCL=`, a fill's `(n)`) they
//! name; in a surface card, its transform and mnemonic; in a material card,
//! its nuclides. Comments (`$ …`, `c` lines) and spacing are kept as written,
//! so the pieces put back together are the card's text.
//!
//! This is for showing a card, not for reading it: the model was compiled
//! from the input by the engine's own parser, and a word this reads wrongly
//! only gets the wrong colour.

use llevant_scene::source::{lines, CardKind};

/// What a piece of a card is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Spacing, `&`, and anything not recognised.
    Plain,
    Comment,
    /// The card's own number (cell, surface, `mN`, `TRn`).
    Name,
    /// A material number (in a cell card; 0 is void).
    Material(i64),
    Density,
    /// A surface of a cell's geometry: its number and whether the cell is
    /// on its negative side.
    Surface {
        id: i64,
        negative: bool,
    },
    /// `#n`: a complemented cell; also the cell after `LIKE`.
    Cell(i64),
    /// `(`, `)`, `:`, `#` before a parenthesis.
    Operator,
    /// `IMP:N=`, `U=`, `LIKE`, `BUT`, a surface's mnemonic…
    Keyword,
    Universe(i64),
    Transform(i64),
    /// A keyword's value that names nothing (a number, a list).
    Value,
    /// A nuclide of a material card (`26056.31c`).
    Nuclide,
}

/// A piece of text and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub text: String,
    pub kind: Kind,
}

/// The card's lines (as written), each cut into pieces.
pub fn tokens(kind: CardKind, text: &str) -> Vec<Vec<Piece>> {
    let mut st = State::new(kind);
    text.trim_end().lines().map(|line| st.line(line)).collect()
}

/// Where a cell card's reading is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// The card's number.
    Name,
    /// Material, or `LIKE`.
    Material,
    Density,
    /// The cell `LIKE` copies.
    Like,
    Geometry,
    /// After the geometry (or `BUT`): keywords and their values.
    Params,
}

/// What a keyword's next numbers name.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Expect {
    Nothing,
    /// `U=`: one universe.
    Universe,
    /// `FILL=`: a universe, then maybe a transform in parentheses (or, for
    /// a lattice, ranges and a universe per element).
    Fill {
        seen: bool,
    },
    /// `TRCL=n`.
    Transform,
}

struct State {
    card: CardKind,
    phase: Phase,
    expect: Expect,
    /// Inside parentheses after `FILL=`.
    fill_paren: bool,
    /// Words read so far, comments and operators aside.
    words: usize,
}

impl State {
    fn new(card: CardKind) -> State {
        State {
            card,
            phase: Phase::Name,
            expect: Expect::Nothing,
            fill_paren: false,
            words: 0,
        }
    }

    fn line(&mut self, line: &str) -> Vec<Piece> {
        let mut out: Vec<Piece> = Vec::new();
        let push = |out: &mut Vec<Piece>, text: &str, kind: Kind| {
            if text.is_empty() {
                return;
            }
            match out.last_mut() {
                Some(p) if p.kind == kind && matches!(kind, Kind::Plain | Kind::Comment) => {
                    p.text.push_str(text)
                }
                _ => out.push(Piece {
                    text: text.to_string(),
                    kind,
                }),
            }
        };
        // A comment line inside a card.
        if lines::is_comment(line) && self.words > 0 {
            push(&mut out, line, Kind::Comment);
            return out;
        }
        let b = line.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i] as char;
            if c == '$' {
                push(&mut out, &line[i..], Kind::Comment);
                break;
            }
            if c.is_whitespace() || c == '&' {
                let j = scan(b, i, |c| c.is_whitespace() || c == '&');
                push(&mut out, &line[i..j], Kind::Plain);
                i = j;
                continue;
            }
            if matches!(c, '(' | ')' | ':') || (c == '#' && b.get(i + 1) == Some(&b'(')) {
                let kind = self.operator(c);
                push(&mut out, &line[i..i + 1], kind);
                i += 1;
                continue;
            }
            if c == '#' {
                let j = scan(b, i + 1, |c| c.is_ascii_digit());
                let id = line[i + 1..j].parse().unwrap_or(0);
                push(&mut out, &line[i..j], Kind::Cell(id));
                i = j;
                continue;
            }
            if c == '=' {
                push(&mut out, "=", Kind::Keyword);
                i += 1;
                continue;
            }
            // A word: up to a blank, a parenthesis, a colon (in the
            // geometry), `=` or a comment.
            let in_geometry = self.phase == Phase::Geometry;
            let j = scan(b, i, |c| {
                !(c.is_whitespace()
                    || matches!(c, '(' | ')' | '$' | '=' | '&')
                    || (c == ':' && in_geometry))
            });
            let j = j.max(i + 1);
            let word = &line[i..j];
            // `IMP:N` and `*TRCL` keep their colon or star; a keyword
            // spelled with `=` gets it next.
            let kind = self.word(word);
            push(&mut out, word, kind);
            i = j;
        }
        out
    }

    fn operator(&mut self, c: char) -> Kind {
        // `TRCL=(…)`: the transform written out, not a number.
        if c == '(' && self.expect == Expect::Transform {
            self.expect = Expect::Nothing;
        }
        if self.phase == Phase::Params && matches!(self.expect, Expect::Fill { .. }) {
            if c == '(' {
                self.fill_paren = true;
            } else if c == ')' {
                self.fill_paren = false;
            }
            return Kind::Operator;
        }
        Kind::Operator
    }

    fn word(&mut self, w: &str) -> Kind {
        self.words += 1;
        match self.card {
            CardKind::Cell => self.cell_word(w),
            CardKind::Surface => self.surface_word(w),
            CardKind::Material => {
                if self.words == 1 {
                    Kind::Name
                } else if w.contains('=') || w.chars().next().is_some_and(char::is_alphabetic) {
                    Kind::Keyword
                } else if self.words.is_multiple_of(2) {
                    Kind::Nuclide
                } else {
                    Kind::Value
                }
            }
            CardKind::Transform | CardKind::Data => {
                if self.words == 1 {
                    Kind::Name
                } else {
                    Kind::Value
                }
            }
        }
    }

    fn cell_word(&mut self, w: &str) -> Kind {
        let alpha = w
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '*');
        match self.phase {
            Phase::Name => {
                self.phase = Phase::Material;
                Kind::Name
            }
            Phase::Material => {
                if w.eq_ignore_ascii_case("like") {
                    self.phase = Phase::Like;
                    return Kind::Keyword;
                }
                let m: i64 = w.parse().unwrap_or(0);
                self.phase = if m == 0 {
                    Phase::Geometry
                } else {
                    Phase::Density
                };
                Kind::Material(m)
            }
            Phase::Density => {
                self.phase = Phase::Geometry;
                Kind::Density
            }
            Phase::Like => {
                self.phase = Phase::Params;
                match w.parse() {
                    Ok(n) => Kind::Cell(n),
                    Err(_) => Kind::Plain,
                }
            }
            Phase::Geometry if !alpha => match surface_ref(w) {
                Some((id, negative)) => Kind::Surface { id, negative },
                None => Kind::Plain,
            },
            Phase::Geometry | Phase::Params => {
                self.phase = Phase::Params;
                if alpha {
                    let key = w.trim_start_matches('*').to_ascii_lowercase();
                    self.expect = if key == "u" {
                        Expect::Universe
                    } else if key == "fill" {
                        Expect::Fill { seen: false }
                    } else if key == "trcl" {
                        Expect::Transform
                    } else {
                        Expect::Nothing
                    };
                    self.fill_paren = false;
                    return Kind::Keyword;
                }
                self.value(w)
            }
        }
    }

    /// A number after a keyword.
    fn value(&mut self, w: &str) -> Kind {
        let n: Option<i64> = w.parse().ok();
        match (self.expect, n) {
            (Expect::Universe, Some(u)) => {
                self.expect = Expect::Nothing;
                Kind::Universe(u)
            }
            (Expect::Transform, Some(t)) => {
                self.expect = Expect::Nothing;
                Kind::Transform(t)
            }
            (Expect::Fill { seen: false }, Some(u)) if !self.fill_paren => {
                self.expect = Expect::Fill { seen: true };
                Kind::Universe(u)
            }
            (Expect::Fill { seen: true }, Some(t)) if self.fill_paren => Kind::Transform(t),
            _ => Kind::Value,
        }
    }

    fn surface_word(&mut self, w: &str) -> Kind {
        let alpha = w.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        match self.words {
            1 => Kind::Name,
            2 if !alpha => match w.parse() {
                Ok(t) => Kind::Transform(t),
                Err(_) => Kind::Value,
            },
            _ if alpha => Kind::Keyword,
            _ => Kind::Value,
        }
    }
}

/// The end of the run from `i` of bytes `keep` accepts.
fn scan(b: &[u8], i: usize, keep: impl Fn(char) -> bool) -> usize {
    let mut j = i;
    while j < b.len() && keep(b[j] as char) {
        j += 1;
    }
    j
}

/// A surface in a cell's geometry: `-4120`, `+12`, `5000.3` (a facet).
fn surface_ref(w: &str) -> Option<(i64, bool)> {
    let negative = w.starts_with('-');
    let digits = w.trim_start_matches(['-', '+']);
    let id = digits.split('.').next()?.parse().ok()?;
    Some((id, negative))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(kind: CardKind, text: &str) -> Vec<(String, Kind)> {
        tokens(kind, text)
            .into_iter()
            .flatten()
            .filter(|p| p.kind != Kind::Plain)
            .map(|p| (p.text, p.kind))
            .collect()
    }

    #[test]
    fn pieces_put_back_are_the_text() {
        let text = "41008 33 -7.93  -4120 4121 (-4130:#7) #(1 -2)\n     u=320 imp:n=1 $ module 9\n";
        let back: String = tokens(CardKind::Cell, text)
            .iter()
            .map(|l| l.iter().map(|p| p.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(back, text.trim_end());
    }

    #[test]
    fn a_cell_card_names_what_it_uses() {
        let k = kinds(
            CardKind::Cell,
            "41008 33 -7.93 -4120 4121 (-4130:#7) #(1 -2) u=320 imp:n=1 $ module",
        );
        let want = [
            ("41008", Kind::Name),
            ("33", Kind::Material(33)),
            ("-7.93", Kind::Density),
            (
                "-4120",
                Kind::Surface {
                    id: 4120,
                    negative: true,
                },
            ),
            (
                "4121",
                Kind::Surface {
                    id: 4121,
                    negative: false,
                },
            ),
            ("(", Kind::Operator),
            (
                "-4130",
                Kind::Surface {
                    id: 4130,
                    negative: true,
                },
            ),
            (":", Kind::Operator),
            ("#7", Kind::Cell(7)),
            (")", Kind::Operator),
            ("#", Kind::Operator),
            ("(", Kind::Operator),
            (
                "1",
                Kind::Surface {
                    id: 1,
                    negative: false,
                },
            ),
            (
                "-2",
                Kind::Surface {
                    id: 2,
                    negative: true,
                },
            ),
            (")", Kind::Operator),
            ("u", Kind::Keyword),
            ("=", Kind::Keyword),
            ("320", Kind::Universe(320)),
            ("imp:n", Kind::Keyword),
            ("=", Kind::Keyword),
            ("1", Kind::Value),
            ("$ module", Kind::Comment),
        ];
        let want: Vec<(String, Kind)> = want.iter().map(|(t, k)| (t.to_string(), *k)).collect();
        assert_eq!(k, want);
    }

    #[test]
    fn void_cells_fills_transforms_and_like() {
        let k = kinds(CardKind::Cell, "10 0 -1 2.3 fill=100 (7) *trcl=(0 0 1)");
        assert!(k.contains(&("0".into(), Kind::Material(0))));
        assert!(k.contains(&(
            "2.3".into(),
            Kind::Surface {
                id: 2,
                negative: false
            }
        )));
        assert!(k.contains(&("100".into(), Kind::Universe(100))));
        assert!(k.contains(&("7".into(), Kind::Transform(7))));
        assert!(k.contains(&("*trcl".into(), Kind::Keyword)));
        assert!(!k.iter().any(|(_, k)| *k == Kind::Transform(0)), "{k:?}");
        let k = kinds(CardKind::Cell, "11 like 10 but trcl=4 u=5");
        assert!(k.contains(&("like".into(), Kind::Keyword)));
        assert!(k.contains(&("10".into(), Kind::Cell(10))));
        assert!(k.contains(&("4".into(), Kind::Transform(4))));
        assert!(k.contains(&("5".into(), Kind::Universe(5))));
    }

    #[test]
    fn comment_lines_and_other_cards() {
        let lines = tokens(CardKind::Cell, "1 0 -1\nc a comment\n     2");
        assert_eq!(
            lines[1],
            vec![Piece {
                text: "c a comment".into(),
                kind: Kind::Comment
            }]
        );
        assert_eq!(
            lines[2].last().unwrap().kind,
            Kind::Surface {
                id: 2,
                negative: false
            }
        );
        let k = kinds(CardKind::Surface, "4120 3 tz 0 0 0 620 223 368");
        assert_eq!(k[0], ("4120".into(), Kind::Name));
        assert_eq!(k[1], ("3".into(), Kind::Transform(3)));
        assert_eq!(k[2], ("tz".into(), Kind::Keyword));
        let k = kinds(
            CardKind::Material,
            "m33 26056.31c -0.65 24052.31c -0.17 nlib=31c",
        );
        assert_eq!(k[1], ("26056.31c".into(), Kind::Nuclide));
        assert_eq!(k[2], ("-0.65".into(), Kind::Value));
    }
}
