//! Lossless block model for the formatted editor: Markdown is split into blocks whose
//! `source` and `trailing` separators concatenate back to the exact original text.

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// The block's Markdown, without the line break that ends it.
    pub source: String,
    /// Line break(s) and blank lines that follow the block.
    pub trailing: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Heading(u8),
    Code,
    Other,
}

impl Block {
    pub fn kind(&self) -> BlockKind {
        let first = self.source.lines().next().unwrap_or_default().trim_start();
        if first.starts_with("```") {
            return BlockKind::Code;
        }
        let hashes = first.bytes().take_while(|b| *b == b'#').count();
        if (1..=6).contains(&hashes) && first[hashes..].starts_with(' ') {
            return BlockKind::Heading(hashes as u8);
        }
        BlockKind::Other
    }
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_heading(line: &str) -> bool {
    Block { source: line.to_owned(), trailing: String::new() }.kind() != BlockKind::Other
        && !line.trim_start().starts_with("```")
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// Byte spans of each line, excluding the `\n`.
fn line_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (ix, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            spans.push((start, ix));
            start = ix + 1;
        }
    }
    if start < text.len() {
        spans.push((start, text.len()));
    }
    spans
}

/// Headings and fenced code are their own blocks; other runs of non-blank lines
/// (paragraphs, lists, quotes, tables) form one block each.
pub fn split(text: &str) -> Vec<Block> {
    let lines = line_spans(text);
    let line = |ix: usize| &text[lines[ix].0..lines[ix].1];
    let mut blocks = Vec::new();
    let mut ix = 0;
    let mut lead_end = 0;
    while ix < lines.len() && is_blank(line(ix)) {
        lead_end = (lines[ix].1 + 1).min(text.len());
        ix += 1;
    }
    if lead_end > 0 {
        blocks.push(Block { source: String::new(), trailing: text[..lead_end].to_owned() });
    }
    while ix < lines.len() {
        let start = lines[ix].0;
        let mut last = ix;
        if is_fence(line(ix)) {
            while last + 1 < lines.len() {
                last += 1;
                if is_fence(line(last)) {
                    break;
                }
            }
        } else if !is_heading(line(ix)) {
            while last + 1 < lines.len() && !is_blank(line(last + 1)) && !is_heading(line(last + 1)) && !is_fence(line(last + 1)) {
                last += 1;
            }
        }
        let end = lines[last].1;
        ix = last + 1;
        while ix < lines.len() && is_blank(line(ix)) {
            ix += 1;
        }
        let trailing_end = if ix < lines.len() { lines[ix].0 } else { text.len() };
        blocks.push(Block { source: text[start..end].to_owned(), trailing: text[end..trailing_end].to_owned() });
    }
    blocks
}

pub fn join(blocks: &[Block]) -> String {
    blocks.iter().flat_map(|block| [block.source.as_str(), block.trailing.as_str()]).collect()
}

/// Re-splits block `index` after an edit (typing a blank line, `## `, or a fence can change
/// its structure). Lossless: the new blocks join back to exactly the edited source followed by
/// the block's original separator.
pub fn resplit(blocks: &mut Vec<Block>, index: usize) -> usize {
    let Some(block) = blocks.get(index).cloned() else { return 0 };
    let mut parts = split(&block.source);
    match parts.last_mut() {
        Some(last) => last.trailing.push_str(&block.trailing),
        None => parts.push(Block { source: String::new(), trailing: block.trailing.clone() }),
    }
    let count = parts.len();
    blocks.splice(index..=index, parts);
    count
}

#[cfg(test)]
mod tests {
    use super::{BlockKind, join, resplit, split};

    #[test]
    fn split_join_is_lossless() {
        for text in [
            "",
            "\n\n# Title\n\nPara one\nstill one\n\n- a\n- b\n\n```rs\nfn x() {}\n\n```\ntrailing",
            "## A\n## B\n| a | b |\n| - | - |\n",
            "text without newline",
        ] {
            assert_eq!(join(&split(text)), text);
        }
        let blocks = split("# T\n\nbody\n");
        assert_eq!(blocks[0].kind(), BlockKind::Heading(1));
        assert_eq!(blocks[1].source, "body");
    }

    #[test]
    fn blank_line_in_edited_block_splits_it() {
        let mut blocks = split("one\n\nthree\n");
        blocks[0].source = "one\n\ntwo".into();
        assert_eq!(resplit(&mut blocks, 0), 2);
        assert_eq!(join(&blocks), "one\n\ntwo\n\nthree\n");
        // Authored leading/trailing blank lines survive the active-block commit.
        let edited = "\n\n## Head\nbody\n\n";
        blocks[0].source = edited.into();
        resplit(&mut blocks, 0);
        assert_eq!(join(&blocks), format!("{edited}\n\ntwo\n\nthree\n"));
    }
}
