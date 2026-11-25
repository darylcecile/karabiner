//! Changed-file index for a tour and patch normalisation for the Kit diff viewer.

use protocol::{CodeTour, FileChangeStatus, TourBlockContent, TourCommit};

/// One stored patch for one file, with where it came from (a chapter block or a commit).
#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub key: String,
    pub path: String,
    pub origin: String,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
}

pub fn count_changes(patch: &str) -> (usize, usize) {
    patch.lines().fold((0, 0), |(add, del), line| {
        if line.starts_with("+++") || line.starts_with("---") {
            (add, del)
        } else if line.starts_with('+') {
            (add + 1, del)
        } else if line.starts_with('-') {
            (add, del + 1)
        } else {
            (add, del)
        }
    })
}

/// Every chapter diff block, in tour order. Nothing is deduplicated: two chapters that
/// show different hunks of the same file both keep their patch.
pub fn chapter_entries(tour: &CodeTour) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for (index, chapter) in tour.chapters.iter().enumerate() {
        for block in &chapter.blocks {
            if let Some(TourBlockContent::Diff { path, patch, .. }) = block.content() {
                let patch = normalize_patch(&path, &FileChangeStatus::Modified, &patch);
                let (additions, deletions) = count_changes(&patch);
                entries.push(FileEntry {
                    key: block_key(&chapter.id, &block.id),
                    origin: format!("{:02} · {}", index + 1, chapter.title),
                    path,
                    additions,
                    deletions,
                    patch,
                });
            }
        }
    }
    entries
}

/// Cache/focus key for a chapter block; block ids are only unique within a chapter.
pub fn block_key(chapter_id: &str, block_id: &str) -> String {
    format!("block:{chapter_id}:{block_id}")
}

/// The files of one commit, each with its own stored patch and counts.
pub fn commit_entries(commit: &TourCommit) -> Vec<FileEntry> {
    commit
        .files
        .iter()
        .map(|file| FileEntry {
            key: format!("commit:{}:{}", commit.sha, file.path),
            path: file.path.clone(),
            origin: short_sha(&commit.sha).to_owned(),
            additions: file.additions as usize,
            deletions: file.deletions as usize,
            patch: normalize_patch(&file.path, &file.status, &file.patch),
        })
        .collect()
}

/// Distinct paths touched anywhere in the tour (chapters and commits), for review counts.
pub fn changed_paths(tour: &CodeTour) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let commits = tour.commits.iter().flatten().flat_map(commit_entries);
    for entry in chapter_entries(tour).into_iter().chain(commits) {
        if !paths.contains(&entry.path) {
            paths.push(entry.path);
        }
    }
    paths
}

/// Adds Git file headers when a stored patch holds only hunks.
pub fn normalize_patch(path: &str, status: &FileChangeStatus, patch: &str) -> String {
    let trimmed = patch.trim_start();
    if trimmed.starts_with("diff ") || trimmed.starts_with("--- ") {
        return patch.to_owned();
    }
    let (old, new) = match status {
        FileChangeStatus::Added => ("/dev/null".to_owned(), format!("b/{path}")),
        FileChangeStatus::Deleted => (format!("a/{path}"), "/dev/null".to_owned()),
        _ => (format!("a/{path}"), format!("b/{path}")),
    };
    let body = if patch.ends_with('\n') { patch.to_owned() } else { format!("{patch}\n") };
    format!("diff --git a/{path} b/{path}\n--- {old}\n+++ {new}\n{body}")
}

/// One patch containing every file of a commit.
pub fn commit_patch(commit: &TourCommit) -> String {
    commit
        .files
        .iter()
        .map(|file| with_newline(normalize_patch(&file.path, &file.status, &file.patch)))
        .collect()
}

fn with_newline(patch: String) -> String {
    if patch.ends_with('\n') { patch } else { patch + "\n" }
}

pub fn file_name(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(ix) => (&path[..=ix], &path[ix + 1..]),
        None => ("", path),
    }
}

pub fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_only_patch_gets_headers() {
        let patch = normalize_patch("src/a.rs", &FileChangeStatus::Modified, "@@ -1 +1 @@\n-a\n+b");
        assert!(patch.starts_with("diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@"));
        assert_eq!(count_changes(&patch), (1, 1));
    }
}
