// Temporary developer screen that exercises scan / render / cancel.
// The real main screen comes in TASK-113.6.
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

type Quality = "low" | "medium" | "high";
type Format = "mp4" | "mov" | "avi";

type ScanSummary = {
  folder: string;
  totalFrames: number;
  exifFrames: number;
  skipped: string[];
  sequences: { frames: number; startMs: number | null; endMs: number | null; output: string }[];
};

type RenderEvent =
  | { type: "sequenceStarted"; index: number; count: number; frames: number; output: string }
  | { type: "progress"; index: number; sequencePercent: number; overallPercent: number }
  | { type: "sequenceFinished"; index: number; output: string };

type RenderFinished = { outputs: string[]; cancelled: boolean; error: string | null };

type Progress = { index: number; count: number; sequence: number; overall: number };

function App() {
  const [folder, setFolder] = useState("");
  const [gap, setGap] = useState(120);
  const [fps, setFps] = useState(30);
  const [quality, setQuality] = useState<Quality>("medium");
  const [format, setFormat] = useState<Format>("mp4");
  const [customFfmpeg, setCustomFfmpeg] = useState("");

  const [ffmpeg, setFfmpeg] = useState<string | null>(null);
  const [scan, setScan] = useState<ScanSummary | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [running, setRunning] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const unlisten = [
      listen<RenderEvent>("render-progress", ({ payload }) => {
        setProgress((p) => {
          const base = p ?? { index: 0, count: 1, sequence: 0, overall: 0 };
          switch (payload.type) {
            case "sequenceStarted":
              return { ...base, index: payload.index, count: payload.count, sequence: 0 };
            case "progress":
              return {
                ...base,
                index: payload.index,
                sequence: payload.sequencePercent,
                overall: payload.overallPercent,
              };
            default:
              return base;
          }
        });
      }),
      listen<RenderFinished>("render-finished", ({ payload }) => {
        setRunning(false);
        if (payload.error) setError(payload.error);
        else if (payload.cancelled) setMessage("Render cancelled; unfinished file removed.");
        else setMessage(`Done: ${payload.outputs.join(", ")}`);
      }),
    ];
    return () => unlisten.forEach((p) => p.then((f) => f()));
  }, []);

  const ffmpegArg = customFfmpeg.trim() || null;

  async function run<T>(action: () => Promise<T>): Promise<T | undefined> {
    setError(null);
    setMessage(null);
    try {
      return await action();
    } catch (e) {
      setError(String(e));
      return undefined;
    }
  }

  const checkFfmpeg = () =>
    run(async () => {
      const status = await invoke<{ path: string; version: string }>("ffmpeg_status", {
        customPath: ffmpegArg,
      });
      setFfmpeg(`${status.version} (${status.path})`);
    });

  const doScan = () =>
    run(async () => {
      setScan(await invoke<ScanSummary>("scan", { folder, gapSeconds: gap, format }));
    });

  const startRender = () =>
    run(async () => {
      setProgress(null);
      await invoke("start_render", {
        folder,
        gapSeconds: gap,
        settings: { fps, quality, format },
        customFfmpeg: ffmpegArg,
      });
      setRunning(true);
    });

  const cancel = () => run(() => invoke("cancel_render"));

  return (
    <main className="container">
      <h1>Timelapse Desktop</h1>
      <p className="muted">Developer screen — the real main screen is coming next.</p>

      <form className="grid" onSubmit={(e) => e.preventDefault()}>
        <label>
          Photo folder
          <input value={folder} onChange={(e) => setFolder(e.currentTarget.value)} placeholder="/Volumes/SD/DCIM" />
        </label>
        <label>
          Gap, seconds
          <input type="number" min={1} value={gap} onChange={(e) => setGap(Number(e.currentTarget.value))} />
        </label>
        <label>
          FPS
          <input type="number" min={1} max={120} value={fps} onChange={(e) => setFps(Number(e.currentTarget.value))} />
        </label>
        <label>
          Quality
          <select value={quality} onChange={(e) => setQuality(e.currentTarget.value as Quality)}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
          </select>
        </label>
        <label>
          Format
          <select value={format} onChange={(e) => setFormat(e.currentTarget.value as Format)}>
            <option value="mp4">MP4</option>
            <option value="mov">MOV</option>
            <option value="avi">AVI</option>
          </select>
        </label>
        <label>
          Custom FFmpeg (optional)
          <input value={customFfmpeg} onChange={(e) => setCustomFfmpeg(e.currentTarget.value)} placeholder="bundled" />
        </label>
      </form>

      <div className="row">
        <button onClick={checkFfmpeg}>Check FFmpeg</button>
        <button onClick={doScan} disabled={!folder || running}>Scan</button>
        <button onClick={startRender} disabled={!folder || running}>Render</button>
        <button onClick={cancel} disabled={!running}>Cancel</button>
      </div>

      {ffmpeg && <p className="muted">FFmpeg: {ffmpeg}</p>}

      {scan && (
        <section className="panel">
          <p>
            {scan.totalFrames} photos ({scan.exifFrames} with EXIF time) in {scan.sequences.length} sequence(s)
            {scan.skipped.length > 0 && `, ${scan.skipped.length} skipped`}
          </p>
          <ol>
            {scan.sequences.map((s) => (
              <li key={s.output}>
                {s.frames} photos → {s.output}
              </li>
            ))}
          </ol>
        </section>
      )}

      {progress && (
        <section className="panel">
          <label>
            Sequence {progress.index + 1} of {progress.count}: {progress.sequence.toFixed(0)}%
            <progress max={100} value={progress.sequence} />
          </label>
          <label>
            Overall: {progress.overall.toFixed(0)}%
            <progress max={100} value={progress.overall} />
          </label>
        </section>
      )}

      {message && <p>{message}</p>}
      {error && <p className="error">{error}</p>}
    </main>
  );
}

export default App;
