//! Perceptual similarity via ffmpeg CLI.
//!
//! dHash (difference hash): scale any image/video frame to 9×8 grayscale,
//! compare each pixel to its right neighbour → 64-bit fingerprint.
//! Works across formats: JPG vs PNG vs WebP vs BMP all produce the same hash
//! if they depict the same image. Threshold ≤ 8 bits catches recompressed/
//! resized duplicates; ≤ 4 bits catches near-identical images.
//!
//! Video: sample N evenly-spaced frames across the full duration, hash each,
//! then compare videos by average frame-hash distance.

use std::path::{Path, PathBuf};
use anyhow::Result;

/// Bits that can differ and still be considered "same image". 0–64.
pub const IMAGE_THRESHOLD: u32 = 8;
/// Average per-frame bits that can differ for "same video".
pub const VIDEO_THRESHOLD: u32 = 10;
/// Frames sampled per video (evenly spaced across full duration).
pub const VIDEO_FRAME_SAMPLES: u32 = 16;

// ── Core dHash ─────────────────────────────────────────────────────────────

/// 64-bit perceptual hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DHash(pub u64);

impl DHash {
    /// Hamming distance — number of differing bits (0 = identical).
    pub fn dist(self, other: DHash) -> u32 {
        (self.0 ^ other.0).count_ones()
    }

    #[allow(dead_code)]
    pub fn similarity_pct(self, other: DHash) -> f32 {
        1.0 - self.dist(other) as f32 / 64.0
    }
}

/// Build dHash from a 72-byte (9×8) row-major grayscale buffer.
fn dhash_from_raw(raw: &[u8]) -> DHash {
    debug_assert!(raw.len() >= 72, "need 9×8 = 72 bytes");
    let mut hash: u64 = 0;
    for row in 0..8usize {
        for col in 0..8usize {
            let left  = raw[row * 9 + col] as i16;
            let right = raw[row * 9 + col + 1] as i16;
            if left > right {
                hash |= 1u64 << (row * 8 + col);
            }
        }
    }
    DHash(hash)
}

/// Use ffmpeg to decode one frame at `t` seconds, scale to 9×8 gray, return raw pixels.
/// Works for any image format or video container that ffmpeg supports.
fn extract_frame_raw(path: &Path, t: f64) -> Result<Vec<u8>> {
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-ss", &format!("{t:.3}"),
            "-i", &path.to_string_lossy(),
            "-frames:v", "1",
            "-vf", "scale=9:8:flags=lanczos,format=gray",
            "-f", "rawvideo",
            "-pix_fmt", "gray",
            "pipe:1",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()?;

    if out.stdout.len() < 72 {
        anyhow::bail!(
            "ffmpeg returned {} bytes (need 72) for {:?}",
            out.stdout.len(), path
        );
    }
    Ok(out.stdout[..72].to_vec())
}

// ── Image fingerprinting ───────────────────────────────────────────────────

pub struct ImageFingerprint {
    pub path: PathBuf,
    pub hash: DHash,
}

/// Compute perceptual hash for any image file.
/// Internally uses ffmpeg so it handles JPG, PNG, WebP, BMP, TIFF, HEIC, AVIF, etc.
pub fn fingerprint_image(path: &Path) -> Result<ImageFingerprint> {
    let raw = extract_frame_raw(path, 0.0)?;
    Ok(ImageFingerprint {
        path: path.to_path_buf(),
        hash: dhash_from_raw(&raw),
    })
}

/// Cluster images into groups where every pair is within `threshold` bits.
pub fn group_similar_images(
    prints: &[ImageFingerprint],
    threshold: u32,
) -> Vec<(Vec<PathBuf>, f32)>   // (paths, avg_similarity)
{
    cluster_dhash(
        prints.iter().map(|p| (p.path.clone(), p.hash)),
        threshold,
    )
}

// ── Video fingerprinting ───────────────────────────────────────────────────

