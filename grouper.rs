//! Merges exact-hash groups, perceptual image groups, and perceptual video groups
//! into a unified list of DuplicateGroup records for display.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::scanner::{FileEntry, FileKind, format_size};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchKind {
    /// Byte-for-byte identical (same XXH3 hash).
    ExactHash,
    /// Visually identical images (dHash within threshold), possibly different formats.
    PerceptualImage,
    /// Visually identical video content (average frame dHash within threshold).
    PerceptualVideo,
}

impl MatchKind {
    pub fn label(&self) -> &'static str {
        match self {
            MatchKind::ExactHash       => "Exact match",
            MatchKind::PerceptualImage => "Visual match",
            MatchKind::PerceptualVideo => "Video match",
        }
    }
    pub fn icon(&self) -> &'static str {
        match self {
            MatchKind::ExactHash       => "≡",
            MatchKind::PerceptualImage => "👁",
            MatchKind::PerceptualVideo => "🎬",
        }
    }
    /// Returns (r, g, b) for this match kind.
    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            MatchKind::ExactHash       => (100, 160, 255),
            MatchKind::PerceptualImage => (255, 180,  80),
            MatchKind::PerceptualVideo => (180, 120, 255),
        }
    }
}

/// Mark applied to a single file within a group.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FileMark {
    #[default]
    None,
    Keep,
    Delete,
    #[allow(dead_code)]
    Quarantine,
}

/// Metadata + mark for one file in a duplicate group.
#[derive(Debug, Clone)]
pub struct DupeFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    #[allow(dead_code)]
    pub extension: String,
    pub kind: FileKind,
    pub mark: FileMark,
}

impl DupeFile {
    pub fn size_str(&self) -> String {
        format_size(self.size)
    }

    pub fn modified_str(&self) -> String {
        match self.modified {
            None => "Unknown".to_string(),
            Some(t) => {
                let secs = t
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                // Simple formatting: YYYY-MM-DD
                let days = secs / 86400;
                let y = 1970 + days / 365;
                let rem = days % 365;
                let m = rem / 30 + 1;
                let d = rem % 30 + 1;
                format!("{:04}-{:02}-{:02}", y, m.min(12), d.min(31))
            }
        }
    }
}

/// One group of duplicate files.
#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    pub id: usize,
    pub kind: MatchKind,
    /// Similarity 0.0–1.0 (1.0 = byte-identical).
    pub similarity: f32,
    pub files: Vec<DupeFile>,
    /// Collapsed in the UI?
    pub collapsed: bool,
}

impl DuplicateGroup {
    pub fn wasted_bytes(&self) -> u64 {
        let size = self.files.first().map(|f| f.size).unwrap_or(0);
        size.saturating_mul((self.files.len() as u64).saturating_sub(1))
    }

    pub fn wasted_str(&self) -> String {
        format_size(self.wasted_bytes())
    }

    #[allow(dead_code)]
    pub fn marked_delete_count(&self) -> usize {
        self.files.iter().filter(|f| f.mark == FileMark::Delete).count()
    }

    #[allow(dead_code)]
    pub fn marked_keep_count(&self) -> usize {
        self.files.iter().filter(|f| f.mark == FileMark::Keep).count()
    }

    /// Mark the newest file as Keep, all others as Delete.
    pub fn auto_keep_newest(&mut self) {
        let newest_idx = self.files
            .iter()
            .enumerate()
            .max_by_key(|(_, f)| f.modified.unwrap_or(SystemTime::UNIX_EPOCH))
            .map(|(i, _)| i)
            .unwrap_or(0);
        for (i, f) in self.files.iter_mut().enumerate() {
            f.mark = if i == newest_idx { FileMark::Keep } else { FileMark::Delete };
        }
    }

