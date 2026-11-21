#!/usr/bin/env node
// Downloads the pinned static FFmpeg build for a Rust target triple and places
// it where Tauri expects the sidecar: src-tauri/binaries/ffmpeg-<triple>[.exe].
//
// Usage:
//   node scripts/fetch-ffmpeg.mjs                  # host target (from `rustc -vV`)
//   node scripts/fetch-ffmpeg.mjs --target <triple>
//   node scripts/fetch-ffmpeg.mjs --from <path>    # use a local ffmpeg instead (dev only)
//
// Builds are listed in scripts/ffmpeg-sources.json and verified by SHA-256.
// The binaries are not committed to git.

import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const binDir = join(root, "src-tauri", "binaries");
const sources = JSON.parse(readFileSync(join(root, "scripts", "ffmpeg-sources.json"), "utf8")).targets;

function arg(name) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

function hostTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const line = out.split("\n").find((l) => l.startsWith("host:"));
  if (!line) throw new Error("cannot determine host target from `rustc -vV`");
  return line.slice("host:".length).trim();
}

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");

const target = arg("--target") ?? hostTriple();
const exe = target.includes("windows") ? ".exe" : "";
const dest = join(binDir, `ffmpeg-${target}${exe}`);
const stamp = `${dest}.source`;
mkdirSync(binDir, { recursive: true });

const from = arg("--from");
if (from) {
  copyFileSync(from, dest);
  chmodSync(dest, 0o755);
  writeFileSync(stamp, `local:${resolve(from)}\n`);
  console.log(`Copied ${from} -> ${dest} (local FFmpeg, do not ship)`);
  process.exit(0);
}

const source = sources[target];
if (!source) {
  console.error(`No pinned FFmpeg build for target '${target}'. Known: ${Object.keys(sources).join(", ")}`);
  process.exit(1);
}

if (existsSync(dest) && existsSync(stamp) && readFileSync(stamp, "utf8").trim() === source.sha256) {
  console.log(`FFmpeg ${source.version} for ${target} is up to date: ${dest}`);
  process.exit(0);
}

console.log(`Downloading FFmpeg ${source.version} for ${target} from ${source.url}`);
const res = await fetch(source.url, { redirect: "follow" });
if (!res.ok) throw new Error(`download failed: HTTP ${res.status} ${res.statusText}`);
const archive = Buffer.from(await res.arrayBuffer());
const actual = sha256(archive);
if (actual !== source.sha256) {
  throw new Error(`SHA-256 mismatch for ${source.url}\n  expected ${source.sha256}\n  actual   ${actual}`);
}

if (source.archive === "gz") {
  writeFileSync(dest, gunzipSync(archive));
} else if (source.archive === "zip") {
  const work = mkdtempSync(join(tmpdir(), "ffmpeg-"));
  try {
    const zip = join(work, "ffmpeg.zip");
    writeFileSync(zip, archive);
    // bsdtar (macOS, Windows) and GNU tar with libarchive both read zip; fall back to unzip.
    try {
      execFileSync("unzip", ["-q", "-o", zip, source.member, "-d", work]);
    } catch {
      execFileSync("tar", ["-xf", zip, "-C", work, source.member]);
    }
    copyFileSync(join(work, source.member), dest);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
} else {
  throw new Error(`unknown archive type '${source.archive}'`);
}
chmodSync(dest, 0o755);
writeFileSync(stamp, `${source.sha256}\n`);
console.log(`Saved ${dest}`);
