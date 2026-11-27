//! What an edit writes, shown before Apply (decision 0012, phase 8): each
//! card before and after, the words removed and added marked, and what
//! runs once it is applied.

use llevant_scene::source::lines::is_comment;

use crate::theme;

/// Lines of a card shown at most on each side.
const LINES: usize = 14;
/// Words compared word by word at most (longer cards: line by line).
const WORDS: usize = 3000;

/// A card shown: its title, its text before and after.
pub type Shown = (String, String, String);

/// Words removed (in the card before) and added (after).
pub const REMOVED: egui::Color32 = egui::Color32::from_rgb(232, 96, 96);
pub const ADDED: egui::Color32 = egui::Color32::from_rgb(96, 200, 120);

/// `text` cut into words and the spaces between them (line breaks are
/// space too), which together give it back.
fn pieces(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut space = None;
    for (i, c) in text.char_indices() {
        let s = c.is_whitespace();
        if space != Some(s) {
            if i > start {
                out.push(&text[start..i]);
            }
            start = i;
            space = Some(s);
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Which pieces of `a` and of `b` are kept (a longest common subsequence
/// of the words; spaces are kept when their words are).
fn kept(a: &[&str], b: &[&str]) -> (Vec<bool>, Vec<bool>) {
    let (n, m) = (a.len(), b.len());
    let mut ka = vec![false; n];
    let mut kb = vec![false; m];
    if n * m > WORDS * WORDS / 4 {
        // Too long to compare word by word: line by line.
        return (ka, kb);
    }
    // Common prefix and suffix first (an edit changes a few words).
    let mut p = 0;
    while p < n && p < m && a[p] == b[p] {
        ka[p] = true;
        kb[p] = true;
        p += 1;
    }
    let mut s = 0;
    while s < n - p && s < m - p && a[n - 1 - s] == b[m - 1 - s] {
        ka[n - 1 - s] = true;
        kb[m - 1 - s] = true;
        s += 1;
    }
    let (a2, b2) = (&a[p..n - s], &b[p..m - s]);
    let (n2, m2) = (a2.len(), b2.len());
    let mut l = vec![0u32; (n2 + 1) * (m2 + 1)];
    let at = |i: usize, j: usize| i * (m2 + 1) + j;
    for i in (0..n2).rev() {
        for j in (0..m2).rev() {
            l[at(i, j)] = if a2[i] == b2[j] {
                l[at(i + 1, j + 1)] + 1
            } else {
                l[at(i + 1, j)].max(l[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < n2 && j < m2 {
        if a2[i] == b2[j] {
            ka[p + i] = true;
            kb[p + j] = true;
            i += 1;
            j += 1;
        } else if l[at(i + 1, j)] >= l[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    (ka, kb)
}

/// The words of `before` and `after` that differ: `(removed, added)`
/// (whitespace changes aside).
#[cfg(test)]
fn changed_words(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
    let (a, b) = (pieces(before), pieces(after));
    let (ka, kb) = kept(&a, &b);
    let pick = |p: &[&str], k: &[bool]| -> Vec<String> {
        p.iter()
            .zip(k)
            .filter(|(w, k)| !**k && !w.trim().is_empty())
            .map(|(w, _)| w.to_string())
            .collect()
    };
    (pick(&a, &ka), pick(&b, &kb))
}

/// One side of a card: its words, those not kept in `colour`.
fn side(ui: &mut egui::Ui, pieces: &[&str], kept: &[bool], colour: egui::Color32, sign: &str) {
    let font = egui::FontId::monospace(11.5);
    let plain = ui.visuals().weak_text_color();
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let mut lines = 1;
    let mut cut = false;
    job.append(sign, 0.0, egui::TextFormat::simple(font.clone(), colour));
    for (p, k) in pieces.iter().zip(kept) {
        let breaks = p.matches('\n').count();
        if lines + breaks > LINES {
            cut = true;
            break;
        }
        lines += breaks;
        let mut f = egui::TextFormat::simple(font.clone(), if *k { plain } else { colour });
        if !*k && !p.trim().is_empty() {
            f.background = colour.gamma_multiply(0.18);
        }
        let text = p.replace('\n', &format!("\n{sign}"));
        job.append(&text, 0.0, f);
    }
    if cut {
        job.append("\n  …", 0.0, egui::TextFormat::simple(font, plain));
    }
    ui.label(job);
}

/// Card `title` before and after, the words changed marked.
pub fn card(ui: &mut egui::Ui, title: &str, before: &str, after: &str) {
    let (b, a) = (before.trim_end(), after.trim_end());
    let (pb, pa) = (pieces(b), pieces(a));
    let (kb, ka) = kept(&pb, &pa);
    ui.label(theme::data(title).weak().small());
    // (A card added has no side before.)
    if !b.is_empty() {
        side(ui, &pb, &kb, REMOVED, "- ");
    }
    side(ui, &pa, &ka, ADDED, "+ ");
}

/// Cards before and after (`(title, before, after)`), and how many more
/// the edit writes than are shown.
pub fn cards(ui: &mut egui::Ui, shown: &[Shown], more: usize) {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            for (k, (t, b, a)) in shown.iter().enumerate() {
                if k > 0 {
                    ui.add_space(3.0);
                }
                card(ui, t, b, a);
            }
            if more > 0 {
                ui.label(
                    egui::RichText::new(format!(
                        "+{more} more card{}",
                        if more == 1 { "" } else { "s" }
                    ))
                    .weak(),
                );
            }
        });
}

/// What a cut or an expansion has put in: a highlight, as a marker pen does.
pub const MARK: egui::Color32 = egui::Color32::from_rgb(235, 200, 60);

/// `text` with the bytes `mark` highlighted in yellow.
pub fn marked(ui: &mut egui::Ui, text: &str, mark: std::ops::Range<usize>) {
    let font = egui::FontId::monospace(11.5);
    let plain = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let (a, b) = (mark.start.min(text.len()), mark.end.min(text.len()));
    let (a, b) = (
        (0..=a)
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0),
        (b..=text.len())
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(text.len()),
    );
    job.append(
        &text[..a],
        0.0,
        egui::TextFormat::simple(font.clone(), plain),
    );
    let mut f = egui::TextFormat::simple(font.clone(), plain);
    f.background = MARK.gamma_multiply(0.35);
    job.append(&text[a..b], 0.0, f);
    job.append(&text[b..], 0.0, egui::TextFormat::simple(font, plain));
    ui.label(job);
}

/// A card as lines for a diff: its comment lines whole, the first words
/// together (number, material, density), then one line for each factor of
/// the geometry (a group in parentheses stays whole), parameter and
/// comment. Only for showing: the card is written as it is.
pub fn display_lines(card: &str) -> Vec<String> {
    // The comment lines above the card.
    let mut lines: Vec<String> = Vec::new();
    let mut rest = card.trim_end();
    while let Some(first) = rest.lines().next().filter(|l| is_comment(l)) {
        lines.push(first.trim_end().to_string());
        rest = rest[first.len()..].trim_start_matches(['\r', '\n']);
    }
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    // `$` ends what is read of a line at any depth: a comment inside a group
    // is shown after the group, which goes on on the next line; so does a
    // comment line.
    let mut held: Vec<String> = Vec::new();
    let mut text = String::new();
    for l in rest.lines() {
        if is_comment(l) {
            if depth > 0 {
                held.push(l.trim_end().to_string());
            } else {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                tokens.push(l.trim_end().to_string());
            }
            continue;
        }
        text.clear();
        text.push_str(l);
        text.push('\n');
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '$' => {
                    let mut comment = String::from("$");
                    while let Some(&n) = chars.peek() {
                        if n == '\n' {
                            break;
                        }
                        comment.push(n);
                        chars.next();
                    }
                    if depth > 0 {
                        held.push(comment);
                    } else {
                        if !cur.is_empty() {
                            tokens.push(std::mem::take(&mut cur));
                        }
                        tokens.push(comment);
                    }
                }
                c if c.is_whitespace() && depth <= 0 => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                        tokens.append(&mut held);
                    }
                }
                c if c.is_whitespace() => {
                    if !cur.ends_with(' ') {
                        cur.push(' ');
                    }
                }
                '(' => {
                    depth += 1;
                    cur.push(c);
                }
                ')' => {
                    depth -= 1;
                    cur.push(c);
                }
                c => cur.push(c),
            }
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens.append(&mut held);
    // "1 0" or "1 2 -7.8" on the first line.
    let head = match tokens.get(1).map(String::as_str) {
        Some("0") | None => 2,
        _ => 3,
    }
    .min(tokens.len());
    if head > 0 {
        lines.push(tokens[..head].join(" "));
    }
    lines.extend(tokens.into_iter().skip(head));
    lines
}

/// One step of a line diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Keep(usize, usize),
    Del(usize),
    Add(usize),
}

/// The lines of `a` and `b` as a longest common subsequence gives them.
fn line_ops(a: &[String], b: &[String]) -> Vec<Op> {
    let (n, m) = (a.len(), b.len());
    let mut p = 0;
    while p < n && p < m && a[p] == b[p] {
        p += 1;
    }
    let mut s = 0;
    while s < n - p && s < m - p && a[n - 1 - s] == b[m - 1 - s] {
        s += 1;
    }
    let (n2, m2) = (n - p - s, m - p - s);
    let mut ops: Vec<Op> = (0..p).map(|i| Op::Keep(i, i)).collect();
    if n2 * m2 > 4_000_000 {
        // Too long to compare line by line: all replaced.
        ops.extend((p..p + n2).map(Op::Del));
        ops.extend((p..p + m2).map(Op::Add));
    } else {
        let mut l = vec![0u32; (n2 + 1) * (m2 + 1)];
        let at = |i: usize, j: usize| i * (m2 + 1) + j;
        for i in (0..n2).rev() {
            for j in (0..m2).rev() {
                l[at(i, j)] = if a[p + i] == b[p + j] {
                    l[at(i + 1, j + 1)] + 1
                } else {
                    l[at(i + 1, j)].max(l[at(i, j + 1)])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n2 || j < m2 {
            if i < n2 && j < m2 && a[p + i] == b[p + j] {
                ops.push(Op::Keep(p + i, p + j));
                i += 1;
                j += 1;
            } else if j < m2 && (i == n2 || l[at(i, j + 1)] >= l[at(i + 1, j)]) {
                ops.push(Op::Add(p + j));
                j += 1;
            } else {
                ops.push(Op::Del(p + i));
                i += 1;
            }
        }
    }
    ops.extend((0..s).map(|k| Op::Keep(n - s + k, m - s + k)));
    ops
}

/// Lines of context around a change in [`unified`].
const CONTEXT: usize = 3;

/// A unified diff (`diff -u`) of two texts as lines: a header, a `@@` line
/// for each hunk, lines with a space (kept), `-` (red) and `+` (green), the
/// words that changed marked inside a line replaced.
pub fn unified(ui: &mut egui::Ui, from: &str, to: &str, before: &[String], after: &[String]) {
    let ops = line_ops(before, after);
    let font = egui::FontId::monospace(11.5);
    let weak = ui.visuals().weak_text_color();
    ui.label(
        egui::RichText::new(format!("--- {from}"))
            .font(font.clone())
            .color(REMOVED),
    );
    ui.label(
        egui::RichText::new(format!("+++ {to}"))
            .font(font.clone())
            .color(ADDED),
    );
    // Hunks: runs of changes with CONTEXT kept lines around them.
    let changed: Vec<usize> = (0..ops.len())
        .filter(|&k| !matches!(ops[k], Op::Keep(..)))
        .collect();
    if changed.is_empty() {
        ui.label(
            egui::RichText::new("(no difference)")
                .font(font)
                .color(weak),
        );
        return;
    }
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &k in &changed {
        let (lo, hi) = (k.saturating_sub(CONTEXT), (k + CONTEXT + 1).min(ops.len()));
        match hunks.last_mut() {
            Some(h) if lo <= h.1 => h.1 = hi,
            _ => hunks.push((lo, hi)),
        }
    }
    for (lo, hi) in hunks {
        let slice = &ops[lo..hi];
        let old_start = slice
            .iter()
            .find_map(|o| match o {
                Op::Keep(i, _) | Op::Del(i) => Some(*i),
                Op::Add(_) => None,
            })
            .unwrap_or(0);
        let new_start = slice
            .iter()
            .find_map(|o| match o {
                Op::Keep(_, j) | Op::Add(j) => Some(*j),
                Op::Del(_) => None,
            })
            .unwrap_or(0);
        let old_len = slice.iter().filter(|o| !matches!(o, Op::Add(_))).count();
        let new_len = slice.iter().filter(|o| !matches!(o, Op::Del(_))).count();
        ui.label(
            egui::RichText::new(format!(
                "@@ -{},{} +{},{} @@",
                old_start + 1,
                old_len,
                new_start + 1,
                new_len
            ))
            .font(font.clone())
            .color(ui.visuals().hyperlink_color),
        );
        let mut k = 0;
        while k < slice.len() {
            match slice[k] {
                Op::Keep(i, _) => {
                    ui.label(
                        egui::RichText::new(format!("  {}", before[i]))
                            .font(font.clone())
                            .color(weak),
                    );
                    k += 1;
                }
                _ => {
                    // A block of changes: its deletions, then its additions.
                    let start = k;
                    while k < slice.len() && !matches!(slice[k], Op::Keep(..)) {
                        k += 1;
                    }
                    let dels: Vec<usize> = slice[start..k]
                        .iter()
                        .filter_map(|o| if let Op::Del(i) = o { Some(*i) } else { None })
                        .collect();
                    let adds: Vec<usize> = slice[start..k]
                        .iter()
                        .filter_map(|o| if let Op::Add(j) = o { Some(*j) } else { None })
                        .collect();
                    // Line k of each side is compared with line k of the other.
                    let pair = |k: usize| -> Option<(&str, &str)> {
                        Some((before.get(*dels.get(k)?)?, after.get(*adds.get(k)?)?))
                    };
                    for (n, &i) in dels.iter().enumerate() {
                        match pair(n) {
                            Some((x, y)) => word_line(ui, "- ", x, y, true),
                            None => word_line(ui, "- ", &before[i], "", true),
                        }
                    }
                    for (n, &j) in adds.iter().enumerate() {
                        match pair(n) {
                            Some((x, y)) => word_line(ui, "+ ", y, x, false),
                            None => word_line(ui, "+ ", &after[j], "", false),
                        }
                    }
                }
            }
        }
    }
}

/// A line of a diff, red (`removed`) or green; the words `other` (the line
/// it is compared with) does not have are marked.
fn word_line(ui: &mut egui::Ui, sign: &str, line: &str, other: &str, removed: bool) {
    let colour = if removed { REMOVED } else { ADDED };
    let font = egui::FontId::monospace(11.5);
    let (p, q) = (pieces(line), pieces(other));
    let (kept_here, _) = kept(&p, &q);
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    job.append(sign, 0.0, egui::TextFormat::simple(font.clone(), colour));
    for (w, k) in p.iter().zip(&kept_here) {
        let mut f = egui::TextFormat::simple(font.clone(), colour);
        if !*k && !other.is_empty() && !w.trim().is_empty() {
            f.background = colour.gamma_multiply(0.25);
        }
        job.append(w, 0.0, f);
    }
    ui.label(job);
}

/// What runs once an edit is applied.
pub fn after_apply(ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new(
            "Apply writes it into the input (not saved), builds again only what changed, \
             then checks there for gaps and overlaps; Undo edit takes it back.",
        )
        .weak()
        .small(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_is_split_for_a_diff_by_factors() {
        let lines = display_lines("2 0 (-2:-4) 3 (1 : -3) imp:n=1 $ the cell\n     vol=3\n");
        assert_eq!(
            lines,
            [
                "2 0",
                "(-2:-4)",
                "3",
                "(1 : -3)",
                "imp:n=1",
                "$ the cell",
                "vol=3"
            ]
        );
        // With a material: number, material and density together.
        assert_eq!(display_lines("1 5 -7.8 -1 imp:n=1")[0], "1 5 -7.8");
    }

    /// A `$` inside a group ends what is read of its line: the `)` in the
    /// comment does not close the group, which goes on on the next line.
    #[test]
    fn a_comment_inside_a_group_does_not_end_it() {
        let lines = display_lines("2 0 (-2 $ note )\n     : -4) 3 imp:n=1");
        assert_eq!(
            lines,
            ["2 0", "(-2 : -4)", "$ note )", "3", "imp:n=1"],
            "{lines:?}"
        );
    }

    /// Comment lines stay whole: those above the card first, those inside
    /// it in place (after the group they interrupt).
    #[test]
    fn comment_lines_are_shown_whole() {
        let lines = display_lines(
            "c the header\n2 0 (-2\nc inside\n     :-4) 3\nc after 3\n     imp:n=1\n",
        );
        assert_eq!(
            lines,
            [
                "c the header",
                "2 0",
                "(-2 :-4)",
                "c inside",
                "3",
                "c after 3",
                "imp:n=1"
            ],
            "{lines:?}"
        );
    }

    /// On cards as a cut writes them (wrapped, a label too long for its
    /// card become comment lines, comment lines above), every comment line is
    /// one line of the diff, as written, and every word of the code is shown.
    #[test]
    fn wrapped_cards_show_each_comment_line_whole() {
        let mut r = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = |n: u64| {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            r % n
        };
        for case in 0..300 {
            let groups: Vec<String> = (0..1 + next(30))
                .map(|_| {
                    let t: Vec<String> = (0..1 + next(10))
                        .map(|_| format!("-{}", 1 + next(99_999)))
                        .collect();
                    if next(2) == 0 {
                        format!("({})", t.join(":"))
                    } else {
                        t.join(" ")
                    }
                })
                .collect();
            let header = "c a header line\n".repeat(next(3) as usize);
            let label = "x".repeat(next(140) as usize);
            let card = format!("{header}1 0 {} imp:n=1 $ {label}\n", groups.join(" "));
            let card = llevant_scene::source::wrap_long_lines(&card);
            let shown = display_lines(&card);
            for l in card.lines().filter(|l| is_comment(l)) {
                assert!(
                    shown.iter().any(|s| s == l.trim_end()),
                    "case {case}: {l:?} split:\n{card}\n{shown:#?}"
                );
            }
            let words = |t: &[String]| -> Vec<String> {
                t.iter()
                    .filter(|l| !is_comment(l) && !l.starts_with('$'))
                    .flat_map(|l| {
                        l.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ':')
                    })
                    .filter(|w| !w.is_empty())
                    .map(String::from)
                    .collect()
            };
            let code: Vec<String> = card
                .lines()
                .map(|l| l.split('$').next().unwrap_or("").to_string())
                .collect();
            assert_eq!(words(&shown), words(&code), "case {case}:\n{card}");
        }
    }

    #[test]
    fn the_line_diff_finds_what_changed() {
        let a: Vec<String> = ["2 0", "(-2:-4)", "3", "(1:-3)", "imp:n=1"]
            .map(String::from)
            .to_vec();
        let b: Vec<String> = ["2 0", "(-2:-4)", "3", "1", "imp:n=1"]
            .map(String::from)
            .to_vec();
        let ops = line_ops(&a, &b);
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Del(_))).count(), 1);
        assert!(ops.contains(&Op::Del(3)) && ops.contains(&Op::Add(3)));
        assert_eq!(ops.iter().filter(|o| matches!(o, Op::Keep(..))).count(), 4);
        // Nothing changed: only kept lines.
        assert!(line_ops(&a, &a).iter().all(|o| matches!(o, Op::Keep(..))));
    }

    #[test]
    fn words_changed() {
        let (r, a) = changed_words(
            "1 1 -7.8 -1 imp:n=1 $ ball\n",
            "1 2 -7.8 -1 imp:n=1 vol=3 $ ball\n",
        );
        assert_eq!(r, ["1"]);
        assert_eq!(a, ["2", "vol=3"]);
        let (r, a) = changed_words("1 so 5", "1  so\n     5");
        assert!(r.is_empty() && a.is_empty(), "spacing only");
        assert_eq!(pieces("a  b\nc").concat(), "a  b\nc");
        // A cell cut: nothing removed, the `#3` added.
        let (r, a) = changed_words("2 0 (-2:-4) 3 imp:n=1\n", "2 0 (-2:-4) 3 #3 imp:n=1\n");
        assert!(r.is_empty(), "{r:?}");
        assert_eq!(a, ["#3"]);
    }
}
