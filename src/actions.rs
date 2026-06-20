//! File actions: delete, quarantine (move), hardlink.

use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    Delete,
    Quarantine(PathBuf),
    #[allow(dead_code)]
    Hardlink(PathBuf), // replace duplicate with hardlink to original
}

#[derive(Debug)]
pub struct ActionResult {
    #[allow(dead_code)]
    pub path: PathBuf,
    pub success: bool,
    pub message: String,
}

/// Permanently delete a file.
pub fn delete_file(path: &Path) -> Result<()> {
    fs::remove_file(path)
        .with_context(|| format!("Failed to delete {}", path.display()))
}

/// Move a file to the quarantine directory, preserving its filename.
/// If a file with the same name already exists, appends a counter.
pub fn quarantine_file(path: &Path, quarantine_root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(quarantine_root)
        .with_context(|| format!("Cannot create quarantine dir {}", quarantine_root.display()))?;

    let fname = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Path has no filename: {}", path.display()))?;

    let dest = unique_path(&quarantine_root.join(fname));

    // Try rename first (fast, same filesystem); fall back to copy+delete
    if fs::rename(path, &dest).is_err() {
        fs::copy(path, &dest)
            .with_context(|| format!("Copy failed: {} → {}", path.display(), dest.display()))?;
        fs::remove_file(path)
            .with_context(|| format!("Cleanup failed after copy: {}", path.display()))?;
    }

    Ok(dest)
}

/// Replace `duplicate` with a hardlink pointing to `original`.
/// Both files must be on the same filesystem.
pub fn hardlink_file(original: &Path, duplicate: &Path) -> Result<()> {
    fs::remove_file(duplicate)
        .with_context(|| format!("Cannot remove duplicate: {}", duplicate.display()))?;
    fs::hard_link(original, duplicate)
        .with_context(|| format!(
            "Cannot hardlink {} → {}",
            duplicate.display(), original.display()
        ))
}

/// Execute an action on a single file; returns a result record.
pub fn execute_action(path: &Path, kind: &ActionKind) -> ActionResult {
    let result = match kind {
        ActionKind::Delete => {
            delete_file(path).map(|_| format!("Deleted {}", path.display()))
        }
        ActionKind::Quarantine(root) => {
            quarantine_file(path, root)
                .map(|dest| format!("Moved to {}", dest.display()))
        }
        ActionKind::Hardlink(original) => {
            hardlink_file(original, path)
                .map(|_| format!("Hardlinked to {}", original.display()))
        }
    };

    match result {
        Ok(msg) => ActionResult { path: path.to_path_buf(), success: true,  message: msg },
        Err(e)  => ActionResult { path: path.to_path_buf(), success: false, message: e.to_string() },
    }
}

/// Execute an action on multiple files. Returns (succeeded, failed) counts.
pub fn execute_bulk(paths: &[PathBuf], kind: &ActionKind) -> (usize, usize, Vec<String>) {
    let mut ok = 0;
    let mut fail = 0;
    let mut errors = Vec::new();

    for path in paths {
        let r = execute_action(path, kind);
        if r.success {
            ok += 1;
        } else {
            fail += 1;
            errors.push(format!("{}: {}", path.display(), r.message));
        }
    }
    (ok, fail, errors)
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = path.extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new("."));
    for i in 1..=9999 {
        let candidate = parent.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// Export duplicate groups to a CSV report.
pub fn export_csv(
    groups: &[crate::grouper::DuplicateGroup],
    dest: &Path,
) -> Result<()> {
    use std::io::Write;
    let mut f = fs::File::create(dest)
        .with_context(|| format!("Cannot create {}", dest.display()))?;

    writeln!(f, "Group,MatchKind,Similarity,FilePath,Size,Modified,Mark")?;
    for g in groups {
        for file in &g.files {
            writeln!(
                f,
                "{},{},{:.2},{},{},{},{:?}",
                g.id,
                g.kind.label(),
                g.similarity,
                file.path.display(),
                file.size,
                file.modified_str(),
                file.mark,
            )?;
        }
    }
    Ok(())
}