pub struct VideoFingerprint {
    pub path: PathBuf,
    pub frame_hashes: Vec<DHash>,
}

/// Get video duration in seconds via ffprobe.
fn video_duration(path: &Path) -> f64 {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            &path.to_string_lossy(),
        ])
        .output()
        .ok();
    out.and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<f64>()
            .ok()
    })
    .unwrap_or(60.0)
}

/// Sample `num_frames` evenly-spaced frames from a video and compute dHash for each.
pub fn fingerprint_video(path: &Path, num_frames: u32) -> Result<VideoFingerprint> {
    let duration = video_duration(path);
    let mut frame_hashes = Vec::with_capacity(num_frames as usize);

    for i in 0..num_frames {
        // Offset by 0.5 so we don't sample exactly at start/end
        let t = duration * (i as f64 + 0.5) / num_frames as f64;
        if let Ok(raw) = extract_frame_raw(path, t) {
            frame_hashes.push(dhash_from_raw(&raw));
        }
    }

    if frame_hashes.is_empty() {
        anyhow::bail!("No frames could be extracted from {:?}", path);
    }

    Ok(VideoFingerprint {
        path: path.to_path_buf(),
        frame_hashes,
    })
}

/// Average Hamming distance between two video fingerprints.
/// Aligns by relative frame position (handles different frame counts).
pub fn video_distance(a: &VideoFingerprint, b: &VideoFingerprint) -> u32 {
    let n = a.frame_hashes.len().min(b.frame_hashes.len());
    if n == 0 {
        return u32::MAX;
    }
    let sum: u32 = (0..n)
        .map(|i| a.frame_hashes[i].dist(b.frame_hashes[i]))
        .sum();
    sum / n as u32
}

/// Cluster videos into groups where average frame distance ≤ threshold.
pub fn group_similar_videos(
    prints: &[VideoFingerprint],
    threshold: u32,
) -> Vec<(Vec<PathBuf>, f32)>   // (paths, avg_similarity)
{
    let n = prints.len();
    let mut visited = vec![false; n];
    let mut groups = Vec::new();

    for i in 0..n {
        if visited[i] {
            continue;
        }
        let mut group = vec![prints[i].path.clone()];
        let mut total_sim = 0.0f32;
        let mut pairs = 0;

        for j in (i + 1)..n {
            if visited[j] {
                continue;
            }
            let dist = video_distance(&prints[i], &prints[j]);
            if dist <= threshold {
                let sim = 1.0 - dist as f32 / 64.0;
                total_sim += sim;
                pairs += 1;
                group.push(prints[j].path.clone());
                visited[j] = true;
            }
        }
        visited[i] = true;

        if group.len() > 1 {
            let avg_sim = if pairs > 0 { total_sim / pairs as f32 } else { 1.0 };
            groups.push((group, avg_sim));
        }
    }
    groups
}

// ── Generic clustering ─────────────────────────────────────────────────────

fn cluster_dhash(
    items: impl Iterator<Item = (PathBuf, DHash)>,
    threshold: u32,
) -> Vec<(Vec<PathBuf>, f32)> {
    let v: Vec<(PathBuf, DHash)> = items.collect();
    let n = v.len();
    let mut visited = vec![false; n];
    let mut groups = Vec::new();

    for i in 0..n {
        if visited[i] {
            continue;
        }
        let mut group = vec![v[i].0.clone()];
        let mut total_sim = 0.0f32;
        let mut pairs = 0;

        for j in (i + 1)..n {
            if visited[j] {
                continue;
            }
            let dist = v[i].1.dist(v[j].1);
            if dist <= threshold {
                let sim = 1.0 - dist as f32 / 64.0;
                total_sim += sim;
                pairs += 1;
                group.push(v[j].0.clone());
                visited[j] = true;
            }
        }
        visited[i] = true;

        if group.len() > 1 {
            let avg_sim = if pairs > 0 { total_sim / pairs as f32 } else { 1.0 };
            groups.push((group, avg_sim));
        }
    }
    groups
}
