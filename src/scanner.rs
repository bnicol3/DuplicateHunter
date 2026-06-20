//! Filesystem traversal — collects FileEntry records from one or more root paths.

use std::path::PathBuf;
use std::time::SystemTime;
use walkdir::WalkDir;
use anyhow::Result;

/// Every file discovered during a scan.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub extension: String,
    pub kind: FileKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileKind {
    Image,
    Video,
    Other,
}

impl FileKind {
    pub fn from_ext(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "tif"
            | "ico" | "heic" | "heif" | "avif" | "jxl" => FileKind::Image,
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v"
            | "mpg" | "mpeg" | "3gp" | "ts" | "mts" | "m2ts" => FileKind::Video,
            _ => FileKind::Other,
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            FileKind::Image => "🖼",
            FileKind::Video => "🎬",
            FileKind::Other => "📄",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub roots: Vec<PathBuf>,
    pub recursive: bool,
    pub skip_hidden: bool,
    pub min_size_bytes: u64,
    pub compare_images_perceptually: bool,
    pub compare_videos_perceptually: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            recursive: true,
            skip_hidden: true,
            min_size_bytes: 1,
            compare_images_perceptually: true,
            compare_videos_perceptually: true,
        }
    }
}

/// Walk the given roots and collect all qualifying files.
/// `progress_cb` receives (files_found_so_far, current_file_path).
pub fn collect_files<F>(opts: &ScanOptions, mut progress_cb: F) -> Result<Vec<FileEntry>>
where
    F: FnMut(usize, &str),
{
    let mut entries: Vec<FileEntry> = Vec::new();

    for root in &opts.roots {
        let max_depth = if opts.recursive { usize::MAX } else { 1 };
        let walker = WalkDir::new(root)
            .max_depth(max_depth)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if opts.skip_hidden {
                    !e.file_name().to_string_lossy().starts_with('.')
                } else {
                    true
                }
            });

        for entry in walker.flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path().to_path_buf();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let size = meta.len();
            if size < opts.min_size_bytes {
                continue;
            }
            let extension = path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let kind = FileKind::from_ext(&extension);
            let modified = meta.modified().ok();

            progress_cb(entries.len() + 1, &path.to_string_lossy());
            entries.push(FileEntry {
                path,
                size,
                modified,
                extension,
                kind,
            });
        }
    }

    Ok(entries)
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut val = bytes as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", val, UNITS[unit])
    }
}
