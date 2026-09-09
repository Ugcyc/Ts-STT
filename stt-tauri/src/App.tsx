import { useEffect, useRef, useState, type ReactNode } from "react";
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

type SectionId =
  | "general"
  | "output"
  | "recognition"
  | "replacements"
  | "transcripts"
  | "model"
  | "diagnostics";

const SECTIONS: { id: SectionId; label: string; blurb: string }[] = [
  {
    id: "general",
    label: "General",
    blurb: "Shortcut, microphone, language, and appearance.",
  },
  {
    id: "output",
    label: "Output",
    blurb: "Where the text goes and how it is inserted.",
  },
  {
    id: "recognition",
    label: "Recognition",
    blurb: "Streaming, spoken punctuation, and silence handling.",
  },
  {
    id: "replacements",
    label: "Replacements",
    blurb: "Phrases to rewrite after recognition.",
  },
  {
    id: "transcripts",
    label: "Transcripts",
    blurb: "Recover, copy, reinsert, and store transcripts.",
  },
  {
    id: "model",
    label: "Speech model",
    blurb: "Nemotron installation and memory use.",
  },
  {
    id: "diagnostics",
    label: "Diagnostics",
    blurb: "Performance data and version information.",
  },
];

type SettingItem = {
  id: string;
  section: SectionId;
  label: string;
  desc?: string;
  keywords?: string;
  keys?: (keyof AppConfig)[];
  hidden?: boolean;
  htmlFor?: string;
  control?: ReactNode;
  block?: ReactNode;
};

function matchesQuery(item: SettingItem, tokens: string[]): boolean {
  const section = SECTIONS.find((s) => s.id === item.section)?.label ?? "";
  const haystack =
    `${item.label} ${item.desc ?? ""} ${item.keywords ?? ""} ${section}`.toLowerCase();
  return tokens.every((token) => haystack.includes(token));
}

