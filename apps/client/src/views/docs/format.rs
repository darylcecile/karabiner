//! Markdown formatting operations applied by the document toolbar.
//! Each returns the new text and the selection to restore (byte offsets).

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Body,
    Heading1,
    Heading2,
    Heading3,
    Quote,
    Bullet,
    Numbered,
    Checklist,
}

impl Block {
    pub const MENU: [Block; 8] = [
        Block::Body,
        Block::Heading1,
        Block::Heading2,
        Block::Heading3,
        Block::Quote,
        Block::Bullet,
        Block::Numbered,
        Block::Checklist,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Block::Body => "Body text",
            Block::Heading1 => "Heading 1",
            Block::Heading2 => "Heading 2",
            Block::Heading3 => "Heading 3",
            Block::Quote => "Quote",
            Block::Bullet => "Bulleted list",
            Block::Numbered => "Numbered list",
            Block::Checklist => "Checklist",
        }
    }

    fn prefix(self, index: usize) -> String {
        match self {
            Block::Body => String::new(),
            Block::Heading1 => "# ".into(),
            Block::Heading2 => "## ".into(),
            Block::Heading3 => "### ".into(),
            Block::Quote => "> ".into(),
            Block::Bullet => "- ".into(),
            Block::Numbered => format!("{}. ", index + 1),
            Block::Checklist => "- [ ] ".into(),
        }
    }
}

pub struct Edit {
    pub text: String,
    pub selection: Range<usize>,
}

/// Splits a line into its block kind and the length of its markdown prefix.
pub fn line_block(line: &str) -> (Block, usize) {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let checks = [
        ("- [ ] ", Block::Checklist),
        ("- [x] ", Block::Checklist),
        ("- [X] ", Block::Checklist),
        ("### ", Block::Heading3),
        ("## ", Block::Heading2),
        ("# ", Block::Heading1),
        ("> ", Block::Quote),
        ("- ", Block::Bullet),
        ("* ", Block::Bullet),
    ];
    if let Some((prefix, block)) = checks.iter().find(|(prefix, _)| trimmed.starts_with(prefix)) {
        return (*block, indent + prefix.len());
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && trimmed[digits..].starts_with(". ") {
        return (Block::Numbered, indent + digits + 2);
    }
    (Block::Body, 0)
}

/// Byte range of the whole lines touched by `selection`.
fn line_span(text: &str, selection: &Range<usize>) -> Range<usize> {
    let start = text[..selection.start].rfind('\n').map_or(0, |ix| ix + 1);
    let end_probe = selection.end.max(selection.start);
    let end = text[end_probe..].find('\n').map_or(text.len(), |ix| end_probe + ix);
    start..end
}

pub fn block_at(text: &str, offset: usize) -> Block {
    let offset = offset.min(text.len());
    let span = line_span(text, &(offset..offset));
    line_block(&text[span]).0
}

/// Sets every touched line to `block`; applying the current block again reverts to body text.
pub fn set_block(text: &str, selection: Range<usize>, block: Block) -> Edit {
    let span = line_span(text, &selection);
    let lines: Vec<&str> = text[span.clone()].split('\n').collect();
    let all_same = lines.iter().all(|line| line_block(line).0 == block);
    let target = if all_same && block != Block::Body { Block::Body } else { block };
    let rewritten: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let (_, prefix_len) = line_block(line);
            format!("{}{}", target.prefix(index), &line[prefix_len..])
        })
        .collect();
    let replacement = rewritten.join("\n");
    let end = span.start + replacement.len();
    Edit {
        text: format!("{}{}{}", &text[..span.start], replacement, &text[span.end..]),
        selection: end..end,
    }
}

/// The word around `offset` (letters, digits, `_`, `'`), or an empty range in whitespace.
fn word_at(text: &str, offset: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '\'';
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(offset, |(ix, _)| ix);
    let end = text[offset..].char_indices().find(|(_, c)| !is_word(*c)).map_or(text.len(), |(ix, _)| offset + ix);
    start..end
}

/// Selection without surrounding whitespace: Markdown emphasis can't start or end on a
/// space (`** word**` stays literal), so spaces stay outside the markers.
fn trim_selection(text: &str, selection: Range<usize>) -> Range<usize> {
    let inner = &text[selection.clone()];
    let start = selection.start + (inner.len() - inner.trim_start().len());
    let end = (selection.end - (inner.len() - inner.trim_end().len())).max(start);
    start..end
}

