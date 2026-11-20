# AGENTS.md

Guidance for AI coding agents working in this repository.

> This project is developed with the help of Claude Code (Anthropic).

## Project

Tauri 2 desktop app that builds timelapse videos from photos. It ports the logic of the
`create_timelapse.sh` script from [timelapse_from_photos](https://github.com/vvapnik/timelapse_from_photos)
to Rust; the bash script is never called (there is no bash on Windows).

## Architecture

- `crates/timelapse-core` — all timelapse logic. Must not depend on Tauri, so a CLI can reuse it.
  The FFmpeg path is passed in as a parameter.
- `src-tauri` — thin layer: Tauri commands, progress events, sidecar/override path resolution.
- `src` — React + TypeScript UI built with Vite.
- FFmpeg ships as a Tauri sidecar (pinned static GPL builds, downloaded by a script, never committed).
  Users may override the path in settings.

## Behaviour parity with the script

Defaults: FPS 30 (1–120), quality low/medium/high = CRF 28/23/18 with preset fast/medium/slow,
`libx264` + `yuv420p`, formats mp4/mov/avi, sequence gap 120 s, output
`{folder}_timelapse.{fmt}` or `{folder}_timelapse_part{N}.{fmt}` in the source folder.

## Checks before finishing a change

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run build
```

## Conventions

- Everything in the repository (code, comments, UI strings, docs, commit messages) is in English.
- Do not commit FFmpeg binaries or build outputs.
