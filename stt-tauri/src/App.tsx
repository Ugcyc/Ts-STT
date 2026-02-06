import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  isRegistered,
  register,
  unregisterAll,
} from "@tauri-apps/plugin-global-shortcut";
import "./App.css";

type AppConfig = {
  hotkey: string;
  theme: string;
  output_mode: string;
  mic_device: string | null;
  model_size: string;
  language: string;
  spoken_punctuation: boolean;
  silence_timeout_secs: number;
  model_idle_unload_secs: number;
  log_transcripts: boolean;
  model_path: string | null;
};

type InitialData = {
  config: AppConfig;
  input_devices: string[];
  model_path: string;
  model_exists: boolean;
  status: string;
};

type TranscriptionResponse = {
  raw_text: string;
  final_text: string;
  seconds: number;
};

type ToggleResponse = {
  is_recording: boolean;
  status: string;
  transcript: TranscriptionResponse | null;
  note: string | null;
};

const defaultConfig: AppConfig = {
  hotkey: "CommandOrControl+Shift+Space",
  theme: "dark",
  output_mode: "clipboard",
  mic_device: null,
  model_size: "base",
  language: "en",
  spoken_punctuation: true,
  silence_timeout_secs: 60,
  model_idle_unload_secs: 300,
  log_transcripts: false,
  model_path: null,
};

function normalizeHotkey(input: string): string {
  const parts = input
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter(Boolean);

  return parts
    .map((part) => {
      if (
        part === "ctrl" ||
        part === "control" ||
        part === "cmdorcontrol" ||
        part === "commandorcontrol"
      ) {
        return "CommandOrControl";
      }
      if (part === "shift") return "Shift";
      if (part === "alt" || part === "option") return "Alt";
      if (part === "meta" || part === "cmd" || part === "command" || part === "super") {
        return "Super";
      }
      if (part === "space" || part === "spacebar") return "Space";
      if (part === "esc") return "Escape";
      if (part.length === 1) return part.toUpperCase();
      return part.charAt(0).toUpperCase() + part.slice(1);
    })
    .join("+");
}

function hotkeyCandidates(input: string): string[] {
  const normalized = normalizeHotkey(input);
  const lower = normalized.toLowerCase();
  const ctrlVariant = normalized.replace("CommandOrControl", "Ctrl");
  const raw = input.trim();
  return [...new Set([normalized, lower, ctrlVariant, raw].filter(Boolean))];
}

