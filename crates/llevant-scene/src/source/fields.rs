//! Cards edited by their fields (decision 0012, phase 8): a cell's
//! material, density and parameters, a material's entries and keywords, a
//! surface's type. Each change is made by migjorn on the card alone, so the
//! card keeps its text and only the words changed are written; the result
//! is the card's new text, for [`Source::edit`] (the input itself is not
//! touched here).

use migjorn::EditError;

use super::lines::columns;
use super::{CardKind, Source};

/// A parameter of a cell card, as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// Its key as migjorn looks it up: lower case, the particles after a
    /// colon (`imp:n`, `vol`, `fill`), without the `*`.
    pub key: String,
    /// `*TRCL`, `*FILL`: rotations in degrees.
    pub starred: bool,
    /// Its value as written (`1`, `100 (7)`).
    pub value: String,
}

impl Param {
    /// As MCNP users write it: `IMP:N`, `*TRCL`.
    pub fn label(&self) -> String {
        format!(
            "{}{}",
            if self.starred { "*" } else { "" },
            self.key.to_uppercase()
        )
    }
}

/// What a cell card gives, field by field.
#[derive(Clone, Debug, PartialEq)]
pub struct CellFields {
    /// `LIKE n BUT`: the cell it copies (it then has no material field; its
    /// `MAT=` and `RHO=` are parameters).
    pub like: Option<i64>,
    /// Its material (0: void); `None` for a `LIKE` cell.
    pub material: Option<i64>,
    /// Its density as written (negative: g/cm³); `None` when void.
    pub density: Option<f64>,
    /// Its parameters, in the card's order.
    pub params: Vec<Param>,
    /// Its `FILL` has ranges (a lattice's array), which stays text.
    pub lattice_fill: bool,
    /// Its geometry, as migjorn reads it (`-1 2 (3 : #4)`, on one line).
    pub geometry: String,
}

impl CellFields {
    /// Why the geometry of this card, cell `id`'s, cannot be rewritten by
    /// llevant (`what`: "cut", "simplify"): a `LIKE` cell has none of its
    /// own, and MCNP moves the surfaces of a `#n` by the cell's own `TRCL`.
    pub(super) fn rewrite_refusal(&self, id: i64, what: &str) -> Option<String> {
        if self.like.is_some() {
            Some(format!("cell {id} is a LIKE cell: {what} it by hand"))
        } else if self.params.iter().any(|p| p.key == "trcl") {
            Some(format!(
                "cell {id} has a TRCL, which would move its #n with it: {what} it by hand"
            ))
        } else {
            None
        }
    }
}

/// One change of a cell card.
#[derive(Clone, Debug, PartialEq)]
pub enum CellChange {
    /// The material (0: void, which drops the density; from void, a
    /// density is written too, so give one after).
    Material(i64),
    /// The density, as MCNP takes it (negative: g/cm³).
    Density(f64),
    /// The density times this, in the unit it is written in (seven
    /// significant digits kept); a void cell stays void.
    ScaleDensity(f64),
    /// A parameter set to a value (added if the card has none), or removed
    /// with `None`. `key` is [`Param::key`] for one the card has; a new one
    /// may be starred (`*trcl`).
    Param { key: String, value: Option<String> },
    /// The geometry, as text (line breaks are spaces). With as many terms
    /// as before, only the terms changed are written, the card's line
    /// breaks kept; else the expression is written again.
    Geometry(String),
}

/// An entry of a material as edited: the ZAID, its fraction (negative: by
/// mass), and which entry of the card it was (`None`: added).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub origin: Option<usize>,
    pub zaid: String,
    pub fraction: f64,
}

/// What a material card gives.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialFields {
    /// `(zaid, fraction)` in the card's order.
    pub entries: Vec<(String, f64)>,
    /// Keywords after the entries (`nlib`, `.80c`…), as written.
    pub keywords: Vec<(String, String)>,
}

impl Source {
    /// Cell `id`'s fields (`None` if it is not in the input or its card
    /// does not read).
    pub fn cell_fields(&self, id: i64) -> Option<CellFields> {
        let tree = self.tree();
        let c = tree.cell(id)?;
        let like = c.like();
        let lattice_fill = matches!(c.fill(), Ok(Some(migjorn::Fill::Array { .. })));
        let params = c
            .params()
            .into_iter()
            .map(|p| Param {
                key: p.qualified_key().to_ascii_lowercase(),
                starred: p.starred,
                value: p.value.trim().to_string(),
            })
            .collect();
        Some(CellFields {
            like,
            material: if like.is_some() { None } else { c.material() },
            density: c.density(),
            params,
            lattice_fill,
            geometry: c.geometry_text(),
        })
    }

