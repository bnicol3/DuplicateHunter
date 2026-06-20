# DupeHunter

A cross-platform duplicate file finder with perceptual media comparison, built in Rust with a native GUI.

Unlike simple duplicate finders that only compare filenames or byte-for-byte content, DupeHunter can identify visually identical images and videos even when they have **different filenames, different file formats, or different compression levels**. A JPEG and a PNG of the same photo will be flagged as duplicates. A video re-encoded at a different bitrate will be matched against the original.

---

## Features

### Detection
- **Exact duplicate detection** — identifies byte-for-byte identical files of any type using XXH3 hashing, one of the fastest hash algorithms available
- **Perceptual image matching** — finds visually identical images across different formats (JPG, PNG, WebP, BMP, TIFF, GIF, ICO, HEIC, HEIF, AVIF, JXL) and different compression levels using dHash fingerprinting
- **Perceptual video matching** — samples 16 evenly-spaced frames across the full duration of each video to identify duplicate content regardless of encoding, bitrate, or container format

### Scanning
- Scan multiple folders simultaneously
- Recursive subdirectory scanning (toggleable)
- Skip hidden files and dotfiles (toggleable)
- Minimum file size filter
- Adjustable perceptual sensitivity thresholds for both images and video
- Configurable number of video frame samples (4–32)

### Results
- Filter results by match type: All, Exact, Image, or Video
- Sort by wasted space, copy count, or filename
- Text search to filter by filename or path
- Show only marked files
- Expand or collapse all groups at once
- Similarity percentage shown for perceptual matches

### Actions
- Mark individual files as **Keep** or **Delete**
- Per-group quick actions: **Keep newest** or **Keep oldest**
- Global bulk actions: **Keep newest across all groups**, **Keep oldest across all groups**, **Clear all marks**
- **Permanently delete** marked files (with confirmation dialog)
- **Quarantine** marked files by moving them to a folder of your choice
- **Export CSV report** of all duplicate groups with their marks

---

## Supported Formats

| Type   | Formats |
|--------|---------|
| Images | JPG, JPEG, PNG, GIF, WebP, BMP, TIFF, TIF, ICO, HEIC, HEIF, AVIF, JXL |
| Video  | MP4, MKV, AVI, MOV, WMV, FLV, WebM, M4V, MPG, MPEG, 3GP, TS, MTS, M2TS |
| Other  | Any file type (exact hash matching only) |

---

## Requirements

### Build-time
- **Rust 1.80 or newer** — https://rustup.rs
- **build-essential** and **pkg-config** (Linux)
- **libgtk-3-dev** (Linux) — required by the native file picker

### Runtime
- **ffmpeg** — required for perceptual image and video matching. Must be available on your system PATH. Exact-hash duplicate detection works without ffmpeg.

---

## Installation

