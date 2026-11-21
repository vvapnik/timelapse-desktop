# Timelapse Desktop

A desktop app (macOS, Windows, Linux) that turns a folder of photos — or a camera SD card — into timelapse videos, no terminal required.

> **Status:** early development (v0.1 in progress). The app currently opens a placeholder window.

It is the GUI counterpart of [timelapse_from_photos](https://github.com/vvapnik/timelapse_from_photos), a bash script that does the same from the command line. Use the script if you prefer the terminal; this app ports its logic to Rust and runs on Windows too.

## Planned for v0.1

- Pick a folder or auto-detect a `DCIM` folder on a removable drive
- Split photos into separate sequences when there is a long pause between shots
- Render each sequence with FFmpeg (FPS 1–120, quality low/medium/high, MP4/MOV/AVI) with progress and cancel
- Quick preview of the resulting video
- FFmpeg bundled with the app; a custom FFmpeg path can be set in settings

## How photos are ordered and split

- **Photos:** JPG, JPEG, PNG, BMP and TIFF files anywhere inside the chosen folder (extension case does not matter). Hidden files such as macOS `._IMG_0001.JPG` metadata files on SD cards are ignored.
- **Capture time:** taken from EXIF `DateTimeOriginal` (with sub-seconds and UTC offset when the camera records them). Without an offset, the EXIF time is read as local time of this computer. If a photo has no usable EXIF date — or it is a BMP, which has no EXIF — the file's modification time is used instead.
- **Order:** by capture time; photos taken at the same moment are ordered by file path.
- **Sequences:** a new video starts when the pause between two consecutive photos is longer than the gap threshold (default 120 seconds, measured in whole seconds). A pause exactly equal to the threshold does not split.

## Rendering

Each sequence becomes its own file in the photo folder: `{folder}_timelapse.{mp4|mov|avi}`, or `{folder}_timelapse_part1…` when there are several. Settings match the script: FPS 1–120 (default 30), quality low/medium/high = x264 CRF 28/23/18 with preset fast/medium/slow, H.264 `yuv420p`. Progress is reported per sequence and overall; a render can be cancelled. The video is written to a hidden `.…partial` file first and renamed when done, so a cancelled or failed render leaves no broken file and does not replace an existing one.

FFmpeg is bundled with the app (pinned static GPL builds, see `scripts/ffmpeg-sources.json`); a different FFmpeg can be chosen in settings.

### Differences from `create_timelapse.sh`

- The script sorts photos by file name and only uses the file modification time; for camera folders, where names follow shooting order, both give the same sequences.
- Every photo becomes exactly one video frame. The script lets FFmpeg's concat demuxer show each photo for 1/25 s and then resamples to the target FPS, which duplicates frames above 25 FPS and drops photos below it.
- Odd image dimensions are rounded down to even ones (required by `yuv420p`) instead of failing.

## Development

Prerequisites: [Rust](https://rustup.rs) (stable), Node.js 22+, and the [Tauri system dependencies](https://tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run fetch-ffmpeg   # downloads the pinned FFmpeg sidecar into src-tauri/binaries/
npm run tauri dev
```

`fetch-ffmpeg` picks the build for your Rust host target from `scripts/ffmpeg-sources.json` and checks its SHA-256. To develop with an FFmpeg you already have instead, run `node scripts/fetch-ffmpeg.mjs --from "$(which ffmpeg)"` (never ship that one).

Other useful commands:

```bash
cargo test --workspace                 # unit tests + render tests (FFmpeg from $FFMPEG or PATH)
cargo clippy --workspace --all-targets # lints
npm run tauri build -- --no-bundle     # release binary without installers
```

### Layout

| Path | What |
|---|---|
| `crates/timelapse-core` | Timelapse logic (scanning, sequence splitting, FFmpeg arguments). No Tauri dependencies. |
| `src-tauri` | Tauri 2 shell: commands and events that expose the core to the UI |
| `src` | React + TypeScript UI (Vite) |

## AI-assisted development

This project is developed with the help of [Claude Code](https://claude.com/claude-code) (Anthropic).

## License

[MIT](LICENSE). Builds will bundle FFmpeg, which is licensed separately under the GPL; third-party notices will ship with the installers.
