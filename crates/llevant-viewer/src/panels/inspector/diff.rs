//! What an edit writes, shown before Apply (decision 0012, phase 8): each
//! card before and after, the words removed and added marked, and what
//! runs once it is applied.

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
