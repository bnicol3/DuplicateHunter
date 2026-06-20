//! Exact-match hashing using XXH3-128 (extremely fast, collision-resistant).
//! Files are first bucketed by size — only same-size files get hashed.
//! Uses a two-pass strategy: partial hash first (first 64KB), then full hash.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::Xxh3;
use anyhow::Result;
use rayon::prelude::*;

const CHUNK_SIZE: usize = 256 * 1024;      // 256 KB read buffer
const PARTIAL_SIZE: usize = 64 * 1024;     // 64 KB for quick pre-filter

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileHash(pub u64);

fn hash_file_partial(path: &Path) -> Result<FileHash> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(PARTIAL_SIZE, file);
    let mut hasher = Xxh3::new();
    let mut buf = vec![0u8; PARTIAL_SIZE];
    let n = reader.read(&mut buf)?;
    hasher.update(&buf[..n]);
    Ok(FileHash(hasher.digest()))
}

pub fn hash_file_full(path: &Path) -> Result<FileHash> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(CHUNK_SIZE, file);
    let mut hasher = Xxh3::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(FileHash(hasher.digest()))
}

/// Group files by exact content hash.
/// Returns groups (only those with 2+ files).
/// `progress_cb` receives (hashed_count, total_to_hash).
pub fn find_exact_duplicates<F>(
    entries: &[crate::scanner::FileEntry],
    mut progress_cb: F,
) -> Result<HashMap<FileHash, Vec<PathBuf>>>
where
    F: FnMut(usize, usize),
{
    // 1. Bucket by size — eliminates most files instantly
    let mut by_size: HashMap<u64, Vec<&crate::scanner::FileEntry>> = HashMap::new();
    for e in entries {
        by_size.entry(e.size).or_default().push(e);
    }

    // Only files sharing a size need hashing
    let size_candidates: Vec<&crate::scanner::FileEntry> = by_size
        .values()
        .filter(|v| v.len() > 1)
        .flat_map(|v| v.iter().copied())
        .collect();

    if size_candidates.is_empty() {
        return Ok(HashMap::new());
    }

    // 2. Partial hash — quick second filter
    let partial_hashed: Vec<(FileHash, &crate::scanner::FileEntry)> = size_candidates
        .par_iter()
        .filter_map(|e| {
            let h = hash_file_partial(&e.path).ok()?;
            Some((h, *e))
        })
        .collect();

    let mut by_partial: HashMap<FileHash, Vec<&crate::scanner::FileEntry>> = HashMap::new();
    for (h, e) in &partial_hashed {
        by_partial.entry(h.clone()).or_default().push(e);
    }

    let full_candidates: Vec<&crate::scanner::FileEntry> = by_partial
        .values()
        .filter(|v| v.len() > 1)
        .flat_map(|v| v.iter().copied())
        .collect();

    let total = full_candidates.len();
    progress_cb(0, total);

    // 3. Full hash in parallel
    let counter = std::sync::atomic::AtomicUsize::new(0);
    let hashed: Vec<(FileHash, PathBuf)> = full_candidates
        .par_iter()
        .filter_map(|e| {
            let h = hash_file_full(&e.path).ok()?;
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some((h, e.path.clone()))
        })
        .collect();

    progress_cb(total, total);

    // 4. Group by full hash
    let mut groups: HashMap<FileHash, Vec<PathBuf>> = HashMap::new();
    for (hash, path) in hashed {
        groups.entry(hash).or_default().push(path);
    }
    groups.retain(|_, v| v.len() > 1);

    Ok(groups)
}