    /// The keys (as [`Param::key`]) that data cards give for every cell
    /// (`imp:n`, `vol`, `u`…): MCNP rejects them on a cell card too.
    pub fn per_cell_keys(&self) -> Vec<String> {
        let tree = self.tree();
        let mut keys: Vec<String> = tree
            .per_cell_cards()
            .iter()
            .filter_map(|d| {
                let name = d.mnemonic()?.trim_start_matches('*').to_ascii_lowercase();
                Some(match d.particle() {
                    Some(p) => format!("{name}:{}", p.to_ascii_lowercase()),
                    None => name,
                })
            })
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// Cell `id`'s card with `changes` made, in order, everything else as
    /// written (for [`Source::edit`]).
    pub fn cell_with(&self, id: i64, changes: &[CellChange]) -> Result<String, String> {
        self.edited_alone(CardKind::Cell, id, |m, slot| {
            let mut c = m.cell_mut_at(slot).ok_or(EditError::WrongKind)?;
            for ch in changes {
                match ch {
                    CellChange::Material(mat) => c.set_material(*mat)?,
                    CellChange::Density(d) => c.set_density(*d)?,
                    CellChange::ScaleDensity(k) => {
                        if let Some(d) = c.view().density() {
                            c.set_density(significant(d * k, 7))?
                        }
                    }
                    CellChange::Param { key, value: None } => {
                        c.remove_param(key.trim_start_matches('*'))?;
                    }
                    CellChange::Param {
                        key,
                        value: Some(v),
                    } => {
                        if v.trim().is_empty() {
                            return Err(EditError::Invalid(format!("{key} needs a value")));
                        }
                        c.set_param(key, v)?
                    }
                    CellChange::Geometry(text) => {
                        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        let new = geometry_terms(&text).ok_or_else(|| {
                            EditError::Invalid("the geometry does not read".into())
                        })?;
                        let old: Vec<String> =
                            c.view().geometry().into_iter().map(|t| t.text).collect();
                        let termwise = old.len() == new.len()
                            && old
                                .iter()
                                .zip(&new)
                                .enumerate()
                                .filter(|(_, (o, n))| o != n)
                                .try_for_each(|(k, (_, n))| c.set_geometry_term(k, n))
                                .is_ok();
                        if !termwise {
                            c.set_geometry(&text)?;
                        }
                    }
                }
            }
            Ok(())
        })
        .map(|t| wrap_long_lines(&t))
    }

    /// The cards of cells `ids` each with `changes` made (as
    /// [`Source::cell_with`]), for [`Source::edit_many`]; the first error
    /// (naming its cell) if any card cannot take them.
    pub fn cells_with(
        &self,
        ids: &[i64],
        changes: &[CellChange],
    ) -> Result<Vec<(CardKind, i64, String)>, String> {
        use rayon::prelude::*;
        ids.par_iter()
            .map(|&id| Ok((CardKind::Cell, id, self.cell_with(id, changes)?)))
            .collect()
    }

    /// Every surface of the input, by number, with its type (for
    /// completing numbers).
    pub fn surface_list(&self) -> Vec<(i64, String)> {
        let tree = self.tree();
        let mut v: Vec<(i64, String)> = tree
            .surfaces()
            .filter_map(|s| Some((s.id()?, s.mnemonic().unwrap_or("").to_uppercase())))
            .collect();
        v.sort_by_key(|e| e.0);
        v.dedup_by_key(|e| e.0);
        v
    }

    /// Cell `id`'s card with cell `n` cut out of it: `#n` added to its
    /// geometry (the geometry put in parentheses first if it has a `:` of
    /// its own).
    pub fn cell_cut(&self, id: i64, n: i64) -> Result<String, String> {
        let f = self
            .cell_fields(id)
            .ok_or_else(|| format!("no cell {id} in the input"))?;
        if let Some(why) = f.rewrite_refusal(id, "cut") {
            return Err(why);
        }
        let g = f.geometry;
        let g = if g.contains(':') { format!("({g})") } else { g };
        self.cell_with(id, &[CellChange::Geometry(format!("{g} #{n}"))])
    }

    /// An input of cell `id` alone with `geometry` (its `TRCL` kept), the
    /// cells its `#n` name (as their geometry and `TRCL`, at any depth),
    /// and the surfaces and transforms these use: compiled, it says where
    /// the cell as edited is, in the frame of its universe (the slice's
    /// preview). Cell `id` may be one not in the input yet (a new cell,
    /// without a `TRCL`). An error for a `LIKE n BUT` cell, or what the
    /// input has not got.
    pub fn cell_alone(&self, id: i64, geometry: &str) -> Result<String, String> {
        const MOST: usize = 200;
        let mut cells: Vec<(i64, String)> = vec![(id, geometry.to_string())];
        let mut surfaces: Vec<i64> = Vec::new();
        let mut transforms: Vec<i64> = Vec::new();
        let mut lines = Vec::new();
        let mut k = 0;
        while k < cells.len() {
            let (c, geo) = cells[k].clone();
            k += 1;
            let f = self.cell_fields(c);
            if f.is_none() && c != id {
                return Err(format!("no cell {c} in the input"));
            }
            if f.as_ref().is_some_and(|f| f.like.is_some()) {
                return Err(format!("cell {c} is a LIKE cell"));
            }
            for w in geometry_words(&geo) {
                match w.kind {
                    Word::Surface { id: s, .. } if !surfaces.contains(&s) => surfaces.push(s),
                    Word::Cell(n) if !cells.iter().any(|e| e.0 == n) => {
                        if cells.len() >= MOST {
                            return Err("too many cells complemented".into());
                        }
                        let g = self
                            .cell_fields(n)
                            .ok_or_else(|| format!("no cell {n} in the input"))?
                            .geometry;
                        cells.push((n, g));
                    }
                    Word::Bad => return Err("the geometry does not read".into()),
                    _ => {}
                }
            }
            let trcl: Vec<String> = f
                .iter()
                .flat_map(|f| &f.params)
                .filter(|p| p.key == "trcl")
                .map(|p| format!("{}trcl={}", if p.starred { "*" } else { "" }, p.value))
                .collect();
            for t in self.transforms_of(CardKind::Cell, c) {
                if !transforms.contains(&t) && trcl.iter().any(|p| !p.contains('(')) {
                    transforms.push(t);
                }
            }
            lines.push(wrap_long_lines(&format!(
                "{c} 0 {geo} {}\n",
                trcl.join(" ")
            )));
        }
        let mut out = format!("preview of cell {id}\n");
        for l in &lines {
            out.push_str(l);
        }
        out.push('\n');
        for &s in &surfaces {
            let card = self
                .surface(s)
                .ok_or_else(|| format!("no surface {s} in the input"))?;
            out.push_str(card.text.trim_end());
            out.push('\n');
            for t in self.transforms_of(CardKind::Surface, s) {
                if !transforms.contains(&t) {
                    transforms.push(t);
                }
            }
        }
        out.push('\n');
        for t in transforms {
            if let Some(card) = self.transform(t) {
                out.push_str(card.text.trim_end());
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// What is wrong with geometry `text` for a cell of this input: words
    /// that are not geometry, surfaces and cells the input has not got,
    /// parentheses that do not pair. Empty when it reads.
    pub fn check_geometry(&self, text: &str) -> Vec<Flag> {
        let words = geometry_words(text);
        let tree = self.tree();
        let mut flags = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for w in &words {
            let flag = |message: String| Flag {
                span: w.span.clone(),
                message,
            };
            match w.kind {
                Word::Surface { id, .. } if tree.surface(id).is_none() => {
                    flags.push(flag(format!("no surface {id}")))
                }
                Word::Cell(c) if tree.cell(c).is_none() => flags.push(flag(format!("no cell {c}"))),
                Word::Bad => flags.push(flag(format!(
                    "`{}` is not a surface, a `#cell`, `(`, `)` or `:`",
                    &text[w.span.clone()]
                ))),
                Word::Open => open.push(w.span.start),
                Word::Close if open.pop().is_none() => {
                    flags.push(flag("a `)` without its `(`".into()))
                }
                _ => {}
            }
        }
        for at in open {
            flags.push(Flag {
                span: at..at + 1,
                message: "a `(` without its `)`".into(),
            });
        }
        let named = words
            .iter()
            .any(|w| matches!(w.kind, Word::Surface { .. } | Word::Cell(_)));
        if flags.is_empty() && !named {
            flags.push(Flag {
                span: 0..text.len(),
                message: "a cell needs a surface or a `#cell`".into(),
            });
        }
        flags
    }

    /// Material `id`'s fields.
    pub fn material_fields(&self, id: i64) -> Option<MaterialFields> {
        let tree = self.tree();
        let m = tree.material(id)?;
        let entries = m
            .entries()
            .into_iter()
            .map(|(z, f)| (z.to_string(), f))
            .collect();
        let keywords = m
            .keywords()
            .ok()?
            .into_iter()
            .map(|k| (k.name.to_string(), k.values.text().trim().to_string()))
            .collect();
        Some(MaterialFields { entries, keywords })
    }

    /// Material `id`'s card with its entries as `entries` (those kept in
    /// the card's order) and `keywords` set (`Some`) or removed (`None`),
    /// the rest as written.
    pub fn material_with(
        &self,
        id: i64,
        entries: &[Entry],
        keywords: &[(String, Option<String>)],
    ) -> Result<String, String> {
        let old = self
            .material_fields(id)
            .ok_or_else(|| format!("material {id} is not in the input (in a READ file?)"))?;
        let kept: Vec<usize> = entries.iter().filter_map(|e| e.origin).collect();
        if kept.windows(2).any(|w| w[0] >= w[1]) || kept.iter().any(|&k| k >= old.entries.len()) {
            return Err(format!("material {id}: entries out of order"));
        }
        if entries.is_empty() {
            return Err(format!("material {id}: a material needs an entry"));
        }
        self.edited_alone(CardKind::Material, id, |m, slot| {
            let mut mat = m.material_mut_at(slot).ok_or(EditError::WrongKind)?;
            // Added first (so the card never has no entry), then the old
            // ones dropped from the back, then the kept ones set.
            for e in entries.iter().filter(|e| e.origin.is_none()) {
                mat.add_entry(e.zaid.trim(), e.fraction)?;
            }
            for k in (0..old.entries.len()).rev() {
                if !kept.contains(&k) {
                    mat.remove_entry(k)?;
                }
            }
            for (now, e) in entries.iter().filter(|e| e.origin.is_some()).enumerate() {
                let (zaid, f) = &old.entries[e.origin.unwrap_or(0)];
                if e.zaid.trim() != zaid {
                    mat.set_zaid(now, e.zaid.trim())?;
                }
                if e.fraction != *f {
                    mat.set_fraction(now, e.fraction)?;
                }
            }
            for (name, value) in keywords {
                match value {
                    Some(v) => mat.set_keyword(name, v)?,
                    None => {
                        mat.remove_keyword(name)?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Surface `id`'s card written as type `mnemonic` with `coeffs`
    /// instead (its number, transform, comments as written).
    pub fn surface_retyped(
        &self,
        id: i64,
        mnemonic: &str,
        coeffs: &[f64],
    ) -> Result<String, String> {
        self.edited_alone(CardKind::Surface, id, |m, slot| {
            let s = m.surface_at(slot).ok_or(EditError::WrongKind)?;
            let text = s.text();
            let old = s.mnemonic().ok_or(EditError::NoSuchField)?;
            // The mnemonic is a slice of the card's text: its place in it.
            let at = (old.as_ptr() as usize)
                .checked_sub(text.as_ptr() as usize)
                .filter(|&a| a + old.len() <= text.len())
                .ok_or(EditError::NoSuchField)?;
            let lower = old.chars().all(|c| !c.is_ascii_uppercase());
            let new = if lower {
                mnemonic.to_ascii_lowercase()
            } else {
                mnemonic.to_ascii_uppercase()
            };
            let retyped = format!("{}{new}{}", &text[..at], &text[at + old.len()..]);
            let mut s = m.surface_mut_at(slot).ok_or(EditError::WrongKind)?;
            // The old coefficients may not suit the new type: the card
            // written again after its mnemonic, its `$` comments kept.
            s.set_text(&retyped).or_else(|_| {
                let first = at + new.len();
                let mut t = retyped[..first].to_string();
                for c in coeffs {
                    t.push(' ');
                    t.push_str(&format!("{c}"));
                }
                let notes: Vec<&str> = retyped[first..]
                    .lines()
                    .filter_map(|l| l.split_once('$').map(|(_, c)| c.trim()))
                    .filter(|c| !c.is_empty())
                    .collect();
                if !notes.is_empty() {
                    t.push_str(" $ ");
                    t.push_str(&notes.join("; "));
                }
                t.push('\n');
                s.set_text(&t)
            })?;
            s.set_coeffs(coeffs)
        })
    }

    /// The card of `kind` and number `id` with `edit` made by migjorn on
    /// it alone (in an input of its own), as text.
    fn edited_alone(
        &self,
        kind: CardKind,
        id: i64,
        edit: impl FnOnce(&mut migjorn::Model, u32) -> Result<(), EditError>,
    ) -> Result<String, String> {
        let what = format!("{} {id}", kind.name());
        let card = self
            .card(kind, id)
            .ok_or_else(|| format!("{what} is not in the input (in a READ file?)"))?;
        let mut text = card.text;
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let input = match kind {
            CardKind::Cell => format!("t\n{text}\n1 so 1\n\n"),
            CardKind::Surface => format!("t\n1 0 -1\n\n{text}\n"),
            CardKind::Material | CardKind::Transform | CardKind::Data => {
                format!("t\n1 0 -1\n\n1 so 1\n\n{text}")
            }
        };
        let mut m = migjorn::Model::parse(&input);
        let slot = match kind {
            CardKind::Cell => m.cell(id).map(|c| c.slot()),
            CardKind::Surface => m.surface(id).map(|c| c.slot()),
            CardKind::Material => m.material(id).map(|c| c.slot()),
            CardKind::Transform => m.transform(id).map(|c| c.slot()),
            CardKind::Data => None,
        }
        .ok_or_else(|| format!("{what}: its card does not read alone"))?;
        edit(&mut m, slot).map_err(|e| format!("{what}: {e}"))?;
        let out = m
            .cst()
            .card(slot)
            .map(|c| c.text().to_string())
            .ok_or_else(|| format!("{what}: lost"))?;
        Ok(out)
    }
}

/// `x` to `digits` significant digits (a density scaled: `-2.97`, not
/// `-2.9700000000000006`).
fn significant(x: f64, digits: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{:.*e}", (digits - 1) as usize, x)
        .parse()
        .unwrap_or(x)
}

/// A word of a geometry expression ([`geometry_words`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    /// A surface (`-470.2`): its number and sense.
    Surface {
        id: i64,
        negative: bool,
    },
    /// `#n`: the complement of cell `n`.
    Cell(i64),
    /// The `#` of `#( … )`.
    Complement,
    Open,
    Close,
    Union,
    /// Not a word of geometry.
    Bad,
}

/// A word of geometry text and where it is (bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spanned {
    pub kind: Word,
    pub span: std::ops::Range<usize>,
}

/// Something wrong with a word of geometry text ([`Source::check_geometry`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flag {
    pub span: std::ops::Range<usize>,
    pub message: String,
}

/// The words of geometry text, in order, as MCNP reads them.
pub fn geometry_words(text: &str) -> Vec<Spanned> {
    let b = text.as_bytes();
    let stop = |c: u8| c.is_ascii_whitespace() || matches!(c, b'(' | b')' | b':' | b'#');
    // The end of the run of non-stop bytes from `k`.
    let run = |mut e: usize| {
        while e < b.len() && !stop(b[e]) {
            e += 1;
        }
        e
    };
    let mut out = Vec::new();
    let mut k = 0;
    while k < b.len() {
        let c = b[k];
        let (kind, end) = match c {
            c if c.is_ascii_whitespace() => {
                k += 1;
                continue;
            }
            b'(' => (Word::Open, k + 1),
            b')' => (Word::Close, k + 1),
            b':' => (Word::Union, k + 1),
            b'#' => {
                let mut j = k + 1;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j >= b.len() || b[j] == b'(' {
                    (Word::Complement, k + 1)
                } else {
                    let e = run(j);
                    match text[j..e].parse::<i64>() {
                        Ok(n) if n > 0 => (Word::Cell(n), e),
                        _ => (Word::Bad, e.max(k + 1)),
                    }
                }
            }
            _ => {
                let e = run(k);
                match migjorn::SurfaceRef::parse(&text[k..e]) {
                    Some(r) => (
                        Word::Surface {
                            id: r.id,
                            negative: r.negative,
                        },
                        e,
                    ),
                    None => (Word::Bad, e.max(k + 1)),
                }
            }
        };
        out.push(Spanned { kind, span: k..end });
        k = end;
    }
    out
}

/// The terms migjorn reads in geometry `text` (`None` if it does not read
/// as a cell's geometry alone).
fn geometry_terms(text: &str) -> Option<Vec<String>> {
    let m = migjorn::Model::parse(&format!("t\n1 0 {text}\n\n1 so 1\n\n"));
    let c = m.cell(1)?;
    if !c.params().is_empty() || !c.well_formed() {
        return None;
    }
    let mut depth = 0i32;
    for w in geometry_words(text) {
        match w.kind {
            Word::Open => depth += 1,
            Word::Close => depth -= 1,
            Word::Bad => return None,
            _ => {}
        }
        if depth < 0 {
            return None;
        }
    }
    if depth != 0 {
        return None;
    }
    Some(c.geometry().into_iter().map(|t| t.text).collect())
}

/// Lines are broken this far in (columns), under [`super::MAX_COLUMNS`].
const BREAK_AT: usize = 100;

/// `word` is a cell parameter, or the start of the parameters: it has an
/// `=`, or starts with a letter or a `*`.
fn ends_geometry(word: &str) -> bool {
    word.contains('=') || word.starts_with(|c: char| c.is_ascii_alphabetic() || c == '*')
}

/// The pieces a line's code can be broken between: its words, and inside a
/// word of geometry alone (digits, signs, `.`, `:`, `(`, `)`, `#`) before
/// each `:` and `)` and after each `(`, as MCNP reads a line break as a
/// blank. Each piece says whether a blank comes before it.
fn pieces<'a>(code: &'a str, in_geometry: &mut bool) -> Vec<(&'a str, bool)> {
    let mut out = Vec::new();
    // The geometry ends where the first parameter (`imp:n=1`, `fill=`…, or
    // `u 1`: the first word with an `=` or that starts with a letter or a
    // `*`) is: after it a `0:0` is a range of a lattice's `fill`, whole. (A
    // letter inside a word does not end it: a density `-1.2e-3` comes before
    // the geometry.) `in_geometry` carries it from line to line.
    for word in code.split_whitespace() {
        *in_geometry &= !ends_geometry(word);
        let geometry = *in_geometry && word.chars().all(|c| "0123456789+-.:()#".contains(c));
        let mut start = 0;
        let mut blank = true;
        if geometry {
            for (k, c) in word.char_indices() {
                let before = matches!(c, ':' | ')') && k > start;
                let after = c == '(' && k + 1 < word.len();
                if before {
                    out.push((&word[start..k], blank));
                    (start, blank) = (k, false);
                }
                if after {
                    out.push((&word[start..=k], blank));
                    (start, blank) = (k + 1, false);
                }
            }
        }
        out.push((&word[start..], blank));
    }
    out
}

/// `note` (a `$` comment) on a continuation line of its own if it fits, as
/// written or with one blank after the `$` (it stays a `$` comment, so a
/// label of the cell stays one), else as comment lines ([`comment_lines`]).
fn own_line_or_comments(note: &str, newline: &str) -> String {
    let text = note.trim_start_matches('$').trim();
    [format!("     {note}"), format!("     $ {text}")]
        .into_iter()
        .find(|l| columns(l) <= super::MAX_COLUMNS)
        .map(|l| format!("{l}{newline}"))
        .unwrap_or_else(|| comment_lines(note, newline))
}

/// `note` (a `$` comment) as comment lines, each within
/// [`super::MAX_COLUMNS`], broken at blanks where it can be.
fn comment_lines(note: &str, newline: &str) -> String {
    let text = note.trim_start_matches('$').trim();
    let room = super::MAX_COLUMNS - 2;
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while columns(&word) > room {
            let head: String = word.chars().take(room).collect();
            word = word.chars().skip(room).collect();
            if !line.is_empty() {
                out += &format!("c {line}{newline}");
                line.clear();
            }
            out += &format!("c {head}{newline}");
        }
        if !line.is_empty() && columns(&line) + 1 + columns(&word) > room {
            out += &format!("c {line}{newline}");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line += &word;
    }
    if !line.is_empty() || out.is_empty() {
        out += &format!("c {line}{newline}");
    }
    out
}

/// A card with each line over [`super::MAX_COLUMNS`] broken into
/// continuation lines of five spaces: between words, or inside a word of
/// geometry at a `:` or a parenthesis (so a union printed without blanks
/// breaks too), never inside a number or a keyword; widths in characters.
/// A `$` comment stays at the end of the last line, with a word moved to a
/// line of its own if that makes room, else it goes on a line of its own
/// if it fits (still a `$` comment), else it becomes comment lines after
/// the card. Lines within the columns, and comment lines, are left as they
/// are; a word longer than a line is left whole.
pub fn wrap_long_lines(card: &str) -> String {
    let max = super::MAX_COLUMNS;
    if card.lines().all(|l| columns(l) <= max) {
        return card.to_string();
    }
    let mut out = String::new();
    // Whether the geometry goes on: it ends at the first parameter, on
    // whichever line that is (a `fill=` range on the next line is whole).
    let mut in_geometry = true;
    for line in card.split_inclusive('\n') {
        let (body, end) = match line.strip_suffix("\r\n") {
            Some(b) => (b, "\r\n"),
            None => match line.strip_suffix('\n') {
                Some(b) => (b, "\n"),
                None => (line, ""),
            },
        };
        if super::lines::is_comment(body) {
            out.push_str(line);
            continue;
        }
        if columns(body) <= max {
            let code = body.split('$').next().unwrap_or("");
            in_geometry &= !code.split_whitespace().any(ends_geometry);
            out.push_str(line);
            continue;
        }
        let newline = if end.is_empty() { "\n" } else { end };
        let (code, note) = match body.find('$') {
            Some(k) => body.split_at(k),
            None => (body, ""),
        };
        let note = note.trim_end();
        let lead = &code[..code.len() - code.trim_start().len()];
        // Pieces packed into lines of at most `BREAK_AT` columns.
        let mut lines: Vec<Vec<(&str, bool)>> = vec![Vec::new()];
        let mut used = columns(lead);
        for (piece, blank) in pieces(code, &mut in_geometry) {
            let cur = lines.last_mut().expect("one line");
            let add = columns(piece) + usize::from(blank && !cur.is_empty());
            if !cur.is_empty() && used + add > BREAK_AT {
                lines.push(vec![(piece, blank)]);
                used = 5 + columns(piece);
            } else {
                cur.push((piece, blank));
                used += add;
            }
        }
        // A line with only a `$` comment: the comment alone (a line of
        // blanks before it would end the cell block where MCNP reads it).
        if lines.iter().all(Vec::is_empty) {
            if note.is_empty() {
                out.push_str(line);
            } else {
                out.push_str(&own_line_or_comments(note, newline));
            }
            continue;
        }
        let text = |k: usize, l: &[(&str, bool)]| {
            let mut s = String::from(if k == 0 { lead } else { "     " });
            for (j, (p, blank)) in l.iter().enumerate() {
                if *blank && j > 0 {
                    s.push(' ');
                }
                s.push_str(p);
            }
            s
        };
        // Room for the comment on the last line, or on a line of its own
        // with the last piece.
        let mut after = String::new();
        if !note.is_empty() {
            let k = lines.len() - 1;
            let fits = |s: &str| columns(s) + 1 + columns(note) <= max;
            if !fits(&text(k, &lines[k])) {
                let moved = (lines[k].len() > 1).then(|| lines[k].pop()).flatten();
                match moved {
                    Some(p) if fits(&text(1, &[p])) => lines.push(vec![p]),
                    Some(p) => {
                        lines[k].push(p);
                        after = own_line_or_comments(note, newline);
                    }
                    None => after = own_line_or_comments(note, newline),
                }
            }
        }
        let n = lines.len();
        for (k, l) in lines.iter().enumerate() {
            out.push_str(&text(k, l));
            if k + 1 == n {
                if after.is_empty() && !note.is_empty() {
                    out.push(' ');
                    out.push_str(note);
                }
                out.push_str(if after.is_empty() { end } else { newline });
            } else {
                out.push_str(newline);
            }
        }
        out.push_str(&after);
    }
    out
}

/// The names of a surface type's coefficients, in order (empty when not
/// known; the positions are then shown).
pub fn coefficient_names(mnemonic: &str, count: usize) -> Vec<&'static str> {
    const V: [&str; 3] = ["vx", "vy", "vz"];
    const H: [&str; 3] = ["hx", "hy", "hz"];
    let m = mnemonic.to_ascii_lowercase();
    let v: Vec<&'static str> = match m.as_str() {
        "p" if count == 4 => vec!["A", "B", "C", "D"],
        "p" if count == 9 => vec!["x₁", "y₁", "z₁", "x₂", "y₂", "z₂", "x₃", "y₃", "z₃"],
        "px" => vec!["x"],
        "py" => vec!["y"],
        "pz" => vec!["z"],
        "so" => vec!["R"],
        "s" => vec!["x₀", "y₀", "z₀", "R"],
        "sx" => vec!["x₀", "R"],
        "sy" => vec!["y₀", "R"],
        "sz" => vec!["z₀", "R"],
        "c/x" => vec!["y₀", "z₀", "R"],
        "c/y" => vec!["x₀", "z₀", "R"],
        "c/z" => vec!["x₀", "y₀", "R"],
        "cx" | "cy" | "cz" => vec!["R"],
        "k/x" | "k/y" | "k/z" => vec!["x₀", "y₀", "z₀", "t²", "±1"],
        "kx" => vec!["x₀", "t²", "±1"],
        "ky" => vec!["y₀", "t²", "±1"],
        "kz" => vec!["z₀", "t²", "±1"],
        "sq" => vec!["A", "B", "C", "D", "E", "F", "G", "x₀", "y₀", "z₀"],
        "gq" => vec!["A", "B", "C", "D", "E", "F", "G", "H", "J", "K"],
        "tx" | "ty" | "tz" => vec!["x₀", "y₀", "z₀", "R", "a", "b"],
        "rpp" => vec!["x min", "x max", "y min", "y max", "z min", "z max"],
        "sph" => vec!["vx", "vy", "vz", "R"],
        "rcc" => [V.as_slice(), H.as_slice(), &["R"]].concat(),
        "trc" => [V.as_slice(), H.as_slice(), &["R₁", "R₂"]].concat(),
        "box" => [
            V.as_slice(),
            &[
                "a1x", "a1y", "a1z", "a2x", "a2y", "a2z", "a3x", "a3y", "a3z",
            ],
        ]
        .concat(),
        "rec" => [
            V.as_slice(),
            H.as_slice(),
            &["v1x", "v1y", "v1z", "v2x", "v2y", "v2z"],
        ]
        .concat(),
        "ell" => vec!["v1x", "v1y", "v1z", "v2x", "v2y", "v2z", "rm"],
        "wed" => [
            V.as_slice(),
            &[
                "a1x", "a1y", "a1z", "a2x", "a2y", "a2z", "a3x", "a3y", "a3z",
            ],
        ]
        .concat(),
        "rhp" | "hex" => [
            V.as_slice(),
            H.as_slice(),
            &["r1", "r2", "r3", "s1", "s2", "s3", "t1", "t2", "t3"],
        ]
        .concat(),
        _ => Vec::new(),
    };
    // Shorter forms of a type (`RCC` with fewer values, `BOX` with 9)
    // keep the first names.
    if v.len() < count {
        return Vec::new();
    }
    v.into_iter().take(count).collect()
}

/// The more general types a surface can be written as, with the same
/// surface: `PX 5` as `P 1 0 0 5`, `CZ 3` as `C/Z 0 0 3`. Offered when an
/// edit needs a coefficient the type has not got (moving a `CZ` off the
/// axis, tilting a `PX`).
pub fn general_forms(mnemonic: &str, c: &[f64]) -> Vec<(&'static str, Vec<f64>)> {
    let m = mnemonic.to_ascii_lowercase();
    match (m.as_str(), c) {
        ("px", &[d]) => vec![("P", vec![1.0, 0.0, 0.0, d])],
        ("py", &[d]) => vec![("P", vec![0.0, 1.0, 0.0, d])],
        ("pz", &[d]) => vec![("P", vec![0.0, 0.0, 1.0, d])],
        ("so", &[r]) => vec![("S", vec![0.0, 0.0, 0.0, r])],
        ("sx", &[x, r]) => vec![("S", vec![x, 0.0, 0.0, r])],
        ("sy", &[y, r]) => vec![("S", vec![0.0, y, 0.0, r])],
        ("sz", &[z, r]) => vec![("S", vec![0.0, 0.0, z, r])],
        ("cx", &[r]) => vec![("C/X", vec![0.0, 0.0, r])],
        ("cy", &[r]) => vec![("C/Y", vec![0.0, 0.0, r])],
        ("cz", &[r]) => vec![("C/Z", vec![0.0, 0.0, r])],
        ("kx", &[x, t]) => vec![("K/X", vec![x, 0.0, 0.0, t])],
        ("ky", &[y, t]) => vec![("K/Y", vec![0.0, y, 0.0, t])],
        ("kz", &[z, t]) => vec![("K/Z", vec![0.0, 0.0, z, t])],
        ("kx", &[x, t, s]) => vec![("K/X", vec![x, 0.0, 0.0, t, s])],
        ("ky", &[y, t, s]) => vec![("K/Y", vec![0.0, y, 0.0, t, s])],
        ("kz", &[z, t, s]) => vec![("K/Z", vec![0.0, 0.0, z, t, s])],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const INPUT: &str = "title
1 1 -7.8 -1 imp:n=1 $ the ball
c about the shell
2 0 1 -9 imp:n=1 u=3
     vol=12 *trcl=(0 0 1 0 90 90 90 0 90 90 90 0)
3 like 1 but mat=2 rho=-2.7
4 0 9 imp:n=0

1 so 5
9 PX 100 $ far

m1 26000 1 nlib=.80c
c water
m2 1001 2
     8016 1 $ oxygen
";

    #[test]
    fn cell_fields_read() {
        let s = Source::from_str(INPUT);
        let c = s.cell_fields(2).unwrap();
        assert_eq!(c.material, Some(0));
        assert_eq!(c.density, None);
        let keys: Vec<_> = c.params.iter().map(|p| p.label()).collect();
        assert_eq!(keys, ["IMP:N", "U", "VOL", "*TRCL"]);
        assert_eq!(c.params[3].value, "(0 0 1 0 90 90 90 0 90 90 90 0)");
        let l = s.cell_fields(3).unwrap();
        assert_eq!((l.like, l.material), (Some(1), None));
        assert!(s.per_cell_keys().is_empty());
    }

    #[test]
    fn cell_changes_keep_the_card() {
        let s = Source::from_str(INPUT);
        let t = s
            .cell_with(
                1,
                &[
                    CellChange::Material(2),
                    CellChange::Density(-1.0),
                    CellChange::Param {
                        key: "imp:n".into(),
                        value: Some("2".into()),
                    },
                    CellChange::Param {
                        key: "tmp".into(),
                        value: Some("2.5e-8".into()),
                    },
                ],
            )
            .unwrap();
        assert_eq!(t, "1 2 -1 -1 imp:n=2 tmp=2.5e-8 $ the ball\n");
        // Scaled: the void cell left void, the digits kept few.
        let many = s
            .cells_with(&[1, 2], &[CellChange::ScaleDensity(1.1)])
            .unwrap();
        assert_eq!(many[0].2, "1 1 -8.58 -1 imp:n=1 $ the ball\n");
        assert!(many[1].2.contains("2 0 1 -9 imp:n=1 u=3"), "{}", many[1].2);
        assert!(s.cells_with(&[1, 77], &[]).unwrap_err().contains("77"));
        // Void, and back.
        let t = s.cell_with(1, &[CellChange::Material(0)]).unwrap();
        assert!(t.starts_with("1 0 -1 imp:n=1"), "{t}");
        let t = s
            .cell_with(2, &[CellChange::Material(1), CellChange::Density(-7.0)])
            .unwrap();
        assert!(t.contains("2 1 -7 1 -9 imp:n=1 u=3"), "{t}");
        assert!(t.starts_with("c about the shell\n"), "{t}");
        let t = s
            .cell_with(
                2,
                &[
                    CellChange::Param {
                        key: "vol".into(),
                        value: None,
                    },
                    CellChange::Param {
                        key: "trcl".into(),
                        value: Some("(1 2 3)".into()),
                    },
                ],
            )
            .unwrap();
        assert!(t.contains("*trcl=(1 2 3)") && !t.contains("vol"), "{t}");
        // What the input keeps is the old card until it is edited.
        assert!(s.cell(1).unwrap().text.contains("1 1 -7.8"));
        let e = s.edit(
            CardKind::Cell,
            1,
            &s.cell_with(1, &[CellChange::Density(-8.0)]).unwrap(),
        );
        assert!(e.is_ok());
        assert!(s.cell(1).unwrap().text.starts_with("1 1 -8 -1"));
        assert!(s
            .cell_with(
                1,
                &[CellChange::Param {
                    key: "imp:p".into(),
                    value: Some(" ".into())
                }]
            )
            .is_err());
    }

    #[test]
    fn geometry_written_term_by_term() {
        let s = Source::from_str(INPUT);
        assert_eq!(s.cell_fields(2).unwrap().geometry, "1 -9");
        let input = "t
1 0 -1
      2 -3 imp:n=1
2 0 1 imp:n=0

1 so 1
2 px 0
3 px 1
4 py 0

";
        let src = Source::from_str(input);
        // As many terms: the card's line breaks kept.
        let t = src
            .cell_with(1, &[CellChange::Geometry("-1 2\n -4".into())])
            .unwrap();
        assert_eq!(t, "1 0 -1\n      2 -4 imp:n=1\n");
        // More terms: written again.
        let t = src
            .cell_with(1, &[CellChange::Geometry("-1 (2 : -3) #2".into())])
            .unwrap();
        let back = Source::from_str(&input.replace("1 0 -1\n      2 -3 imp:n=1\n", &t));
        assert_eq!(back.cell_fields(1).unwrap().geometry, "-1 (2 : -3) #2");
        assert!(t.contains("imp:n=1"), "{t}");
        for bad in ["-1 imp:n=2", "-1 (2", "-1 2)", "-1 x"] {
            assert!(
                src.cell_with(1, &[CellChange::Geometry(bad.into())])
                    .is_err(),
                "{bad}"
            );
        }
        // A long geometry: continuation lines, each within 128 columns.
        let long = vec!["-1 2"; 60].join(" ");
        let t = src.cell_with(1, &[CellChange::Geometry(long)]).unwrap();
        assert!(
            t.lines().all(|l| l.len() <= super::super::MAX_COLUMNS),
            "{t}"
        );
        assert!(t.lines().nth(1).unwrap().starts_with("     "), "{t}");
        let back = Source::from_str(&input.replace("1 0 -1\n      2 -3 imp:n=1\n", &t));
        let c = back.cell_fields(1).unwrap();
        assert_eq!(c.geometry.split_whitespace().count(), 120, "{}", c.geometry);
        assert_eq!(c.params.len(), 1);
    }

    /// Cell `id` of `card`, read alone: its geometry and its parameters
    /// (each value as its words: a line break in one is a blank to MCNP).
    fn read_alone(card: &str, id: i64) -> (migjorn::Expr, Vec<(String, String)>) {
        let s = Source::from_str(&format!("t\n{}\n\n1 so 1\n\n", card.trim_end()));
        let g = s
            .tree()
            .cell(id)
            .unwrap_or_else(|| panic!("cell {id} does not read:\n{card}"))
            .geometry_expr()
            .unwrap_or_else(|e| panic!("{}:\n{card}", e.message));
        let p = s
            .cell_fields(id)
            .unwrap()
            .params
            .iter()
            .map(|p| {
                let words: Vec<&str> = p.value.split_whitespace().collect();
                (p.key.clone(), words.join(" "))
            })
            .collect();
        (g, p)
    }

    fn fits(card: &str) -> bool {
        card.lines()
            .all(|l| columns(l) <= super::super::MAX_COLUMNS)
    }

    #[test]
    fn a_union_without_spaces_is_broken_at_its_colons() {
        // As the expansion of a `#n` of many surfaces is printed.
        let union: Vec<String> = (0..40).map(|k| format!("-{}", 10001 + k)).collect();
        let card = format!("1 0 -1 ({}) imp:n=1\n", union.join(":"));
        let w = wrap_long_lines(&card);
        assert!(fits(&w), "{w}");
        assert_eq!(read_alone(&w, 1), read_alone(&card, 1), "{w}");
    }

    #[test]
    fn a_density_with_an_exponent_does_not_stop_the_union_being_broken() {
        let union: Vec<String> = (0..40).map(|k| format!("-{}", 10001 + k)).collect();
        for density in ["-1.2e-3", "-1.2E-3", "-7.8", "1.0d-2"] {
            let card = format!("1 5 {density} -1 ({}) imp:n=1\n", union.join(":"));
            let w = wrap_long_lines(&card);
            assert!(fits(&w), "{density}:\n{w}");
            assert_eq!(read_alone(&w, 1), read_alone(&card, 1), "{density}:\n{w}");
        }
    }

    #[test]
    fn a_fill_range_is_never_broken_at_its_colon() {
        // The geometry is moved by one column at a time, so a break falls on
        // every place of the ranges that fit one.
        let tail = "fill=0:0 0:1 -1:1 1 2 3 4 5 6";
        for n in 20..40 {
            let card = format!("1 0 {} u=1 lat=1 {tail}\n", vec!["-1 2"; n].join(" "));
            let w = wrap_long_lines(&card);
            assert!(fits(&w), "{n}:\n{w}");
            let words = |t: &str| t.split_whitespace().map(String::from).collect::<Vec<_>>();
            for r in ["fill=0:0", "0:1", "-1:1"] {
                assert!(words(&w).contains(&r.to_string()), "{n}: {r} broken:\n{w}");
            }
            assert_eq!(words(&w), words(&card), "{n}:\n{w}");
        }
    }

    /// Parameters written without `=` (`u 1 lat 1 fill 0:0 …`): the ranges
    /// still follow a keyword, and stay whole.
    #[test]
    fn a_fill_range_without_equals_is_never_broken() {
        let tail = "u 1 lat 1 fill 0:0 0:1 -1:1 1 2 3 4 5 6";
        for n in 20..40 {
            let card = format!("1 0 {} {tail}\n", vec!["-1 2"; n].join(" "));
            let w = wrap_long_lines(&card);
            assert!(fits(&w), "{n}:\n{w}");
            let words = |t: &str| t.split_whitespace().map(String::from).collect::<Vec<_>>();
            assert_eq!(words(&w), words(&card), "{n}:\n{w}");
        }
    }

    /// A `fill=` at the end of one line: the ranges on the next are its, and
    /// whole, wherever that line is broken.
    #[test]
    fn a_fill_range_on_a_continuation_line_is_never_broken() {
        let note = "x".repeat(60);
        for n in 80..125 {
            let card = format!(
                "1 0 -1 u=1 lat=1 fill=\n{}-1:0 0:0 0:0 1 1 $ {note}\n",
                " ".repeat(n)
            );
            let w = wrap_long_lines(&card);
            let words = |t: &str| -> Vec<String> {
                t.lines()
                    .map(|l| l.split('$').next().unwrap_or(""))
                    .filter(|l| !super::super::lines::is_comment(l))
                    .flat_map(|l| l.split_whitespace().map(String::from))
                    .collect()
            };
            for r in ["-1:0", "0:0"] {
                assert!(words(&w).contains(&r.to_string()), "{n}: {r} broken:\n{w}");
            }
            assert_eq!(words(&w), words(&card), "{n}:\n{w}");
        }
    }

    #[test]
    fn a_label_stays_a_label_when_the_card_wraps() {
        let comments = |card: &str| -> Vec<String> {
            let s = Source::from_str(&format!("t\n{}\n\n1 so 1\n\n", card.trim_end()));
            let tree = s.tree();
            let c = tree.cell(1).unwrap().comments();
            c.into_iter().map(String::from).collect()
        };
        // A card of about 100 columns and a label, a term added: the label
        // goes on a line of its own, as a `$` comment, while that fits.
        let code = format!("1 0 -1 {} 12345678 imp:n=1", vec!["2"; 45].join(" "));
        for n in [1usize, 30, 100, 118, 121] {
            let note = "x".repeat(n);
            let card = format!("{code} $ {note}\n");
            let w = wrap_long_lines(&card);
            assert!(fits(&w), "{n}:\n{w}");
            assert_eq!(comments(&w), vec![note], "{n}: the label moved:\n{w}");
        }
        // Too long for a line of its own: comment lines, all its text kept.
        let note = "x".repeat(125);
        let w = wrap_long_lines(&format!("{code} $ {note}\n"));
        assert!(fits(&w), "{w}");
        assert_eq!(w.matches('x').count(), 125, "{w}");
    }

    /// A long card as people write them: comment lines above and inside it,
    /// continuation lines (five blanks, more, or a tab), some holding only a
    /// `$` comment, geometry with unions written with or without blanks and
    /// `#n`, parameters with `fill` ranges, a label.
    fn long_card() -> impl Strategy<Value = String> {
        let term = (1u32..100_000, any::<bool>())
            .prop_map(|(n, neg)| format!("{}{n}", if neg { "-" } else { "" }));
        let group = (prop::collection::vec(term, 1..12), 0..4u8).prop_map(|(t, how)| match how {
            0 => format!("({})", t.join(":")),
            1 => t.join(" "),
            2 => format!("({})", t.join(" : ")),
            _ => format!("#{}", 2 + t.len()),
        });
        let note = prop::collection::vec(prop::sample::select(vec!["x", "é", " "]), 0..140)
            .prop_map(|v| format!("$ {}", v.concat()).trim_end().to_string());
        let comment = (
            prop::sample::select(vec!["c", "C", "  c", "c\t"]),
            0..60usize,
        )
            .prop_map(|(c, n)| format!("{c} {}", "q".repeat(n)).trim_end().to_string());
        let lead = prop::sample::select(vec!["     ", "        ", "\t"]);
        // After each group: go on, a new line, a comment line, or a line
        // with only a `$` comment.
        let after = (0..6u8, lead.clone(), comment.clone(), note.clone());
        (
            prop::collection::vec(comment, 0..3),
            any::<bool>(),
            prop::collection::vec((group, after), 1..30),
            any::<bool>(),
            prop::option::of(note),
            lead,
        )
            .prop_map(|(header, material, body, lattice, label, last)| {
                let mut lines: Vec<String> = header;
                let mut cur = String::from(if material { "1 5 -7.8" } else { "1 0" });
                for (g, (how, lead, comment, note)) in body {
                    cur = format!("{cur} {g}");
                    match how {
                        0 => {
                            lines.push(std::mem::replace(&mut cur, lead.to_string()));
                        }
                        1 => {
                            lines.push(std::mem::replace(&mut cur, lead.to_string()));
                            lines.push(comment);
                        }
                        2 => {
                            lines.push(std::mem::replace(&mut cur, lead.to_string()));
                            lines.push(format!("{lead}{note}"));
                        }
                        _ => {}
                    }
                }
                let params = if lattice {
                    "imp:n=1 u=1 lat=1 fill=0:0 0:1 -1:1 1 2 3 4 5 6"
                } else {
                    "imp:n=1 vol=2"
                };
                cur = format!("{cur} {params}");
                if cur.trim().is_empty() {
                    cur = format!("{last}{params}");
                }
                if let Some(l) = label {
                    cur = format!("{cur} {l}");
                }
                lines.push(cur);
                lines.join("\n") + "\n"
            })
    }

    proptest! {
        /// `wrap_long_lines` on any such card: every line fits MCNP's
        /// columns, none is blank (that would end the cell block), the
        /// geometry and parameters read back the same, every comment line
        /// stays as it was, no letter of a comment is lost, and the labels
        /// stay labels while each fits on a line of its own.
        #[test]
        fn long_cards_wrap_to_what_mcnp_reads_the_same(card in long_card()) {
            let w = wrap_long_lines(&card);
            prop_assert!(fits(&w), "{card}\n=>\n{w}");
            prop_assert!(!w.lines().any(super::super::lines::is_blank), "{w:?}");
            prop_assert_eq!(read_alone(&w, 1), read_alone(&card, 1), "{}\n=>\n{}", card, w);
            let comments = |t: &str| -> Vec<String> {
                t.lines()
                    .filter(|l| super::super::lines::is_comment(l) && l.contains('q'))
                    .map(String::from)
                    .collect()
            };
            prop_assert_eq!(comments(&w), comments(&card), "{}", w);
            for letter in ['x', 'é', 'q'] {
                prop_assert_eq!(w.matches(letter).count(), card.matches(letter).count(), "{}", w);
            }
            let labels = |t: &str| -> Vec<String> {
                let s = Source::from_str(&format!("t\n{}\n\n1 so 1\n\n", t.trim_end()));
                let tree = s.tree();
                tree.cell(1).unwrap().comments().into_iter().map(String::from).collect()
            };
            let all_fit = labels(&card)
                .iter()
                .all(|l| columns(&format!("     $ {l}")) <= super::super::MAX_COLUMNS);
            if all_fit {
                prop_assert_eq!(labels(&w), labels(&card), "{}", w);
            }
        }
    }

    #[test]
    fn cells_cut() {
        let s = Source::from_str("cut\n1 0 -1 imp:n=1\n2 0 1 : -2 imp:n=1\n\n1 so 5\n2 so 9\n\n");
        assert_eq!(s.cell_cut(1, 7).unwrap(), "1 0 -1 #7 imp:n=1\n");
        let t = s.cell_cut(2, 7).unwrap();
        assert!(
            t.starts_with("2 0 (1 : -2) #7 imp:n=1") || t.starts_with("2 0 (1:-2) #7"),
            "{t}"
        );
        let s = Source::from_str(
            "cut
1 0 -1 trcl=(1 0 0) imp:n=1

1 so 5

",
        );
        assert!(s.cell_cut(1, 7).unwrap_err().contains("TRCL"));
    }

    #[test]
    fn cells_alone_for_the_preview() {
        let s = Source::from_str(
            "alone
1 0 -1 trcl=5 imp:n=1
2 0 -2 #1 *trcl=(1 2 3) imp:n=1
3 like 1 but imp:n=2
9 0 2 imp:n=0

1 2 so 5
2 cz 50

tr2 1 0 0
tr5 0 0 7
",
        );
        let t = s.cell_alone(2, "-2 #1 (1:-1)").unwrap();
        assert!(t.contains("2 0 -2 #1 (1:-1) *trcl=(1 2 3)\n"), "{t}");
        assert!(t.contains("\n1 0 -1 trcl=5\n"), "{t}");
        assert!(t.contains("\n2 cz 50\n1 2 so 5\n"), "{t}");
        assert!(t.contains("tr2 1 0 0") && t.contains("tr5 0 0 7"), "{t}");
        assert!(s.cell_alone(3, "-1").unwrap_err().contains("LIKE"));
        assert!(s.cell_alone(2, "-77").unwrap_err().contains("77"));
    }

    #[test]
    fn geometry_words_and_flags() {
        let w: Vec<Word> = geometry_words("-1 2.3(#4:#(5))x")
            .into_iter()
            .map(|w| w.kind)
            .collect();
        assert_eq!(
            w,
            [
                Word::Surface {
                    id: 1,
                    negative: true
                },
                Word::Surface {
                    id: 2,
                    negative: false
                },
                Word::Open,
                Word::Cell(4),
                Word::Union,
                Word::Complement,
                Word::Open,
                Word::Surface {
                    id: 5,
                    negative: false
                },
                Word::Close,
                Word::Close,
                Word::Bad,
            ]
        );
        let s = Source::from_str(INPUT);
        assert!(s.check_geometry("-1 9 #2").is_empty());
        let text = "-1 7 (#8 x";
        let f = s.check_geometry(text);
        let m: Vec<&str> = f.iter().map(|f| f.message.as_str()).collect();
        assert_eq!(m.len(), 4, "{m:?}");
        assert_eq!((m[0], &text[f[0].span.clone()]), ("no surface 7", "7"));
        assert_eq!((m[1], &text[f[1].span.clone()]), ("no cell 8", "#8"));
        assert!(!s.check_geometry("( )").is_empty());
        assert_eq!(
            s.surface_list(),
            [(1, "SO".to_string()), (9, "PX".to_string())]
        );
    }

    #[test]
    fn data_cards_for_every_cell() {
        let s = Source::from_str(&format!("{INPUT}imp:n 1 1 1 0\nvol 1 2 3 4\n"));
        assert_eq!(s.per_cell_keys(), ["imp:n", "vol"]);
    }

    #[test]
    fn material_entries_and_keywords() {
        let s = Source::from_str(INPUT);
        let m = s.material_fields(2).unwrap();
        assert_eq!(
            m.entries,
            [("1001".to_string(), 2.0), ("8016".to_string(), 1.0)]
        );
        assert!(m.keywords.is_empty());
        assert_eq!(
            s.material_fields(1).unwrap().keywords,
            [("nlib".to_string(), ".80c".to_string())]
        );
        // Fractions changed, one added, one dropped.
        let t = s
            .material_with(
                2,
                &[
                    Entry {
                        origin: Some(1),
                        zaid: "8016".into(),
                        fraction: 0.5,
                    },
                    Entry {
                        origin: None,
                        zaid: "1002".into(),
                        fraction: 1.0,
                    },
                ],
                &[("nlib".into(), Some(".31c".into()))],
            )
            .unwrap();
        let fresh = Source::from_str(&format!("t\n1 0 -1\n\n1 so 1\n\n{t}"));
        let m = fresh.material_fields(2).unwrap();
        assert_eq!(
            m.entries,
            [("8016".to_string(), 0.5), ("1002".to_string(), 1.0)]
        );
        assert_eq!(m.keywords, [("nlib".to_string(), ".31c".to_string())]);
        assert!(t.starts_with("c water\n"), "{t}");
        // Same entries: only the words changed.
        let t = s
            .material_with(
                2,
                &[
                    Entry {
                        origin: Some(0),
                        zaid: "1001".into(),
                        fraction: 2.0,
                    },
                    Entry {
                        origin: Some(1),
                        zaid: "8016".into(),
                        fraction: 1.5,
                    },
                ],
                &[],
            )
            .unwrap();
        assert_eq!(t, "c water\nm2 1001 2\n     8016 1.5 $ oxygen\n");
        let t = s
            .material_with(
                1,
                &[Entry {
                    origin: Some(0),
                    zaid: "26000".into(),
                    fraction: 1.0,
                }],
                &[("nlib".into(), None)],
            )
            .unwrap();
        assert_eq!(t.trim_end(), "m1 26000 1");
        assert!(s.material_with(2, &[], &[]).is_err());
    }

    #[test]
    fn surfaces_in_more_general_forms() {
        let s = Source::from_str(INPUT);
        let (k, c) = s.surface_coeffs(9).unwrap();
        let forms = general_forms(&k, &c);
        assert_eq!(forms, [("P", vec![1.0, 0.0, 0.0, 100.0])]);
        let t = s.surface_retyped(9, "P", &forms[0].1).unwrap();
        assert_eq!(t, "9 P 1 0 0 100 $ far\n");
        let t = s.surface_retyped(1, "S", &[0.0, 0.0, 0.0, 5.0]).unwrap();
        assert_eq!(t, "1 s 0 0 0 5\n");
        assert!(s.surface_retyped(1, "S", &[1.0]).is_err());
        assert_eq!(coefficient_names("C/Z", 3), ["x₀", "y₀", "R"]);
        assert_eq!(coefficient_names("rcc", 7).len(), 7);
        assert!(coefficient_names("box", 7).is_empty() || coefficient_names("box", 7).len() == 7);
        assert!(coefficient_names("weird", 2).is_empty());
    }
}