/// Wraps the selection in `marker` (e.g. `**`), or removes it when already wrapped.
/// An empty selection formats the word under the caret, or inserts an empty pair with
/// the caret inside when the caret is between words.
pub fn toggle_inline(text: &str, selection: Range<usize>, marker: &str) -> Edit {
    if selection.is_empty() {
        let word = word_at(text, selection.start);
        if word.is_empty() {
            return toggle_range(text, word, marker);
        }
        // Format the caret's word but keep the caret collapsed at the same spot in it, so
        // typing continues inside the formatting (`one|` → `**one|**`).
        let edit = toggle_range(text, word.clone(), marker);
        let shift = edit.selection.start as isize - word.start as isize;
        let caret = (selection.start as isize + shift) as usize;
        let caret = caret.clamp(edit.selection.start, edit.selection.end);
        return Edit { text: edit.text, selection: caret..caret };
    }
    toggle_range(text, trim_selection(text, selection), marker)
}

fn toggle_range(text: &str, selection: Range<usize>, marker: &str) -> Edit {
    let Range { start, end } = selection;
    let before = &text[..start];
    let after = &text[end..];
    if start != end && before.ends_with(marker) && after.starts_with(marker) {
        let inner = &text[start..end];
        let new_start = start - marker.len();
        return Edit {
            text: format!("{}{}{}", &before[..new_start], inner, &after[marker.len()..]),
            selection: new_start..new_start + inner.len(),
        };
    }
    let inner = &text[start..end];
    if inner.len() >= marker.len() * 2 && inner.starts_with(marker) && inner.ends_with(marker) {
        let stripped = &inner[marker.len()..inner.len() - marker.len()];
        return Edit {
            text: format!("{before}{stripped}{after}"),
            selection: start..start + stripped.len(),
        };
    }
    let new_start = start + marker.len();
    Edit {
        text: format!("{before}{marker}{inner}{marker}{after}"),
        selection: new_start..new_start + inner.len(),
    }
}

/// Turns the selection into `[selection](url)` and selects the URL placeholder.
pub fn insert_link(text: &str, selection: Range<usize>) -> Edit {
    let label = if selection.is_empty() { "link text" } else { &text[selection.clone()] };
    let url = "https://";
    let link = format!("[{label}]({url})");
    let url_start = selection.start + label.len() + 3;
    Edit {
        text: format!("{}{}{}", &text[..selection.start], link, &text[selection.end..]),
        selection: url_start..url_start + url.len(),
    }
}

/// Inserts a small table on its own lines after the current line.
pub fn insert_table(text: &str, selection: Range<usize>) -> Edit {
    let span = line_span(text, &selection);
    let table = "| Column | Column |\n| --- | --- |\n| Cell | Cell |";
    let lead = if text[span.clone()].trim().is_empty() { "" } else { "\n\n" };
    let insert_at = span.end;
    let first_cell = insert_at + lead.len() + 2;
    Edit {
        text: format!("{}{lead}{table}\n{}", &text[..insert_at], &text[insert_at..]),
        selection: first_cell..first_cell + "Column".len(),
    }
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_toggle_round_trips() {
        let edit = set_block("one\ntwo", 0..5, Block::Numbered);
        assert_eq!(edit.text, "1. one\n2. two");
        let back = set_block(&edit.text, 0..edit.text.len(), Block::Numbered);
        assert_eq!(back.text, "one\ntwo");
    }

    #[test]
    fn heading_replaces_list_prefix() {
        assert_eq!(set_block("- [ ] task", 3..3, Block::Heading2).text, "## task");
    }

    #[test]
    fn inline_marker_toggles() {
        let bold = toggle_inline("a word b", 2..6, "**");
        assert_eq!(bold.text, "a **word** b");
        let plain = toggle_inline(&bold.text, bold.selection, "**");
        assert_eq!(plain.text, "a word b");
        // Spaces stay outside the markers; a caret inside a word formats that word.
        assert_eq!(toggle_inline("a  word b", 1..7, "**").text, "a  **word** b");
        assert_eq!(toggle_inline("a word b", 4..4, "**").text, "a **word** b");
        // Caret at a word's end stays collapsed inside the markers, so typed text is bold.
        let caret = toggle_inline("- one", 5..5, "**");
        assert_eq!(caret.selection, 7..7);
        let typed = format!("{} BoldQA{}", &caret.text[..7], &caret.text[7..]);
        assert_eq!(typed, "- **one BoldQA**");
        let empty = toggle_inline("a  b", 2..2, "**");
        assert_eq!((empty.text.as_str(), empty.selection), ("a **** b", 4..4));
    }
}