### Step 1 — Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.bashrc
```

Verify with:
```bash
cargo --version
```

### Step 2 — Install system dependencies

**Ubuntu / Debian:**
```bash
sudo apt update
sudo apt install ffmpeg build-essential pkg-config libgtk-3-dev
```

**Fedora:**
```bash
sudo dnf install ffmpeg gcc pkg-config gtk3-devel
```

**Arch Linux:**
```bash
sudo pacman -S ffmpeg base-devel pkg-config gtk3
```

**macOS:**
```bash
brew install ffmpeg
# Rust and build tools come from Xcode Command Line Tools
```

**Windows:**
1. Install Rust from https://rustup.rs
2. Download ffmpeg from https://www.gyan.dev/ffmpeg/builds/ (the `release-essentials` build)
3. Extract it and add the `bin` folder to your system PATH:
   - Search for **"Edit the system environment variables"**
   - Click **Environment Variables → System variables → Path → Edit → New**
   - Add the path to the ffmpeg `bin` folder (e.g. `C:\ffmpeg\bin`)
   - Click OK on all dialogs, then open a new terminal

### Step 3 — Build

```bash
unzip dupehunter-source.zip
cd dupehunter-v2
cargo build --release
```

The first build downloads and compiles all dependencies, which takes a few minutes. Subsequent builds are much faster.

### Step 4 — Run

```bash
./target/release/dupehunter
```

On Windows: `target\release\dupehunter.exe`

You can copy the binary anywhere on your system — it has no external runtime dependencies beyond ffmpeg.

---

## Usage

### 1. Add folders to scan

Click **Add Folder…** to open a folder picker. You can add multiple folders — DupeHunter will scan all of them together and find duplicates both within and across folders.

To remove a folder, click the **✕** next to it.

### 2. Configure options

| Option | Default | Description |
|--------|---------|-------------|
| Scan subdirectories | On | Recursively scan all nested folders |
| Skip hidden files | On | Ignore files and folders starting with `.` |
| Minimum file size | 1 byte | Skip files smaller than this |
| Perceptual image matching | On | Find visually identical images across formats |
| Perceptual video matching | On | Find duplicate video content |

### 3. Adjust sensitivity

The **perceptual threshold** controls how similar two files must be to be considered duplicates. The value is the maximum allowed Hamming distance between two 64-bit fingerprints — lower is stricter.

**Image threshold (default: 8 bits out of 64):**

| Range | Meaning |
|-------|---------|
| 0–2   | Strict — byte-near-identical, only catches trivial re-saves |
| 3–8   | Normal — same image in different format or compression |
| 9–15  | Loose — similar images, catches minor crops or colour adjustments |
| 16+   | Very loose — may produce false positives |

**Video threshold (default: 10):** Applied to the average distance across all sampled frames.

**Video frame samples (default: 16):** How many frames are extracted and compared per video. More samples improves accuracy for longer or more varied content but slows down the scan.

### 4. Scan

Click **Scan for Duplicates**. A progress bar shows the current stage:
- Collecting files
- Hashing files (exact matching)
- Perceptual image fingerprinting
- Perceptual video frame sampling
- Building duplicate groups

### 5. Review results

Each duplicate group shows:
- Match type badge (Exact / Visual / Video) with colour coding
- Similarity percentage for perceptual matches
- Representative filename
- Number of copies and total reclaimable space
- Per-file: filename, full path, file size, last modified date

Use the toolbar to filter and sort:
- **Filter by type:** All / Exact / Image / Video
- **Sort:** Largest waste, Smallest waste, Most copies, Name A→Z
- **Search box:** filter groups by filename or path
- **Marked only:** show only groups that have at least one marked file

### 6. Mark files

For each file in a group you can click:
- **✓ Keep** — this file will be preserved (green highlight)
- **🗑 Delete** — this file will be acted on (red highlight)

Clicking the same button again clears the mark.

**Per-group quick actions** (buttons in the group header):
- **Keep newest** — marks the most recently modified file as Keep, all others as Delete
- **Keep oldest** — marks the earliest file as Keep, all others as Delete
- **Clear** — removes all marks for that group

**Global bulk actions** (toolbar):
- **Keep newest** — applies "keep newest" logic to every group at once
- **Keep oldest** — applies "keep oldest" logic to every group at once
- **Clear marks** — removes all marks from all groups

### 7. Act on marked files

The right-hand **Actions panel** shows a summary of how many files are marked and how much space will be reclaimed.

**Delete marked files** permanently removes them. A confirmation dialog shows the count and total size before proceeding.

**Move to quarantine** moves marked files to a folder you specify. Click **📂** to browse for the destination, or type a path directly. Files are renamed with a counter if there are naming conflicts. This is a safe alternative to deletion — you can review and delete the quarantine folder manually later.

**Export CSV report** saves a spreadsheet of all duplicate groups, their match type, similarity, file paths, sizes, modification dates, and current marks. Useful for reviewing before taking action.

---

## How it works

### Exact matching

1. All files are bucketed by size — files of different sizes cannot be byte-identical
2. Within each size bucket, a 64KB partial XXH3 hash is computed — eliminates most non-duplicates cheaply
3. Remaining candidates get a full XXH3 hash across the entire file
4. Steps 2 and 3 run in parallel across all CPU cores via Rayon

### Perceptual image matching

1. Each image is passed to ffmpeg, which decodes it regardless of format
2. ffmpeg scales the decoded frame to 9×8 pixels in grayscale
3. A 64-bit dHash fingerprint is computed by comparing each pixel to its right neighbour across all 8 rows
4. All image fingerprints are compared pairwise; those within the Hamming distance threshold are grouped

Because ffmpeg handles the decoding, the hash is format-agnostic — a JPEG and a PNG of the same photograph will produce near-identical fingerprints.

### Perceptual video matching

1. ffprobe determines the duration of each video
2. ffmpeg extracts 16 frames at evenly-spaced timestamps across the full duration
3. Each frame is fingerprinted using the same dHash algorithm as images
4. Two videos are compared by computing the average Hamming distance across all frame pairs
5. Videos within the average distance threshold are grouped as duplicates

---

## Performance notes

- Exact hashing is very fast — a folder of 10,000 files typically completes in seconds
- Perceptual image hashing requires a ffmpeg call per image — expect roughly 5–20 images per second depending on image size and disk speed
- Perceptual video hashing requires 16 ffmpeg calls per video (one per frame sample) — this is the slowest stage, but runs one video at a time to avoid overwhelming disk I/O
- The binary is compiled with LTO and full optimisation (`opt-level = 3`) for maximum runtime performance

---

## Building from source

```bash
# Debug build (faster to compile, slower to run)
cargo build

# Release build (slower to compile, fast to run — use this normally)
cargo build --release

# Run directly without a separate build step
cargo run --release
```

---

## Project structure

```
src/
├── main.rs        — Application entry point, egui UI, scan orchestration
├── scanner.rs     — Filesystem traversal and file classification
├── hasher.rs      — XXH3 exact hashing with two-pass strategy
├── perceptual.rs  — dHash perceptual fingerprinting via ffmpeg
├── grouper.rs     — Duplicate group data structures and auto-mark logic
└── actions.rs     — Delete, quarantine, hardlink, and CSV export
assets/
└── icon.png       — Application icon
```

---

## Troubleshooting

**"ffmpeg: command not found" or perceptual matching produces no results**
Install ffmpeg and ensure it is on your PATH. Run `ffmpeg -version` in a terminal to verify.

**Build fails with missing library errors on Linux**
Install the GTK development headers: `sudo apt install libgtk-3-dev pkg-config`

If errors mention `libxcb`, run:
```bash
sudo apt install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

**The app opens but shows a blank window or crashes on Linux**
Ensure you have a display server running (X11 or Wayland). DupeHunter is a graphical application and requires a desktop environment.

**Large video collections are slow to scan**
Reduce the **video frame samples** slider (4–8 is often sufficient for identifying re-encoded duplicates). You can also disable perceptual video matching entirely if you only need to find exact video duplicates.

**Too many false positives in image matching**
Lower the **image threshold** slider. Values of 4 or below will only match images that are extremely similar — essentially the same image re-saved with no edits.

**A file failed to delete**
This usually means a permissions issue. Check that you own the file with `ls -la` and that it is not currently open in another application.

---

## License

MIT
MIT
