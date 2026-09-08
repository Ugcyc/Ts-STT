import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  isRegistered,
  register,
  unregisterAll,
} from "@tauri-apps/plugin-global-shortcut";
import "./App.css";

type Replacement = { heard: string; replacement: string };
type TranscriptEntry = {
  id: string;
  created_at: number;
  text: string;
  seconds: number;
};

type AppConfig = {
  hotkey: string;
  mode: string;
  theme: string;
  output_mode: string;
  mic_device: string | null;
  language: string;
  streaming: boolean;
  live_typing: boolean;
  live_update_ms: number;
  key_delay_ms: number;
  model_keep: string;
  spoken_punctuation: boolean;
  silence_timeout_secs: number;
  log_transcripts: boolean;
  model_path: string | null;
  protect_destination: boolean;
  stable_live_text: boolean;
  live_preview: boolean;
  vad_enabled: boolean;
  auto_stop_enabled: boolean;
  replacements_enabled: boolean;
  replacements: Replacement[];
  history_enabled: boolean;
  history_limit: number;
};

type InitialData = {
  config: AppConfig;
  input_devices: string[];
  model_path: string;
  model_exists: boolean;
  status: string;
  history: TranscriptEntry[];
  last_transcript: TranscriptEntry | null;
  protected_insertion_supported: boolean;
};

type TranscriptionResponse = {
  raw_text: string;
  final_text: string;
  seconds: number;
  warning: string | null;
};

type ToggleResponse = {
  is_recording: boolean;
  status: string;
  transcript: TranscriptionResponse | null;
  note: string | null;
};

type DownloadProgress = {
  stage: "downloading" | "skipped" | "extracting" | "complete";
  item: string;
  itemIndex: number;
  itemCount: number;
  bytesDownloaded: number;
  bytesTotal: number | null;
  message: string;
};

const defaultConfig: AppConfig = {
  hotkey: "CommandOrControl+Shift+Space",
  mode: "toggle",
  theme: "dark",
  output_mode: "paste",
  mic_device: null,
  language: "en-US",
  streaming: true,
  live_typing: false,
  live_update_ms: 560,
  key_delay_ms: 0,
  model_keep: "always",
  spoken_punctuation: false,
  silence_timeout_secs: 60,
  log_transcripts: false,
  model_path: null,
  protect_destination: false,
  stable_live_text: false,
  live_preview: false,
  vad_enabled: false,
  auto_stop_enabled: false,
  replacements_enabled: false,
  replacements: [],
  history_enabled: false,
  history_limit: 20,
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
      if (
        part === "meta" ||
        part === "cmd" ||
        part === "command" ||
        part === "super"
      ) {
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

function formatBytes(bytes: number): string {
  if (!bytes) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const unit = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  return `${(bytes / 1024 ** unit).toFixed(unit > 1 ? 1 : 0)} ${units[unit]}`;
}

function prettyHotkey(hotkey: string): string {
  return hotkey
    .split("+")
    .map((part) => (part === "CommandOrControl" ? "Ctrl" : part))
    .join(" + ");
}

function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className={`toggle ${checked ? "on" : ""}`}
      onClick={() => onChange(!checked)}
      disabled={disabled}
    >
      <span className="toggle-knob" />
    </button>
  );
}

