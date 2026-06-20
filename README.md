# DupeHunter

A cross-platform duplicate file finder with perceptual media comparison.

## Features

- **Exact duplicate detection** — XXH3 hashing (byte-for-byte identical files)
- **Perceptual image matching** — finds visually identical images across different
  formats (JPG, PNG, WebP, BMP, TIFF, HEIC, AVIF, etc.) and different compression levels
- **Perceptual video matching** — samples frames across the full video duration to
  find re-encoded, re-compressed, or re-formatted duplicate videos
- **Bulk actions** — delete, quarantine (move to folder), or export a CSV report
- **Auto-mark** — automatically keep newest or oldest file in each group
- **Adjustable sensitivity** — tune the perceptual threshold per use case
- **Fully native** — single self-contained binary, no Electron, no JVM, no Python

## Requirements

### Build
- **Rust 1.80+** — install from https://rustup.rs
- **ffmpeg** (CLI) — must be on your PATH for perceptual matching
  - Windows: https://ffmpeg.org/download.html (add to PATH)
  - macOS: `brew install ffmpeg`
  - Linux: `sudo apt install ffmpeg` or `sudo dnf install ffmpeg`

### Runtime
- ffmpeg must remain on PATH for perceptual features
- Exact-hash matching works without ffmpeg

## Building

```bash
# Clone / extract the source, then:
cd dupehunter
cargo build --release

# The binary will be at:
#   Windows:  target\release\dupehunter.exe
#   macOS:    target/release/dupehunter
#   Linux:    target/release/dupehunter
```

## Running

```bash
# Run the release build:
./target/release/dupehunter          # Linux/macOS
target\release\dupehunter.exe        # Windows
```

Or just `cargo run --release` from the project directory.

## Usage

1. **Add Folders** — click "Add Folder…" to add one or more directories to scan
2. **Configure Options** — toggle recursive scan, hidden file skipping, min file size
3. **Set Sensitivity** — adjust the perceptual threshold sliders:
   - `0–4 bits`: very strict (byte-near-identical, just re-saved)
   - `5–8 bits`: normal (same image, different format or compression)
   - `9–15 bits`: loose (similar images, minor crops or edits)
4. **Scan** — click "Scan for Duplicates"
5. **Review results** — duplicate groups appear sorted by wasted space
6. **Mark files** — click **✓ Keep** or **🗑 Delete** per file, or use
   "Keep newest" / "Keep oldest" per group, or "Keep newest" globally
7. **Act** — delete marked files permanently, or move them to a quarantine folder

## How it works

| Stage | Method | Speed |
|---|---|---|
| File collection | walkdir traversal | Very fast |
| Exact dedup | Size bucket → XXH3 partial → XXH3 full | Fast |
| Image perceptual | ffmpeg → 9×8 grayscale → dHash (64-bit) | Moderate |
| Video perceptual | ffmpeg → 16 evenly-spaced frames → avg dHash | Slow (per video) |

The perceptual hash (dHash) converts any image or video frame to a 64-bit
fingerprint by scaling to 9×8 grayscale and comparing adjacent pixels.
Two files are considered duplicates if their Hamming distance is ≤ threshold.

## Platform notes

- **Windows**: the native folder picker (rfd) uses the Windows Shell dialog
- **macOS**: requires Gatekeeper approval on first run (`xattr -d com.apple.quarantine ./dupehunter`)
- **Linux**: requires a display server (X11 or Wayland); works in all major DEs
- ffmpeg CLI is used for perceptual hashing — no C library bindings needed

## License

MIT
