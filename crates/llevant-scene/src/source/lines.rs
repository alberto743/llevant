// SPDX-FileCopyrightText: 2026 Alberto Previti
// SPDX-License-Identifier: EUPL-1.2
//! MCNP's rules for a line, as migjorn reads them: columns, not bytes or
//! characters (a tab moves to the next multiple of 8), a comment line by
//! its first five columns, and a blank line (which ends a block). One place
//! for each rule, so that what is written and what is shown read a line as
//! MCNP does.

/// The 1-based column after the last character of `s` (`columns("ab")` is
/// 2): a tab moves to the next multiple of 8, any other character is one.
pub fn columns(s: &str) -> usize {
    s.chars().fold(
        0,
        |col, c| if c == '\t' { col / 8 * 8 + 8 } else { col + 1 },
    )
}

/// The 1-based column of the first character of `line` that is not a blank
/// (`None` for a blank line), and its byte index.
fn first_content(line: &str) -> Option<(usize, usize)> {
    let i = line.find(|c: char| c != ' ' && c != '\t')?;
    Some((columns(&line[..i]) + 1, i))
}

/// A blank line: nothing but blanks (and the line's end). It ends a block of
/// the input.
pub fn is_blank(line: &str) -> bool {
    first_content(line.trim_end_matches(['\r', '\n'])).is_none()
}

/// A comment line: `c` or `C` within the first five columns, alone or
/// followed by a blank. (A `c` in column 6 or later continues a card.)
pub fn is_comment(line: &str) -> bool {
    let body = line.trim_end_matches(['\r', '\n']);
    let Some((col, i)) = first_content(body) else {
        return false;
    };
    col <= 5
        && matches!(body.as_bytes()[i], b'c' | b'C')
        && body[i + 1..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_expand_tabs_to_the_next_multiple_of_eight() {
        assert_eq!(columns(""), 0);
        assert_eq!(columns("abc"), 3);
        assert_eq!(columns("\t"), 8);
        assert_eq!(columns("ab\tc"), 9);
        assert_eq!(columns("café"), 4);
    }

    /// The cases of migjorn's own rule (`migjorn_syntax::lines`).
    #[test]
    fn comment_lines_respect_the_five_column_rule() {
        assert!(is_comment("c a comment\n"));
        assert!(is_comment("C a comment\r\n"));
        assert!(is_comment("    c indented but still column 5\n"));
        assert!(is_comment("c\n"));
        assert!(is_comment("c"));
        assert!(is_comment("c\tafter a tab"));
        assert!(!is_comment("     c this continues the previous card\n"));
        assert!(!is_comment("cx 1 2 3\n"));
        // A tab takes the `c` to column 9: a continuation.
        assert!(!is_comment("\tc not a comment\n"));
        assert!(!is_comment("   \n"));
    }

    #[test]
    fn blank_lines() {
        assert!(is_blank(""));
        assert!(is_blank("   \t \r\n"));
        assert!(!is_blank("c\n"));
        assert!(!is_blank("     1\n"));
    }
}