function SettingRow({ item }: { item: SettingItem }) {
  const text = (
    <div className="setting-text">
      {item.htmlFor ? (
        <label className="setting-label" htmlFor={item.htmlFor}>
          {item.label}
        </label>
      ) : (
        <span className="setting-label">{item.label}</span>
      )}
      {item.desc && <span className="setting-desc">{item.desc}</span>}
    </div>
  );
  if (item.block) {
    return (
      <div className="setting-row column" data-setting={item.id}>
        {text}
        {item.block}
      </div>
    );
  }
  return (
    <div className="setting-row" data-setting={item.id}>
      {text}
      {item.control}
    </div>
  );
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
  const [downloadProgress, setDownloadProgress] =
    useState<DownloadProgress | null>(null);
  const [activeSection, setActiveSection] = useState<SectionId>("general");
  const [query, setQuery] = useState("");
  const [savedConfig, setSavedConfig] = useState<AppConfig>(defaultConfig);
  const [capturingHotkey, setCapturingHotkey] = useState(false);
  const [preview, setPreview] = useState("");
  const [history, setHistory] = useState<TranscriptEntry[]>([]);
  const [protectedSupported, setProtectedSupported] = useState(true);
  const [hasLastTranscript, setHasLastTranscript] = useState(false);

  const hotkeyInFlight = useRef(false);
  const searchRef = useRef<HTMLInputElement>(null);
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
    if (!settingsOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
        searchRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [settingsOpen]);

  useEffect(() => {
    if (settingsOpen) return;
    const message = Boolean(error || info);
    const showPreview = Boolean(savedConfig.live_preview && preview);
    const height = 58 + (message ? 84 : 0) + (showPreview ? 84 : 0);
    void getCurrentWindow()
      .setSize(new LogicalSize(height > 58 ? 320 : 210, height))
      .catch(() => {});
  }, [
    settingsOpen,
    error,
    info,
    preview,
    savedConfig.live_preview,
  ]);

  // Informational notes are transient in the overlay; errors stay until the
  // next action clears them.
  useEffect(() => {
    if (settingsOpen || !info || error) return;
    const timer = window.setTimeout(() => setInfo(null), 4000);
    return () => window.clearTimeout(timer);
  }, [settingsOpen, info, error]);

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

  const items: SettingItem[] = [
    {
      id: "hotkey",
      section: "general",
      label: "Keyboard shortcut",
      desc: capturingHotkey
        ? "Press the new key combination, or Esc to cancel"
        : "Starts and stops recording from any app",
      keywords: "hotkey keys global toggle reset",
      keys: ["hotkey"],
      control: (
        <div className="setting-control">
          <input
            className={`hotkey-input ${capturingHotkey ? "capturing" : ""}`}
            value={
              capturingHotkey ? "Press keys..." : prettyHotkey(config.hotkey)
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
      ),
    },
    {
      id: "microphone",
      section: "general",
      label: "Microphone",
      desc: "Input device used for recording",
      keywords: "audio input device refresh",
      keys: ["mic_device"],
      control: (
        <div className="setting-control">
          <select
            aria-label="Microphone"
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
      ),
    },
    {
      id: "language",
      section: "general",
      label: "Language",
      desc: "Language you will be speaking",
      keywords:
        "english french spanish german portuguese italian japanese korean chinese auto detect",
      keys: ["language"],
      control: (
        <select
          aria-label="Language"
          value={config.language}
          onChange={(e) => updateConfig("language", e.currentTarget.value)}
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
      ),
    },
    {
      id: "theme",
      section: "general",
      label: "Appearance",
      desc: "Light or dark look for this window",
      keywords: "theme dark light mode colors",
      keys: ["theme"],
      control: (
        <div className="segmented compact" role="radiogroup" aria-label="Theme">
          {(["dark", "light"] as const).map((value) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={theme === value}
              className={`segment ${theme === value ? "active" : ""}`}
              onClick={() => updateConfig("theme", value)}
            >
              {value === "dark" ? "Dark" : "Light"}
            </button>
          ))}
        </div>
      ),
    },
    {
      id: "output_mode",
      section: "output",
      label: "Where the text goes",
      desc: outputModeDesc,
      keywords: "paste type clipboard insert output mode destination",
      keys: ["output_mode"],
      block: (
        <div className="segmented" role="radiogroup" aria-label="Output mode">
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
      ),
    },
    {
      id: "key_delay",
      section: "output",
      label: "Typing speed",
      desc:
        config.output_mode === "type"
          ? "How fast the keyboard types the words out"
          : 'Only used when Output is set to "Type"',
      keywords: "keys per second delay instant fast slow",
      keys: ["key_delay_ms"],
      control: (
        <select
          aria-label="Typing speed"
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
      ),
    },
    {
      id: "live_typing",
      section: "output",
      label: "Live typing",
      desc: liveTypingAvailable
        ? "Type words on screen while you speak"
        : 'Needs Output set to "Type" and Streaming turned on',
      keywords: "real time while speaking",
      keys: ["live_typing"],
      control: (
        <Toggle
          checked={config.live_typing && liveTypingAvailable}
          onChange={(value) => updateConfig("live_typing", value)}
          disabled={!liveTypingAvailable}
          label="Live typing"
        />
      ),
    },
    {
      id: "live_update",
      section: "output",
      label: "Typing delay",
      desc: "How long to wait before new words appear; longer delays make fewer visible corrections",
      keywords: "live update interval corrections",
      keys: ["live_update_ms"],
      hidden: !(config.live_typing && liveTypingAvailable),
      control: (
        <select
          aria-label="Typing delay"
          value={config.live_update_ms}
          onChange={(e) =>
            updateConfig("live_update_ms", Number(e.currentTarget.value))
          }
        >
          <option value={560}>Short (560 ms)</option>
          <option value={1120}>Medium (1120 ms)</option>
          <option value={1680}>Long (1680 ms)</option>
        </select>
      ),
    },
    {
      id: "stable_live_text",
      section: "output",
      label: "Hold back unfinished words",
      desc: "Delay inserting the last two words until more speech arrives. Stopping inserts everything.",
      keywords: "stable live typing text corrections steady",
      keys: ["stable_live_text"],
      control: (
        <Toggle
          checked={config.stable_live_text}
          onChange={(v) => updateConfig("stable_live_text", v)}
          label="Hold back unfinished words"
        />
      ),
    },
    {
      id: "protect_destination",
      section: "output",
      label: "Protect the destination",
      desc: protectedSupported
        ? "Pause insertion if you change apps, click, or type. Start and stop with your hotkey."
        : "Protected insertion is Windows-only. Turn off to allow insertion on this platform, or use Clipboard mode.",
      keywords: "focus window safety pause insertion click typing guard",
      keys: ["protect_destination"],
      control: (
        <Toggle
          checked={config.protect_destination}
          onChange={(v) => updateConfig("protect_destination", v)}
          label="Protect the destination"
        />
      ),
    },
    {
      id: "live_preview",
      section: "output",
      label: "Live transcript preview",
      desc: "Show recognized words in the overlay without typing into another app.",
      keywords: "overlay partial words compact",
      keys: ["live_preview"],
      control: (
        <Toggle
          checked={config.live_preview}
          onChange={(v) => updateConfig("live_preview", v)}
          label="Live transcript preview"
        />
      ),
    },
    {
      id: "streaming",
      section: "recognition",
      label: "Streaming",
      desc: "Enable live typing while speaking. Audio always processes in the background to avoid a large backlog.",
      keywords: "chunks real time",
      keys: ["streaming"],
      control: (
        <Toggle
          checked={config.streaming}
          onChange={(value) => updateConfig("streaming", value)}
          label="Streaming"
        />
      ),
    },
    {
      id: "spoken_punctuation",
      section: "recognition",
      label: "Spoken punctuation",
      desc: 'Say "comma" or "period" to insert punctuation',
      keywords: "comma period new line paragraph question mark exclamation",
      keys: ["spoken_punctuation"],
      control: (
        <Toggle
          checked={config.spoken_punctuation}
          onChange={(value) => updateConfig("spoken_punctuation", value)}
          label="Spoken punctuation"
        />
      ),
    },
    {
      id: "vad",
      section: "recognition",
      label: "Filter long silences",
      desc: "Use local speech detection to skip extended silence. Does not stop recording by itself.",
      keywords: "vad voice activity detection skip quiet",
      keys: ["vad_enabled"],
      control: (
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
      ),
    },
    {
      id: "auto_stop",
      section: "recognition",
      label: "Stop after silence",
      desc: config.vad_enabled
        ? "Off by default so you can pause to think. Starts counting after speech and the detection grace period."
        : "Turn on Filter long silences first.",
      keywords: "auto stop timeout pause think end recording",
      keys: ["auto_stop_enabled"],
      control: (
        <Toggle
          checked={config.auto_stop_enabled}
          disabled={!config.vad_enabled}
          onChange={(v) => updateConfig("auto_stop_enabled", v)}
          label="Stop after silence"
        />
      ),
    },
    {
      id: "silence_timeout",
      section: "recognition",
      label: "Silence timeout (seconds)",
      desc: "Between 2 and 600 seconds of silence before recording stops",
      keywords: "auto stop duration",
      keys: ["silence_timeout_secs"],
      hidden: !config.auto_stop_enabled,
      htmlFor: "silence-seconds",
      control: (
        <input
          id="silence-seconds"
          type="number"
          min={2}
          max={600}
          value={config.silence_timeout_secs}
          onChange={(e) =>
            updateConfig("silence_timeout_secs", Number(e.target.value))
          }
        />
      ),
    },
    {
      id: "replacements_enabled",
      section: "replacements",
      label: "Apply replacements",
      desc: "Only phrases you add here. Matches whole phrases, ignoring capitalization.",
      keywords: "enable phrase rewrite",
      keys: ["replacements_enabled"],
      control: (
        <Toggle
          checked={config.replacements_enabled}
          onChange={(v) => updateConfig("replacements_enabled", v)}
          label="Apply replacements"
        />
      ),
    },
    {
      id: "replacements",
      section: "replacements",
      label: "Phrase list",
      desc: "Each row rewrites a heard phrase into what you want written. Longer phrases win and results never cascade.",
      keywords: "vocabulary dictionary correction custom words heard write add remove",
      keys: ["replacements"],
      block: (
        <div className="replacement-list">
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
                      i === index ? { ...r, replacement: e.target.value } : r,
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
        </div>
      ),
    },
    {
      id: "history_enabled",
      section: "transcripts",
      label: "Save local transcript history",
      desc: "Optional text-only history on this device. No audio is saved. Turning this off and saving deletes the stored history.",
      keywords: "store keep persist privacy",
      keys: ["history_enabled"],
      control: (
        <Toggle
          checked={config.history_enabled}
          onChange={(v) => updateConfig("history_enabled", v)}
          label="Save local transcript history"
        />
      ),
    },
    {
      id: "history_limit",
      section: "transcripts",
      label: "Transcripts to keep",
      desc: "Between 1 and 100 entries; older ones are dropped",
      keywords: "history limit count",
      keys: ["history_limit"],
      hidden: !config.history_enabled,
      htmlFor: "history-limit",
      control: (
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
      ),
    },
    {
      id: "last_transcript",
      section: "transcripts",
      label: "Last transcript",
      desc: "The most recent dictation, kept in memory until the app closes.",
      keywords: "copy reinsert paste final raw recover recovery",
      block: (
        <div className="transcript">
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
        </div>
      ),
    },
    {
      id: "history",
      section: "transcripts",
      label: "Saved on this device",
      desc: history.length
        ? `${history.length} stored ${history.length === 1 ? "transcript" : "transcripts"}`
        : "Nothing stored yet. Turn on local history to keep transcripts between sessions.",
      keywords: "history stored clear delete list",
      block: (
        <div className="transcript">
          {history.map((entry) => (
            <article key={entry.id} className="history-entry">
              <small>{new Date(entry.created_at * 1000).toLocaleString()}</small>
              <pre>{entry.text}</pre>
              <div className="transcript-actions">
                <button
                  className="field-action"
                  disabled={busy}
                  onClick={() => transcriptAction("copy_transcript", entry.id)}
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
        </div>
      ),
    },
    {
      id: "model",
      section: "model",
      label: "Nemotron 3.5 ASR Streaming",
      desc: "Local 0.6B ONNX Runtime GenAI model",
      keywords: "download repair install nemotron onnx runtime",
      block: (
        <div className="model-card">
          <div className="model-card-row">
            <span
              className={`model-state ${modelExists ? "ready" : "missing"}`}
            >
              {modelExists ? "Installed and ready" : "Not installed"}
            </span>
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
          <div className="model-facts">
            <span>Quantization: INT4</span>
            <span>Native inference chunk: 560 ms</span>
            <span>Runs fully offline</span>
          </div>
        </div>
      ),
    },
    {
      id: "model_keep",
      section: "model",
      label: "Keep model in memory",
      desc:
        config.model_keep === "always"
          ? "Fastest: the model stays loaded once it has been used"
          : "Frees RAM when idle; the model reloads in the background while you speak",
      keywords: "ram memory unload idle cache",
      keys: ["model_keep"],
      control: (
        <select
          aria-label="Keep model in memory"
          value={config.model_keep}
          onChange={(e) => updateConfig("model_keep", e.currentTarget.value)}
        >
          <option value="always">Always (fastest)</option>
          <option value="15min">For 15 min after use</option>
          <option value="5min">For 5 min after use</option>
          <option value="after_use">Only while dictating</option>
        </select>
      ),
    },
    {
      id: "model_path",
      section: "model",
      label: "Custom model directory",
      desc: "Leave empty to use the managed Nemotron download",
      keywords: "path folder advanced location",
      keys: ["model_path"],
      block: (
        <input
          aria-label="Custom model directory"
          value={config.model_path ?? ""}
          onChange={(e) => {
            const value = e.currentTarget.value;
            updateConfig("model_path", value.length ? value : null);
          }}
          placeholder="C:\path\to\nemotron-onnx"
        />
      ),
    },
    {
      id: "performance",
      section: "diagnostics",
      label: "Performance report",
      desc: "Copies measured load time, inference time, first-text delay, and audio backlog for the last dictation",
      keywords: "timing logs troubleshooting copy real time factor",
      control: (
        <button
          className="field-action"
          onClick={handleCopyPerfLogs}
          disabled={busy}
        >
          Copy
        </button>
      ),
    },
    {
      id: "about",
      section: "diagnostics",
      label: "About",
      desc: "Backend and version information",
      keywords: "version app backend",
      block: (
        <div className="paths">
          <div>
            <strong>Backend:</strong> Nemotron 3.5 ASR Streaming ONNX INT4
          </div>
          <div>
            <strong>App version:</strong> 0.5.0
          </div>
        </div>
      ),
    },
  ];
  const visibleItems = items.filter((item) => !item.hidden);
  const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
  const searching = tokens.length > 0;
  const results = searching
    ? visibleItems.filter((item) => matchesQuery(item, tokens))
    : [];
  const dirtyKeys = new Set(
    (Object.keys(config) as (keyof AppConfig)[]).filter(
      (key) => JSON.stringify(config[key]) !== JSON.stringify(savedConfig[key]),
    ),
  );
  const dirtySections = new Set(
    items
      .filter((item) => item.keys?.some((key) => dirtyKeys.has(key)))
      .map((item) => item.section),
  );
  const currentSection =
    SECTIONS.find((section) => section.id === activeSection) ?? SECTIONS[0];

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
        </div>
      ) : (
        <section className="mini-card expanded">
          <header className="mini-header" data-tauri-drag-region>
            <div>
              <h1>T's STT</h1>
              <p>
                {hotkeyReady
                  ? prettyHotkey(config.hotkey)
                  : "Hotkey unavailable"}
              </p>
            </div>
            <div className="mini-header-actions">
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

          <div className="status-strip">
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

          <div className="settings-layout">
            <nav className="settings-nav" aria-label="Settings sections">
              <input
                ref={searchRef}
                type="search"
                className="settings-search"
                placeholder="Search settings"
                aria-label="Search settings"
                value={query}
                onChange={(e) => setQuery(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    setQuery("");
                    e.currentTarget.blur();
                  }
                }}
              />
              {SECTIONS.map((section) => {
                const active = !searching && activeSection === section.id;
                return (
                  <button
                    key={section.id}
                    type="button"
                    className={`nav-item ${active ? "active" : ""}`}
                    aria-current={active ? "page" : undefined}
                    aria-label={section.label}
                    data-dirty={dirtySections.has(section.id) || undefined}
                    title={
                      dirtySections.has(section.id)
                        ? `${section.label}: unsaved changes`
                        : undefined
                    }
                    onClick={() => {
                      setQuery("");
                      setActiveSection(section.id);
                    }}
                  >
                    <span>{section.label}</span>
                    {dirtySections.has(section.id) && (
                      <span className="nav-dot" aria-hidden="true" />
                    )}
                  </button>
                );
              })}
              <p className="nav-hint">Ctrl+F searches settings</p>
            </nav>

            <div className="settings-content">
              {searching ? (
                results.length === 0 ? (
                  <div className="empty-state" role="status">
                    <strong>No settings match "{query.trim()}"</strong>
                    <span>
                      Try another word, such as "typing", "silence", or
                      "model".
                    </span>
                  </div>
                ) : (
                  <>
                    <p className="results-count" role="status">
                      {results.length} matching{" "}
                      {results.length === 1 ? "setting" : "settings"}
                    </p>
                    {SECTIONS.filter((section) =>
                      results.some((item) => item.section === section.id),
                    ).map((section) => (
                      <section className="settings-group" key={section.id}>
                        <button
                          type="button"
                          className="settings-group-title jump"
                          onClick={() => {
                            setQuery("");
                            setActiveSection(section.id);
                          }}
                        >
                          {section.label} ›
                        </button>
                        {results
                          .filter((item) => item.section === section.id)
                          .map((item) => (
                            <SettingRow key={item.id} item={item} />
                          ))}
                      </section>
                    ))}
                  </>
                )
              ) : (
                <>
                  <div className="section-head">
                    <h2>{currentSection.label}</h2>
                    <p>{currentSection.blurb}</p>
                  </div>
                  <section className="settings-group">
                    {visibleItems
                      .filter((item) => item.section === activeSection)
                      .map((item) => (
                        <SettingRow key={item.id} item={item} />
                      ))}
                  </section>
                </>
              )}
            </div>
          </div>
        </section>
      )}
    </main>
  );
}

export default App;
