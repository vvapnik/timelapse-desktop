import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type CoreInfo = {
  version: string;
  supported_extensions: string[];
};

function App() {
  const [info, setInfo] = useState<CoreInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<CoreInfo>("core_info")
      .then(setInfo)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <main className="container">
      <h1>Timelapse Desktop</h1>
      <p>Turn a folder of photos into timelapse videos. The main screen is coming next.</p>
      {info && (
        <p className="muted" data-testid="core-info">
          Core v{info.version} · formats: {info.supported_extensions.join(", ")}
        </p>
      )}
      {error && <p className="error">Core is unavailable: {error}</p>}
    </main>
  );
}

export default App;