const OUTPUT_MODES = [
  {
    value: "paste",
    label: "Paste",
    desc: "Pastes the transcript into the active app when you stop.",
  },
  {
    value: "type",
    label: "Type",
    desc: "Types the words out key by key — allows live typing while you speak.",
  },
  {
    value: "clipboard",
    label: "Clipboard",
    desc: "Only copies the transcript, so you can paste it yourself.",
  },
];

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
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [savedConfig, setSavedConfig] = useState<AppConfig>(defaultConfig);
  const [capturingHotkey, setCapturingHotkey] = useState(false);
  const [preview, setPreview] = useState("");
  const [history, setHistory] = useState<TranscriptEntry[]>([]);
  const [protectedSupported, setProtectedSupported] = useState(true);
  const [hasLastTranscript, setHasLastTranscript] = useState(false);

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
      setHistory(data.history);
      setProtectedSupported(data.protected_insertion_supported);
      setHasLastTranscript(Boolean(data.last_transcript));
      if (data.last_transcript) setFinalText(data.last_transcript.text);
      setConfig(data.config);
      setSavedConfig(data.config);
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
    let disposed = false;
    const unlisteners: (() => void)[] = [];
    const subscriptions = [
      listen<DownloadProgress>("download-progress", (event) => {
        setDownloadProgress(event.payload);
        setInfo(event.payload.message);
      }),
      listen<string>("dictation-preview", (event) => setPreview(event.payload)),
      listen<string>("dictation-warning", (event) => setError(event.payload)),
      listen<TranscriptionResponse>("dictation-completed", (event) => {
        applyTranscript(event.payload);
        setStatus("Idle");
        setIsRecording(false);
        isRecordingRef.current = false;
      }),
    ];
    for (const subscription of subscriptions) {
      void subscription.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    }
    return () => {
      disposed = true;
      void unregisterAll();
      unlisteners.forEach((fn) => fn());
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

  useEffect(() => {
    if (settingsOpen) return;
    const message = Boolean(error || info);
    const showPreview = Boolean(savedConfig.live_preview && preview);
    const showRecovery = hasLastTranscript && !isRecording;
    const height =
      58 +
      (message ? 84 : 0) +
      (showPreview ? 84 : 0) +
      (showRecovery ? 36 : 0);
    void getCurrentWindow()
      .setSize(new LogicalSize(height > 58 ? 320 : 210, height))
      .catch(() => {});
  }, [
    settingsOpen,
    error,
    info,
    preview,
    savedConfig.live_preview,
    hasLastTranscript,
    isRecording,
  ]);

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
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const wasRecording = isRecordingRef.current;
      if (!wasRecording) setPreview("");
      if (wasRecording) {
        setStatus("Transcribing");
      }
      const result = await invoke<ToggleResponse>(
        "toggle_recording_and_process",
      );
      applyToggleResult(result);
    } catch (err) {
      setError(String(err));
      setStatus("Idle");
      setIsRecording(false);
      isRecordingRef.current = false;
    } finally {
      hotkeyInFlight.current = false;
      setBusy(false);
    }
  };

  const applyToggleResult = (result: ToggleResponse) => {
    setStatus(result.status);
    setIsRecording(result.is_recording);
    isRecordingRef.current = result.is_recording;
    if (result.transcript) {
      applyTranscript(result.transcript);
    }
    if (result.note) {
      setInfo(result.note);
    }
  };

  const applyTranscript = (transcript: TranscriptionResponse) => {
    setFinalText(transcript.final_text);
    setRawText(transcript.raw_text);
    setPreview("");
    if (transcript.final_text) setHasLastTranscript(true);
    if (transcript.warning) setError(transcript.warning);
    void invoke<TranscriptEntry[]>("get_transcript_history")
      .then(setHistory)
      .catch((e) => setError(String(e)));
  };

  const handleStart = async () => {
    if (hotkeyInFlight.current) return;
    hotkeyInFlight.current = true;
    setPreview("");
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
      isRecordingRef.current = true;
    } catch (err) {
      setError(String(err));
      setStatus("Idle");
      setIsRecording(false);
      isRecordingRef.current = false;
    } finally {
      setBusy(false);
      hotkeyInFlight.current = false;
    }
  };

  const handleStop = async () => {
    if (hotkeyInFlight.current) return;
    hotkeyInFlight.current = true;
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      setStatus("Transcribing");
      const result = await invoke<TranscriptionResponse>(
        "stop_recording_and_process",
      );
      setStatus("Idle");
      setIsRecording(false);
      isRecordingRef.current = false;
      applyTranscript(result);
      if (!result.final_text) {
        setInfo("No speech detected.");
      }
    } catch (err) {
      setError(String(err));
      setStatus("Idle");
      setIsRecording(false);
      isRecordingRef.current = false;
    } finally {
      setBusy(false);
      hotkeyInFlight.current = false;
    }
  };

  const handleSaveConfig = async () => {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      const saved = await invoke<AppConfig>("save_config", { config });
      setConfig(saved);
      setSavedConfig(saved);
      setHistory(await invoke<TranscriptEntry[]>("get_transcript_history"));
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
    setDownloadProgress(null);
    setInfo("Preparing Nemotron download...");
    try {
      const path = await invoke<string>("download_model");
      setModelExists(true);
      setConfig((prev) => ({ ...prev, model_path: path }));
      setInfo("Nemotron is installed and ready.");
    } catch (err) {
      setError(String(err));
      setInfo("Download stopped. Click Repair Nemotron to resume.");
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

  const transcriptAction = async (
    action: "copy_transcript" | "reinsert_transcript",
    id: string | null = null,
  ) => {
    if (hotkeyInFlight.current) return;
    hotkeyInFlight.current = true;
    setBusy(true);
    setError(null);
    setInfo(
      action === "reinsert_transcript"
        ? "Switch to your text field. Pasting in 3 seconds…"
        : null,
    );
    try {
      await invoke(action, { id });
      setInfo(
        action === "copy_transcript"
          ? "Transcript copied."
          : "Transcript pasted.",
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      hotkeyInFlight.current = false;
    }
  };
  const clearHistory = async () => {
    try {
      await invoke("clear_transcript_history");
      setHistory([]);
      setHasLastTranscript(false);
      setFinalText("");
      setRawText("");
    } catch (e) {
      setError(String(e));
    }
  };

  const updateConfig = <K extends keyof AppConfig>(
    key: K,
    value: AppConfig[K],
  ) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  };

  const handleHotkeyCapture = (e: React.KeyboardEvent<HTMLInputElement>) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      e.currentTarget.blur();
      return;
    }
    // Wait until a non-modifier key completes the combination.
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;

    const mods: string[] = [];
    if (e.ctrlKey || e.metaKey) mods.push("CommandOrControl");
    if (e.shiftKey) mods.push("Shift");
    if (e.altKey) mods.push("Alt");

    const isFunctionKey = /^F\d{1,2}$/.test(e.key);
    if (!mods.length && !isFunctionKey) {
      setInfo("Include a modifier key (Ctrl, Shift, or Alt) in the shortcut.");
      return;
    }

    const main =
      e.key === " "
        ? "Space"
        : e.key.length === 1
          ? e.key.toUpperCase()
          : e.key;
    updateConfig("hotkey", [...mods, main].join("+"));
    setInfo(null);
    e.currentTarget.blur();
  };

  const dirty = JSON.stringify(config) !== JSON.stringify(savedConfig);
  const liveTypingAvailable = config.output_mode === "type" && config.streaming;
  const outputModeDesc = OUTPUT_MODES.find(
    (m) => m.value === config.output_mode,
  )?.desc;

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
  const itemPercent =
    downloadProgress?.bytesTotal && downloadProgress.bytesTotal > 0
      ? Math.min(
          100,
          (downloadProgress.bytesDownloaded / downloadProgress.bytesTotal) *
            100,
        )
      : null;

  return (
    <main
      className={`app-shell ${settingsOpen ? "expanded" : "compact"} theme-${theme}`}
    >
      {!settingsOpen ? (
        <div className="compact-stack">
          <section className={`capsule ${capsuleTone}`} data-tauri-drag-region>
            <button
              className="mic-button"
              onClick={isRecording ? handleStop : handleStart}
              disabled={busy}
            >
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
            <span className="status-text" data-tauri-drag-region>
              {error ? "Needs attention" : compactText}
            </span>
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
          {error && (
            <div className="compact-message error" role="alert">
              {error}
            </div>
          )}
          {!error && info && (
            <div className="compact-message" role="status">
              {info}
            </div>
          )}
          {savedConfig.live_preview && preview && (
            <div className="compact-preview" aria-live="polite">
              {preview}
            </div>
          )}
          {hasLastTranscript && !isRecording && (
            <button
              className="field-action"
              disabled={busy}
              onClick={() => transcriptAction("copy_transcript")}
            >
              Copy last dictation
            </button>
          )}
        </div>
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
                  updateConfig(
                    "theme",
                    config.theme === "light" ? "dark" : "light",
                  )
                }
                disabled={busy}
              >
                {config.theme === "light" ? "Dark Mode" : "Light Mode"}
              </button>
              <button
                className={`ghost-btn primary ${dirty ? "attention" : ""}`}
                onClick={handleSaveConfig}
                disabled={busy || isRecording || !dirty}
              >
                {dirty ? "Save changes" : "Saved"}
              </button>
              <button
                className="ghost-btn"
                onClick={() => setSettingsOpen(false)}
                disabled={busy}
              >
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
            <span
              className={`badge ${status.toLowerCase().includes("transcrib") ? "live" : ""}`}
            >
              {status}
            </span>
          </div>

          {info && <p className="info">{info}</p>}
          {error && <p className="error">{error}</p>}
          {downloadProgress && downloadProgress.stage !== "complete" && (
            <section className="download-progress" aria-live="polite">
              <div className="download-progress-head">
                <strong>{downloadProgress.message}</strong>
                <span>
                  File {downloadProgress.itemIndex} of{" "}
                  {downloadProgress.itemCount}
                </span>
              </div>
              <div
                className={`progress-track ${itemPercent === null ? "indeterminate" : ""}`}
              >
                <span
                  style={
                    itemPercent === null
                      ? undefined
                      : { width: `${itemPercent}%` }
                  }
                />
              </div>
              <div className="download-progress-detail">
                <span>{downloadProgress.item}</span>
                {downloadProgress.bytesTotal ? (
                  <span>
                    {formatBytes(downloadProgress.bytesDownloaded)} /{" "}
                    {formatBytes(downloadProgress.bytesTotal)}
                  </span>
                ) : (
                  <span>{downloadProgress.stage}</span>
                )}
              </div>
            </section>
          )}

          <section className="settings-panel">
            <section className="settings-group">
              <h3 className="settings-group-title">Recording</h3>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Keyboard shortcut</span>
                  <span className="setting-desc">
                    {capturingHotkey
                      ? "Press the new key combination, or Esc to cancel"
                      : "Starts and stops recording from any app"}
                  </span>
                </div>
                <div className="setting-control">
                  <input
                    className={`hotkey-input ${capturingHotkey ? "capturing" : ""}`}
                    value={
                      capturingHotkey
                        ? "Press keys..."
                        : prettyHotkey(config.hotkey)
                    }
                    onKeyDown={handleHotkeyCapture}
                    onFocus={() => setCapturingHotkey(true)}
                    onBlur={() => setCapturingHotkey(false)}
                    readOnly
                  />
                  <button
                    className="field-action"
                    onClick={() => updateConfig("hotkey", defaultConfig.hotkey)}
                    disabled={busy}
                  >
                    Reset
                  </button>
                </div>
              </div>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Microphone</span>
                  <span className="setting-desc">
                    Input device used for recording
                  </span>
                </div>
                <div className="setting-control">
                  <select
                    value={config.mic_device ?? "Default"}
                    onChange={(e) =>
                      updateConfig(
                        "mic_device",
                        e.currentTarget.value === "Default"
                          ? null
                          : e.currentTarget.value,
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
                  <button
                    className="field-action"
                    onClick={handleRefreshDevices}
                    disabled={busy}
                  >
                    Refresh
                  </button>
                </div>
              </div>
            </section>

            <section className="settings-group">
              <h3 className="settings-group-title">
                Optional dictation features
              </h3>
              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Protect the destination</span>
                  <span className="setting-desc">
                    {protectedSupported
                      ? "Pause insertion if you change apps, click, or type. Start and stop with your hotkey."
                      : "Protected insertion is Windows-only. Turn off to allow insertion on this platform, or use Clipboard mode."}
                  </span>
                </div>
                <Toggle
                  checked={config.protect_destination}
                  onChange={(v) => updateConfig("protect_destination", v)}
                  label="Protect the destination"
                />
              </div>
              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Live transcript preview</span>
                  <span className="setting-desc">
                    Show recognized words in the overlay without typing into
                    another app.
                  </span>
                </div>
                <Toggle
                  checked={config.live_preview}
                  onChange={(v) => updateConfig("live_preview", v)}
                  label="Live transcript preview"
                />
              </div>
              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">
                    Hold back unfinished words
                  </span>
                  <span className="setting-desc">
                    Delay inserting the last two words until more speech
                    arrives. Stopping inserts everything.
                  </span>
                </div>
                <Toggle
                  checked={config.stable_live_text}
                  onChange={(v) => updateConfig("stable_live_text", v)}
                  label="Hold back unfinished words"
                />
              </div>
              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Filter long silences</span>
                  <span className="setting-desc">
                    Use local speech detection to skip extended silence. Does
                    not stop recording by itself.
                  </span>
                </div>
                <Toggle
                  checked={config.vad_enabled}
                  onChange={(v) =>
                    setConfig((prev) => ({
                      ...prev,
                      vad_enabled: v,
                      auto_stop_enabled: v && prev.auto_stop_enabled,
                    }))
                  }
                  label="Filter long silences"
                />
              </div>
              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Stop after silence</span>
                  <span className="setting-desc">
                    Off by default so you can pause to think. Starts counting
                    after speech and the detection grace period.
                  </span>
                </div>
                <Toggle
                  checked={config.auto_stop_enabled}
                  disabled={!config.vad_enabled}
                  onChange={(v) => updateConfig("auto_stop_enabled", v)}
                  label="Stop after silence"
                />
              </div>
              {config.auto_stop_enabled && (
                <div className="setting-row">
                  <label htmlFor="silence-seconds">
                    Silence timeout (seconds)
                  </label>
                  <input
                    id="silence-seconds"
                    type="number"
                    min={2}
                    max={600}
                    value={config.silence_timeout_secs}
                    onChange={(e) =>
                      updateConfig(
                        "silence_timeout_secs",
                        Number(e.target.value),
                      )
                    }
                  />
                </div>
              )}
              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">
                    Save local transcript history
                  </span>
                  <span className="setting-desc">
                    Optional text-only history on this device. No audio is
                    saved. Turning this off and saving deletes the stored
                    history.
                  </span>
                </div>
                <Toggle
                  checked={config.history_enabled}
                  onChange={(v) => updateConfig("history_enabled", v)}
                  label="Save local transcript history"
                />
              </div>
              {config.history_enabled && (
                <div className="setting-row">
                  <label htmlFor="history-limit">Transcripts to keep</label>
                  <input
                    id="history-limit"
                    type="number"
                    min={1}
                    max={100}
                    value={config.history_limit}
                    onChange={(e) =>
                      updateConfig("history_limit", Number(e.target.value))
                    }
                  />
                </div>
              )}
            </section>
            <section className="settings-group">
              <h3 className="settings-group-title">Your phrase replacements</h3>
              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Apply replacements</span>
                  <span className="setting-desc">
                    Only phrases you add here. Matches whole phrases, ignoring
                    capitalization.
                  </span>
                </div>
                <Toggle
                  checked={config.replacements_enabled}
                  onChange={(v) => updateConfig("replacements_enabled", v)}
                  label="Apply replacements"
                />
              </div>
              {config.replacements.length === 0 && (
                <p className="field-hint">No replacements added.</p>
              )}
              {config.replacements.map((rule, index) => (
                <div className="replacement-row" key={index}>
                  <input
                    aria-label={`Heard phrase ${index + 1}`}
                    placeholder="What it hears"
                    value={rule.heard}
                    maxLength={200}
                    onChange={(e) =>
                      updateConfig(
                        "replacements",
                        config.replacements.map((r, i) =>
                          i === index ? { ...r, heard: e.target.value } : r,
                        ),
                      )
                    }
                  />
                  <input
                    aria-label={`Replacement ${index + 1}`}
                    placeholder="What to write"
                    value={rule.replacement}
                    maxLength={2000}
                    onChange={(e) =>
                      updateConfig(
                        "replacements",
                        config.replacements.map((r, i) =>
                          i === index
                            ? { ...r, replacement: e.target.value }
                            : r,
                        ),
                      )
                    }
                  />
                  <button
                    className="field-action"
                    aria-label={`Remove replacement ${index + 1}`}
                    onClick={() =>
                      updateConfig(
                        "replacements",
                        config.replacements.filter((_, i) => i !== index),
                      )
                    }
                  >
                    Remove
                  </button>
                </div>
              ))}
              <button
                className="field-action"
                disabled={config.replacements.length >= 200}
                onClick={() =>
                  updateConfig("replacements", [
                    ...config.replacements,
                    { heard: "", replacement: "" },
                  ])
                }
              >
                Add replacement
              </button>
            </section>
            <section className="settings-group">
              <h3 className="settings-group-title">Speech Model</h3>
              <div className="model-card">
                <div className="model-card-row">
                  <div>
                    <strong>Nemotron 3.5 ASR Streaming</strong>
                    <small className="field-hint">
                      Local 0.6B ONNX Runtime GenAI model
                    </small>
                  </div>
                  <button
                    className="field-action"
                    onClick={handleDownloadModel}
                    disabled={busy}
                  >
                    {busy && downloadProgress
                      ? "Downloading..."
                      : modelExists
                        ? "Repair Nemotron"
                        : "Download Nemotron"}
                  </button>
                </div>
                <span
                  className={`model-state ${modelExists ? "ready" : "missing"}`}
                >
                  {modelExists ? "Installed and ready" : "Not installed"}
                </span>
                <div className="model-facts">
                  <span>Quantization: INT4</span>
                  <span>Native inference chunk: 560 ms</span>
                </div>
              </div>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Keep model in memory</span>
                  <span className="setting-desc">
                    {config.model_keep === "always"
                      ? "Fastest: the model stays loaded once it has been used"
                      : "Frees RAM when idle; the model reloads in the background while you speak"}
                  </span>
                </div>
                <select
                  value={config.model_keep}
                  onChange={(e) =>
                    updateConfig("model_keep", e.currentTarget.value)
                  }
                >
                  <option value="always">Always (fastest)</option>
                  <option value="15min">For 15 min after use</option>
                  <option value="5min">For 5 min after use</option>
                  <option value="after_use">Only while dictating</option>
                </select>
              </div>
            </section>

            <section className="settings-group">
              <h3 className="settings-group-title">Output</h3>

              <div className="setting-row column">
                <div className="setting-text">
                  <span className="setting-label">Where the text goes</span>
                  <span className="setting-desc">{outputModeDesc}</span>
                </div>
                <div
                  className="segmented"
                  role="radiogroup"
                  aria-label="Output mode"
                >
                  {OUTPUT_MODES.map((mode) => (
                    <button
                      key={mode.value}
                      type="button"
                      role="radio"
                      aria-checked={config.output_mode === mode.value}
                      className={`segment ${config.output_mode === mode.value ? "active" : ""}`}
                      onClick={() => updateConfig("output_mode", mode.value)}
                    >
                      {mode.label}
                    </button>
                  ))}
                </div>
              </div>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Typing speed</span>
                  <span className="setting-desc">
                    {config.output_mode === "type"
                      ? "How fast the keyboard types the words out"
                      : 'Only used when Output is set to "Type"'}
                  </span>
                </div>
                <select
                  value={config.key_delay_ms}
                  onChange={(e) =>
                    updateConfig("key_delay_ms", Number(e.currentTarget.value))
                  }
                  disabled={config.output_mode !== "type"}
                >
                  <option value={0}>Instant</option>
                  <option value={8}>Fast (~120 keys/sec)</option>
                  <option value={16}>Medium (~60 keys/sec)</option>
                  <option value={32}>Slow (~30 keys/sec)</option>
                </select>
              </div>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Live typing</span>
                  <span className="setting-desc">
                    {liveTypingAvailable
                      ? "Type words on screen while you speak"
                      : 'Needs Output set to "Type" and Streaming turned on'}
                  </span>
                </div>
                <Toggle
                  checked={config.live_typing && liveTypingAvailable}
                  onChange={(value) => updateConfig("live_typing", value)}
                  disabled={!liveTypingAvailable}
                  label="Live typing"
                />
              </div>

              {config.live_typing && liveTypingAvailable && (
                <div className="setting-row">
                  <div className="setting-text">
                    <span className="setting-label">Typing delay</span>
                    <span className="setting-desc">
                      How long to wait before new words appear; longer delays
                      make fewer visible corrections
                    </span>
                  </div>
                  <select
                    value={config.live_update_ms}
                    onChange={(e) =>
                      updateConfig(
                        "live_update_ms",
                        Number(e.currentTarget.value),
                      )
                    }
                  >
                    <option value={560}>Short (560 ms)</option>
                    <option value={1120}>Medium (1120 ms)</option>
                    <option value={1680}>Long (1680 ms)</option>
                  </select>
                </div>
              )}
            </section>

            <section className="settings-group">
              <h3 className="settings-group-title">Transcription</h3>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Language</span>
                  <span className="setting-desc">
                    Language you will be speaking
                  </span>
                </div>
                <select
                  value={config.language}
                  onChange={(e) =>
                    updateConfig("language", e.currentTarget.value)
                  }
                >
                  <option value="en-US">English (US)</option>
                  <option value="en-GB">English (UK)</option>
                  <option value="auto">Auto detect</option>
                  <option value="fr">French</option>
                  <option value="es">Spanish</option>
                  <option value="de">German</option>
                  <option value="pt">Portuguese</option>
                  <option value="it">Italian</option>
                  <option value="ja">Japanese</option>
                  <option value="ko">Korean</option>
                  <option value="zh-CN">Chinese (Simplified)</option>
                </select>
              </div>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Streaming</span>
                  <span className="setting-desc">
                    Enable live typing while speaking. Audio always processes in
                    the background to avoid a large backlog.
                  </span>
                </div>
                <Toggle
                  checked={config.streaming}
                  onChange={(value) => updateConfig("streaming", value)}
                  label="Streaming"
                />
              </div>

              <div className="setting-row">
                <div className="setting-text">
                  <span className="setting-label">Spoken punctuation</span>
                  <span className="setting-desc">
                    Say "comma" or "period" to insert punctuation
                  </span>
                </div>
                <Toggle
                  checked={config.spoken_punctuation}
                  onChange={(value) =>
                    updateConfig("spoken_punctuation", value)
                  }
                  label="Spoken punctuation"
                />
              </div>
            </section>

            <section className="settings-group">
              <button
                className="advanced-toggle"
                onClick={() => setShowAdvanced((value) => !value)}
              >
                {showAdvanced
                  ? "Hide advanced settings"
                  : "Show advanced settings"}
              </button>

              {showAdvanced && (
                <>
                  <div className="setting-row column">
                    <div className="setting-text">
                      <span className="setting-label">
                        Custom model directory
                      </span>
                      <span className="setting-desc">
                        Leave empty to use the managed Nemotron download
                      </span>
                    </div>
                    <input
                      value={config.model_path ?? ""}
                      onChange={(e) => {
                        const value = e.currentTarget.value;
                        updateConfig("model_path", value.length ? value : null);
                      }}
                      placeholder="C:\path\to\nemotron-onnx"
                    />
                  </div>

                  <div className="setting-row">
                    <div className="setting-text">
                      <span className="setting-label">Performance report</span>
                      <span className="setting-desc">
                        Copies measured load time, inference time, first-text
                        delay, and audio backlog for the last dictation
                      </span>
                    </div>
                    <button
                      className="field-action"
                      onClick={handleCopyPerfLogs}
                      disabled={busy}
                    >
                      Copy
                    </button>
                  </div>

                  <div className="paths">
                    <div>
                      <strong>Backend:</strong> Nemotron 3.5 ASR Streaming ONNX
                      INT4
                    </div>
                    <div>
                      <strong>App version:</strong> 0.4.0
                    </div>
                  </div>
                </>
              )}
            </section>

            <section className="transcript settings-group">
              <h3 className="settings-group-title">Last Transcript</h3>
              {savedConfig.live_preview && preview && (
                <pre aria-live="polite">{preview}</pre>
              )}
              <div className="transcript-actions">
                <button
                  className="field-action"
                  disabled={!hasLastTranscript || busy}
                  onClick={() => transcriptAction("copy_transcript")}
                >
                  Copy last
                </button>
                <button
                  className="field-action"
                  disabled={!hasLastTranscript || busy || isRecording}
                  onClick={() => transcriptAction("reinsert_transcript")}
                >
                  Reinsert in 3 seconds
                </button>
              </div>
              <h3>Final</h3>
              <pre>{finalText || "(empty)"}</pre>
              <h3>Raw</h3>
              <pre>{rawText || "(empty)"}</pre>
              {history.length > 0 && (
                <>
                  <h3>Saved on this device</h3>
                  {history.map((entry) => (
                    <article key={entry.id} className="history-entry">
                      <small>
                        {new Date(entry.created_at * 1000).toLocaleString()}
                      </small>
                      <pre>{entry.text}</pre>
                      <div className="transcript-actions">
                        <button
                          className="field-action"
                          disabled={busy}
                          onClick={() =>
                            transcriptAction("copy_transcript", entry.id)
                          }
                        >
                          Copy
                        </button>
                        <button
                          className="field-action"
                          disabled={busy || isRecording}
                          onClick={() =>
                            transcriptAction("reinsert_transcript", entry.id)
                          }
                        >
                          Reinsert in 3 seconds
                        </button>
                      </div>
                    </article>
                  ))}
                </>
              )}
              <button
                className="field-action"
                disabled={
                  busy ||
                  isRecording ||
                  (!hasLastTranscript && history.length === 0)
                }
                onClick={clearHistory}
              >
                Clear transcripts
              </button>
            </section>
          </section>
        </section>
      )}
    </main>
  );
}

export default App;