function App() {
  const MINI_SIZE = new LogicalSize(210, 58);
  const EXPANDED_SIZE = new LogicalSize(820, 700);

  const [config, setConfig] = useState<AppConfig>(defaultConfig);
  const [devices, setDevices] = useState<string[]>(["Default"]);
  const [status, setStatus] = useState("Idle");
  const [isRecording, setIsRecording] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [modelExists, setModelExists] = useState(false);
  const [finalText, setFinalText] = useState("");
  const [rawText, setRawText] = useState("");
  const [busy, setBusy] = useState(false);
  const [hotkeyReady, setHotkeyReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);

  const hotkeyInFlight = useRef(false);
  const isRecordingRef = useRef(false);

  useEffect(() => {
    isRecordingRef.current = isRecording;
  }, [isRecording]);

  const loadInitial = async () => {
    setBusy(true);
    setError(null);
    try {
      const data = await invoke<InitialData>("get_initial_data");
      setConfig(data.config);
      setDevices(data.input_devices.length ? data.input_devices : ["Default"]);
      setStatus(data.status);
      setModelExists(data.model_exists);
      await registerHotkey(data.config.hotkey);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void loadInitial();
    return () => {
      void unregisterAll();
    };
  }, []);

  useEffect(() => {
    const win = getCurrentWindow();
    const applyWindowMode = async () => {
      try {
        if (settingsOpen) {
          await win.setDecorations(true);
          await win.setResizable(true);
          await win.setAlwaysOnTop(false);
          await win.setSize(EXPANDED_SIZE);
          await win.center();
        } else {
          await win.setAlwaysOnTop(true);
          await win.setResizable(false);
          await win.setDecorations(false);
          await win.setSize(MINI_SIZE);
        }
      } catch {
        // no-op: capability may be missing in early startup
      }
    };
    void applyWindowMode();
  }, [settingsOpen]);

  const registerHotkey = async (hotkey: string) => {
    setError(null);
    setInfo(null);
    try {
      await unregisterAll();
      let lastError = "Unknown registration failure.";

      for (const candidate of hotkeyCandidates(hotkey)) {
        try {
          await register(candidate, (event) => {
            if (event.state === "Pressed") {
              void toggleFromHotkey();
            }
          });

          const ok = await isRegistered(candidate);
          if (ok) {
            setHotkeyReady(true);
            if (candidate !== hotkey) {
              setInfo(`Hotkey registered as: ${candidate}`);
            }
            return;
          }
        } catch (err) {
          lastError = String(err);
        }
      }

      setHotkeyReady(false);
      setError(`Hotkey registration failed: ${lastError}`);
    } catch (outerErr) {
      setHotkeyReady(false);
      setError(`Hotkey setup failed: ${String(outerErr)}`);
    }
  };

  const toggleFromHotkey = async () => {
    if (hotkeyInFlight.current) return;
    hotkeyInFlight.current = true;
    setError(null);
    setInfo(null);
    try {
      const wasRecording = isRecordingRef.current;
      if (wasRecording) {
        setStatus("Transcribing");
      }
      const result = await invoke<ToggleResponse>("toggle_recording_and_process");
      applyToggleResult(result);
    } catch (err) {
      setError(String(err));
      setStatus("Idle");
      setIsRecording(false);
    } finally {
      hotkeyInFlight.current = false;
    }
  };

  const applyToggleResult = (result: ToggleResponse) => {
    setStatus(result.status);
    setIsRecording(result.is_recording);
    if (result.transcript) {
      setFinalText(result.transcript.final_text);
      setRawText(result.transcript.raw_text);
    }
    if (result.note) {
      setInfo(result.note);
    }
  };

  const handleStart = async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      await invoke<string>("start_recording", {
        deviceName: config.mic_device,
        device_name: config.mic_device,
      });
      setStatus("Listening");
      setIsRecording(true);
    } catch (err) {
      setError(String(err));
      setStatus("Idle");
      setIsRecording(false);
    } finally {
      setBusy(false);
    }
  };

  const handleStop = async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      setStatus("Transcribing");
      const result = await invoke<TranscriptionResponse>("stop_recording_and_process");
      setStatus("Idle");
      setIsRecording(false);
      setFinalText(result.final_text);
      setRawText(result.raw_text);
      if (!result.final_text) {
        setInfo("No speech detected.");
      }
    } catch (err) {
      setError(String(err));
      setStatus("Idle");
      setIsRecording(false);
    } finally {
      setBusy(false);
    }
  };

  const handleSaveConfig = async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const saved = await invoke<AppConfig>("save_config", { config });
      setConfig(saved);
      await registerHotkey(saved.hotkey);
      setInfo("Settings saved.");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleRefreshDevices = async () => {
    setBusy(true);
    setError(null);
    try {
      const list = await invoke<string[]>("list_input_devices");
      setDevices(list.length ? list : ["Default"]);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleDownloadModel = async () => {
    setBusy(true);
    setError(null);
    setInfo("Downloading model...");
    try {
      const path = await invoke<string>("download_model");
      setModelExists(true);
      setConfig((prev) => ({ ...prev, model_path: path }));
      setInfo("Model downloaded.");
    } catch (err) {
      setError(String(err));
      setInfo(null);
    } finally {
      setBusy(false);
    }
  };

  const handleCopyPerfLogs = async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const note = await invoke<string>("copy_performance_report");
      setInfo(note);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const updateConfig = <K extends keyof AppConfig>(key: K, value: AppConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const lower = status.toLowerCase();
  const capsuleTone = lower.includes("listen")
    ? "listening"
    : lower.includes("transcrib")
      ? "transcribing"
      : "idle";
  const compactText = lower.includes("listen")
    ? "Listening"
    : lower.includes("transcrib")
      ? "Transcribing"
      : "Ready";
  const theme = config.theme === "light" ? "light" : "dark";

  return (
    <main className={`app-shell ${settingsOpen ? "expanded" : "compact"} theme-${theme}`}>
      {!settingsOpen ? (
        <section
          className={`capsule ${capsuleTone}`}
          data-tauri-drag-region
        >
          <button className="mic-button" onClick={isRecording ? handleStop : handleStart} disabled={busy}>
            <svg
              className="mic-icon"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3z"></path>
              <path d="M19 10v2a7 7 0 0 1-14 0v-2"></path>
              <line x1="12" y1="19" x2="12" y2="22"></line>
            </svg>
          </button>
          <span className="status-text" data-tauri-drag-region>{compactText}</span>
          <button
            className="capsule-settings"
            onClick={() => setSettingsOpen(true)}
            disabled={busy}
            title="Open settings"
            aria-label="Open settings"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true" fill="none">
              <line x1="4" y1="6" x2="20" y2="6" />
              <circle cx="9" cy="6" r="1.8" />
              <line x1="4" y1="12" x2="20" y2="12" />
              <circle cx="15" cy="12" r="1.8" />
              <line x1="4" y1="18" x2="20" y2="18" />
              <circle cx="11" cy="18" r="1.8" />
            </svg>
          </button>
        </section>
      ) : (
        <section className="mini-card expanded">
          <header className="mini-header" data-tauri-drag-region>
            <div>
              <h1>T's STT</h1>
              <p>{hotkeyReady ? config.hotkey : "Hotkey unavailable"}</p>
            </div>
            <div className="mini-header-actions">
              <button
                className="ghost-btn"
                onClick={() =>
                  updateConfig("theme", config.theme === "light" ? "dark" : "light")
                }
                disabled={busy}
              >
                {config.theme === "light" ? "Dark Mode" : "Light Mode"}
              </button>
              <button className="ghost-btn primary" onClick={handleSaveConfig} disabled={busy}>
                Save
              </button>
              <button className="ghost-btn" onClick={() => setSettingsOpen(false)} disabled={busy}>
                Close
              </button>
            </div>
          </header>

          <button
            className={`listen-btn ${isRecording ? "live" : ""}`}
            onClick={isRecording ? handleStop : handleStart}
            disabled={busy}
          >
            <span className="dot" />
            <span>{isRecording ? "Listening..." : "Tap To Listen"}</span>
          </button>

          <div className="badge-row">
            <span className={`badge ${hotkeyReady ? "ok" : "warn"}`}>
              Hotkey {hotkeyReady ? "On" : "Off"}
            </span>
            <span className={`badge ${modelExists ? "ok" : "warn"}`}>
              Model {modelExists ? "Ready" : "Missing"}
            </span>
            <span className={`badge ${status.toLowerCase().includes("transcrib") ? "live" : ""}`}>
              {status}
            </span>
          </div>

          {info && <p className="info">{info}</p>}
          {error && <p className="error">{error}</p>}

          <section className="settings-panel">
            <div className="settings-head">
              <div className="settings-head-text">
                <h2>Settings</h2>
                <p>Fine-tune hotkey, model, and microphone behavior.</p>
              </div>
              <button className="ghost-btn" onClick={handleCopyPerfLogs} disabled={busy}>
                Copy Perf Logs
              </button>
            </div>

            <div className="form-grid">
              <label>
                Hotkey
                <input
                  value={config.hotkey}
                  onChange={(e) => updateConfig("hotkey", e.currentTarget.value)}
                  placeholder="CommandOrControl+Shift+Space"
                />
              </label>

              <label>
                Output
                <select
                  value={config.output_mode}
                  onChange={(e) => updateConfig("output_mode", e.currentTarget.value)}
                >
                  <option value="clipboard">clipboard</option>
                  <option value="paste">paste</option>
                </select>
              </label>

              <label>
                Model
                <div className="field-inline">
                  <select
                    value={config.model_size}
                    onChange={(e) => updateConfig("model_size", e.currentTarget.value)}
                  >
                    <option value="tiny">tiny</option>
                    <option value="base">base</option>
                    <option value="small">small</option>
                    <option value="medium">medium</option>
                    <option value="large">large-v3</option>
                  </select>
                  <button className="field-action" onClick={handleDownloadModel} disabled={busy}>
                    Download
                  </button>
                </div>
              </label>

              <label>
                Language
                <input
                  value={config.language}
                  onChange={(e) => updateConfig("language", e.currentTarget.value)}
                  placeholder="en"
                />
              </label>

              <label>
                Spoken Punctuation
                <select
                  value={config.spoken_punctuation ? "enabled" : "disabled"}
                  onChange={(e) =>
                    updateConfig("spoken_punctuation", e.currentTarget.value === "enabled")
                  }
                >
                  <option value="enabled">enabled</option>
                  <option value="disabled">disabled</option>
                </select>
              </label>

              <label>
                Model Idle Unload (sec)
                <input
                  type="number"
                  min={0}
                  step={30}
                  value={config.model_idle_unload_secs}
                  onChange={(e) => {
                    const value = Number(e.currentTarget.value);
                    updateConfig("model_idle_unload_secs", Number.isFinite(value) ? value : 300);
                  }}
                />
                <small className="field-hint">0 = unload immediately after transcription</small>
              </label>

              <label className="full-span">
                Microphone
                <div className="field-inline">
                  <select
                    value={config.mic_device ?? "Default"}
                    onChange={(e) =>
                      updateConfig(
                        "mic_device",
                        e.currentTarget.value === "Default" ? null : e.currentTarget.value,
                      )
                    }
                  >
                    <option value="Default">Default</option>
                    {devices.map((device) => (
                      <option key={device} value={device}>
                        {device}
                      </option>
                    ))}
                  </select>
                  <button className="field-action" onClick={handleRefreshDevices} disabled={busy}>
                    Refresh
                  </button>
                </div>
              </label>

              <label className="full-span">
                Model Path
                <input
                  value={config.model_path ?? ""}
                  onChange={(e) => {
                    const value = e.currentTarget.value;
                    updateConfig("model_path", value.length ? value : null);
                  }}
                  placeholder="C:\\models\\ggml-base.bin"
                />
                <small className="field-hint">Optional custom local model file path</small>
              </label>
            </div>

            <div className="paths settings-block">
              <div>
                <strong>Mode:</strong> toggle (only supported mode)
              </div>
            </div>

            <section className="transcript settings-block">
              <h2>Transcript</h2>
              <h3>Final</h3>
              <pre>{finalText || "(empty)"}</pre>
              <h3>Raw</h3>
              <pre>{rawText || "(empty)"}</pre>
            </section>
          </section>
        </section>
      )}
    </main>
  );
}

export default App;