    /// Mark the oldest file as Keep, all others as Delete.
    pub fn auto_keep_oldest(&mut self) {
        let oldest_idx = self.files
            .iter()
            .enumerate()
            .min_by_key(|(_, f)| f.modified.unwrap_or(SystemTime::UNIX_EPOCH))
            .map(|(i, _)| i)
            .unwrap_or(0);
        for (i, f) in self.files.iter_mut().enumerate() {
            f.mark = if i == oldest_idx { FileMark::Keep } else { FileMark::Delete };
        }
    }

    pub fn clear_marks(&mut self) {
        for f in &mut self.files {
            f.mark = FileMark::None;
        }
    }
}

/// Build the full list of DuplicateGroup from all three sources.
pub fn build_groups(
    entries: &[FileEntry],
    exact_groups: HashMap<crate::hasher::FileHash, Vec<PathBuf>>,
    perceptual_image_groups: Vec<(Vec<PathBuf>, f32)>,
    perceptual_video_groups: Vec<(Vec<PathBuf>, f32)>,
) -> Vec<DuplicateGroup> {
    // Index entries by path for fast lookup
    let entry_map: HashMap<PathBuf, &FileEntry> =
        entries.iter().map(|e| (e.path.clone(), e)).collect();

    let lookup = |path: &PathBuf| -> DupeFile {
        if let Some(e) = entry_map.get(path) {
            DupeFile {
                path: e.path.clone(),
                size: e.size,
                modified: e.modified,
                extension: e.extension.clone(),
                kind: e.kind.clone(),
                mark: FileMark::None,
            }
        } else {
            DupeFile {
                path: path.clone(),
                size: 0,
                modified: None,
                extension: path.extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                kind: FileKind::Other,
                mark: FileMark::None,
            }
        }
    };

    let mut groups = Vec::new();
    let mut id = 0usize;

    // Exact hash groups — sorted largest first
    let mut exact: Vec<_> = exact_groups.into_iter().collect();
    exact.sort_by(|a, b| {
        let sa = entry_map.get(&a.1[0]).map(|e| e.size).unwrap_or(0);
        let sb = entry_map.get(&b.1[0]).map(|e| e.size).unwrap_or(0);
        sb.cmp(&sa)
    });
    for (_, paths) in exact {
        let files = paths.iter().map(lookup).collect();
        groups.push(DuplicateGroup {
            id,
            kind: MatchKind::ExactHash,
            similarity: 1.0,
            files,
            collapsed: false,
        });
        id += 1;
    }

    // Perceptual image groups
    for (paths, sim) in perceptual_image_groups {
        let files = paths.iter().map(lookup).collect();
        groups.push(DuplicateGroup {
            id,
            kind: MatchKind::PerceptualImage,
            similarity: sim,
            files,
            collapsed: false,
        });
        id += 1;
    }

    // Perceptual video groups
    for (paths, sim) in perceptual_video_groups {
        let files = paths.iter().map(lookup).collect();
        groups.push(DuplicateGroup {
            id,
            kind: MatchKind::PerceptualVideo,
            similarity: sim,
            files,
            collapsed: false,
        });
        id += 1;
    }

    groups
}

/// Summary stats for the results panel header.
pub struct ScanStats {
    pub files_scanned: usize,
    pub groups_found: usize,
    pub total_wasted: u64,
    pub marked_delete_count: usize,
    pub marked_delete_bytes: u64,
}

impl ScanStats {
    pub fn from_groups(scanned: usize, groups: &[DuplicateGroup]) -> Self {
        let total_wasted = groups.iter().map(|g| g.wasted_bytes()).sum();
        let marked_delete_count = groups.iter()
            .flat_map(|g| &g.files)
            .filter(|f| f.mark == FileMark::Delete)
            .count();
        let marked_delete_bytes = groups.iter()
            .flat_map(|g| &g.files)
            .filter(|f| f.mark == FileMark::Delete)
            .map(|f| f.size)
            .sum();
        ScanStats {
            files_scanned: scanned,
            groups_found: groups.len(),
            total_wasted,
            marked_delete_count,
            marked_delete_bytes,
        }
    }
}
