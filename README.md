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

## Development

Prerequisites: [Rust](https://rustup.rs) (stable), Node.js 22+, and the [Tauri system dependencies](https://tauri.app/start/prerequisites/) for your OS.

```bash
npm install
npm run tauri dev
```

Other useful commands:

```bash
cargo test --workspace                 # Rust unit tests
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
