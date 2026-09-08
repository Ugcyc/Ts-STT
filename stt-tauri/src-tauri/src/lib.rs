mod features;
mod focus;
mod nemotron;
mod output;

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use arboard::Clipboard;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use directories::BaseDirs;
use enigo::{Enigo, Key, KeyboardControllable};
#[cfg(test)]
use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Listener, Manager, State};

use features::StreamingResampler;
use features::{transform_text, PerformanceReport, Replacement, TranscriptEntry};
use nemotron::{language_id, NemotronModel, NemotronSession};
use output::OutputWorker;
use sha2::{Digest, Sha256};

const MODEL_REPO: &str = "onnx-community/nemotron-3.5-asr-streaming-0.6b-onnx-int4";
const MODEL_FOLDER: &str = "nemotron-3.5-asr-streaming-0.6b-onnx-int4";
const GENAI_VERSION: &str = "0.14.0";
const ONNX_RUNTIME_VERSION: &str = "1.24.1";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const RUNTIME_PLATFORM: &str = "osx-arm64";
#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
const RUNTIME_PLATFORM: &str = "osx-x64";
#[cfg(not(target_os = "macos"))]
const RUNTIME_PLATFORM: &str = "win-x64";

#[cfg(target_os = "macos")]
const GENAI_ARCHIVE_EXT: &str = "tar.gz";
#[cfg(not(target_os = "macos"))]
const GENAI_ARCHIVE_EXT: &str = "zip";

#[cfg(target_os = "macos")]
const ORT_ARCHIVE_EXT: &str = "tgz";
#[cfg(not(target_os = "macos"))]
const ORT_ARCHIVE_EXT: &str = "zip";

#[cfg(target_os = "macos")]
const GENAI_LIB_NAME: &str = "libonnxruntime-genai.dylib";
#[cfg(not(target_os = "macos"))]
const GENAI_LIB_NAME: &str = "onnxruntime-genai.dll";
const MODEL_FILES: &[&str] = &[
    "audio_processor_config.json",
    "decoder.onnx",
    "decoder.onnx.data",
    "encoder.onnx",
    "encoder.onnx.data",
    "genai_config.json",
    "joint.onnx",
    "joint.onnx.data",
    "model_config.json",
    "silero_vad.onnx",
    "tokenizer.json",
    "tokenizer_config.json",
    "vocab.txt",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct AppConfig {
    hotkey: String,
    mode: String,
    theme: String,
    output_mode: String,
    mic_device: Option<String>,
    language: String,
    streaming: bool,
    live_typing: bool,
    live_update_ms: u32,
    key_delay_ms: u32,
    model_keep: String,
    spoken_punctuation: bool,
    silence_timeout_secs: f32,
    log_transcripts: bool,
    model_path: Option<String>,
    protect_destination: bool,
    stable_live_text: bool,
    live_preview: bool,
    vad_enabled: bool,
    auto_stop_enabled: bool,
    replacements_enabled: bool,
    replacements: Vec<Replacement>,
    history_enabled: bool,
    history_limit: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: "CommandOrControl+Shift+Space".to_string(),
            mode: "toggle".to_string(),
            theme: "dark".to_string(),
            output_mode: "paste".to_string(),
            mic_device: None,
            language: "en-US".to_string(),
            streaming: true,
            live_typing: false,
            live_update_ms: 560,
            key_delay_ms: 0,
            model_keep: "always".to_string(),
            spoken_punctuation: false,
            silence_timeout_secs: 60.0,
            log_transcripts: false,
            model_path: None,
            protect_destination: false,
            stable_live_text: false,
            live_preview: false,
            vad_enabled: false,
            auto_stop_enabled: false,
            replacements_enabled: false,
            replacements: Vec::new(),
            history_enabled: false,
            history_limit: 20,
        }
    }
}

#[derive(Debug, Serialize)]
struct InitialData {
    config: AppConfig,
    input_devices: Vec<String>,
    model_path: String,
    model_exists: bool,
    status: String,
    history: Vec<TranscriptEntry>,
    last_transcript: Option<TranscriptEntry>,
    protected_insertion_supported: bool,
}

#[derive(Debug, Clone, Serialize)]
struct TranscriptionResponse {
    raw_text: String,
    final_text: String,
    seconds: f32,
    warning: Option<String>,
}

#[derive(Debug, Serialize)]
struct ToggleResponse {
    is_recording: bool,
    status: String,
    transcript: Option<TranscriptionResponse>,
    note: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    stage: String,
    item: String,
    item_index: usize,
    item_count: usize,
    bytes_downloaded: u64,
    bytes_total: Option<u64>,
    message: String,
}

enum ModelCommand {
    Unload {
        response: mpsc::Sender<()>,
    },
    Start {
        model_path: PathBuf,
        runtime_dll: PathBuf,
        config: Box<AppConfig>,
        app: AppHandle,
        session_id: usize,
        destination: Result<Option<focus::Destination>, String>,
        response: mpsc::Sender<Result<(), String>>,
    },
    Audio {
        samples: Vec<f32>,
        sample_rate: u32,
    },
    Finish {
        response: mpsc::Sender<Result<WorkerResult, String>>,
    },
    Fault(String),
}

struct WorkerResult {
    raw: String,
    text: String,
    warning: Option<String>,
    report: PerformanceReport,
}

#[derive(Clone)]
struct ModelHandle {
    tx: mpsc::SyncSender<ModelCommand>,
    queued_samples: Arc<AtomicUsize>,
    overflow: Arc<AtomicBool>,
}
impl ModelHandle {
    fn unload(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(ModelCommand::Unload { response: tx })
            .map_err(|_| "Worker unavailable.")?;
        rx.recv().map_err(|_| "Worker unavailable.".into())
    }
    fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(128);
        let queued_samples = Arc::new(AtomicUsize::new(0));
        let overflow = Arc::new(AtomicBool::new(false));
        let q = queued_samples.clone();
        let o = overflow.clone();
        thread::spawn(move || model_worker(rx, q, o));
        Self {
            tx,
            queued_samples,
            overflow,
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn start(
        &self,
        model_path: PathBuf,
        runtime_dll: PathBuf,
        config: AppConfig,
        app: AppHandle,
        session_id: usize,
        destination: Result<Option<focus::Destination>, String>,
    ) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.overflow.store(false, Ordering::SeqCst);
        self.tx
            .send(ModelCommand::Start {
                model_path,
                runtime_dll,
                config: Box::new(config),
                app,
                session_id,
                destination,
                response: tx,
            })
            .map_err(|_| "Nemotron worker is unavailable.".to_string())?;
        rx.recv()
            .map_err(|_| "Nemotron worker did not respond.".to_string())?
    }
    fn push(&self, samples: Vec<f32>, sample_rate: u32) {
        let count = samples.len();
        // Bound both callback count and audio duration. Never block the audio callback.
        let queued = self.queued_samples.fetch_add(count, Ordering::SeqCst);
        if queued + count > sample_rate as usize * 10 || self.overflow.load(Ordering::SeqCst) {
            self.queued_samples.fetch_sub(count, Ordering::SeqCst);
            self.overflow.store(true, Ordering::SeqCst);
            return;
        }
        if self
            .tx
            .try_send(ModelCommand::Audio {
                samples,
                sample_rate,
            })
            .is_err()
        {
            self.queued_samples.fetch_sub(count, Ordering::SeqCst);
            self.overflow.store(true, Ordering::SeqCst);
        }
    }
    fn fault(&self, error: String) {
        if self.tx.try_send(ModelCommand::Fault(error)).is_err() {
            self.overflow.store(true, Ordering::SeqCst);
        }
    }
    fn finish(&self) -> Result<WorkerResult, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(ModelCommand::Finish { response: tx })
            .map_err(|_| "Nemotron worker is unavailable.".to_string())?;
        rx.recv()
            .map_err(|_| "Nemotron worker did not respond.".to_string())?
    }
}

fn model_worker(
    rx: mpsc::Receiver<ModelCommand>,
    queued: Arc<AtomicUsize>,
    overflow: Arc<AtomicBool>,
) {
    let mut cached: Option<(PathBuf, PathBuf, NemotronModel)> = None;
    let mut session: Option<NemotronSession> = None;
    let mut resampler = StreamingResampler::new();
    let mut config = AppConfig::default();
    let mut app: Option<AppHandle> = None;
    let mut output: Option<OutputWorker> = None;
    let mut keep_alive: Option<Duration> = None;
    let mut last_used = Instant::now();
    let mut started = Instant::now();
    let mut warning: Option<String> = None;
    let mut last_text = String::new();
    let mut last_raw = String::new();
    let mut report = PerformanceReport::default();
    let mut samples_since_update = 0;
    let mut stop_requested = false;
    let mut current_session = 0;
    loop {
        let command = if cached.is_some() && session.is_none() {
            if let Some(timeout) = keep_alive {
                match rx
                    .recv_timeout((last_used + timeout).saturating_duration_since(Instant::now()))
                {
                    Ok(c) => c,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        cached = None;
                        continue;
                    }
                    Err(_) => return,
                }
            } else {
                match rx.recv() {
                    Ok(c) => c,
                    Err(_) => return,
                }
            }
        } else {
            match rx.recv() {
                Ok(c) => c,
                Err(_) => return,
            }
        };
        match command {
            ModelCommand::Unload { response } => {
                session = None;
                output = None;
                cached = None;
                let _ = response.send(());
            }
            ModelCommand::Start {
                model_path,
                runtime_dll,
                config: new_config,
                app: handle,
                session_id,
                destination,
                response,
            } => {
                session = None;
                output = None;
                config = *new_config;
                app = Some(handle.clone());
                current_session = session_id;
                resampler = StreamingResampler::new();
                warning = None;
                last_text.clear();
                last_raw.clear();
                samples_since_update = 0;
                stop_requested = false;
                report = PerformanceReport::default();
                started = Instant::now();
                keep_alive = match config.model_keep.as_str() {
                    "after_use" => Some(Duration::ZERO),
                    "5min" => Some(Duration::from_secs(300)),
                    "15min" => Some(Duration::from_secs(900)),
                    _ => None,
                };
                // Start capture immediately while the cold model loads in this worker.
                let _ = response.send(Ok(()));
                let result = (|| {
                    if cached
                        .as_ref()
                        .is_none_or(|(m, r, _)| m != &model_path || r != &runtime_dll)
                    {
                        cached = None;
                        cached = Some((
                            model_path.clone(),
                            runtime_dll.clone(),
                            NemotronModel::load(&runtime_dll, &model_path)?,
                        ));
                    }
                    cached
                        .as_ref()
                        .unwrap()
                        .2
                        .start_session(&config.language, config.vad_enabled)
                })();
                report.load_ms = started.elapsed().as_millis() as u64;
                match result {
                    Ok(active) => {
                        session = Some(active);
                        if config.live_typing && config.output_mode == "type" && config.streaming {
                            output = Some(OutputWorker::new(
                                config.key_delay_ms,
                                config.stable_live_text,
                                destination,
                                Some(handle.clone()),
                            ));
                        }
                    }
                    Err(e) => {
                        warning = Some(e.clone());
                        let _ = handle.emit("dictation-warning", e);
                        let _ = handle.emit("dictation-auto-stop", current_session);
                        stop_requested = true;
                    }
                }
                last_used = Instant::now();
            }
            ModelCommand::Audio {
                samples,
                sample_rate,
            } => {
                queued.fetch_sub(samples.len(), Ordering::SeqCst);
                let queue_ms =
                    queued.load(Ordering::SeqCst) as u64 * 1000 / sample_rate.max(1) as u64;
                report.max_queue_ms = report.max_queue_ms.max(queue_ms);
                let audio = resampler.push(&samples, sample_rate);
                report.audio_seconds += audio.len() as f64 / 16_000.0;
                if let Some(active) = session.as_mut() {
                    if warning.is_none() {
                        let t = Instant::now();
                        match active.push(&audio) {
                            Ok(raw) => {
                                last_raw = raw;
                                last_text = format_transcript(&config, &last_raw);
                                if report.first_text_ms.is_none() && !last_text.is_empty() {
                                    report.first_text_ms =
                                        Some(started.elapsed().as_millis() as u64);
                                }
                                samples_since_update += audio.len();
                                if samples_since_update >= config.live_update_ms as usize * 16 {
                                    samples_since_update = 0;
                                    if let Some(writer) = &output {
                                        writer.update(last_text.clone());
                                    }
                                    if config.live_preview {
                                        if let Some(handle) = &app {
                                            let _ = handle.emit("dictation-preview", &last_text);
                                        }
                                    }
                                }
                                if config.auto_stop_enabled
                                    && !last_text.is_empty()
                                    && active.silent_seconds() >= config.silence_timeout_secs
                                    && !stop_requested
                                {
                                    stop_requested = true;
                                    if let Some(handle) = &app {
                                        let _ = handle.emit("dictation-auto-stop", current_session);
                                    }
                                }
                            }
                            Err(e) => {
                                warning = Some(e.clone());
                                if let Some(handle) = &app {
                                    let _ = handle.emit("dictation-warning", e);
                                    if !stop_requested {
                                        let _ = handle.emit("dictation-auto-stop", current_session);
                                        stop_requested = true;
                                    }
                                }
                            }
                        }
                        report.inference_ms += t.elapsed().as_millis() as u64;
                    }
                }
                if overflow.load(Ordering::SeqCst) && !stop_requested {
                    report.overflowed = true;
                    warning = Some("Audio processing fell behind. Recording stopped before further audio could be lost; this transcript may be incomplete.".into());
                    if let Some(handle) = &app {
                        let _ = handle.emit("dictation-warning", warning.clone());
                        let _ = handle.emit("dictation-auto-stop", current_session);
                    }
                    stop_requested = true;
                }
            }
            ModelCommand::Fault(error) => {
                warning = Some(error.clone());
                if let Some(handle) = &app {
                    let _ = handle.emit("dictation-warning", error);
                    if !stop_requested {
                        let _ = handle.emit("dictation-auto-stop", current_session);
                        stop_requested = true;
                    }
                }
            }
            ModelCommand::Finish { response } => {
                let t = Instant::now();
                if let Some(active) = session.as_mut() {
                    match active.finish() {
                        Ok(raw) => {
                            last_raw = raw;
                            last_text = format_transcript(&config, &last_raw);
                        }
                        Err(e) => warning = Some(e),
                    }
                }
                report.finish_ms = t.elapsed().as_millis() as u64;
                report.inference_ms += report.finish_ms;
                report.overflowed |= overflow.load(Ordering::SeqCst);
                if let Some(writer) = output.take() {
                    if let Err(e) = writer.finish(last_text.clone()) {
                        warning = Some(e);
                    }
                }
                let _ = response.send(Ok(WorkerResult {
                    raw: last_raw.clone(),
                    text: last_text.clone(),
                    warning: warning.take(),
                    report: report.clone(),
                }));
                session = None;
                last_used = Instant::now();
            }
        }
    }
}

fn format_transcript(config: &AppConfig, raw: &str) -> String {
    transform_text(
        raw,
        config.spoken_punctuation,
        if config.replacements_enabled {
            &config.replacements
        } else {
            &[]
        },
    )
}

enum RecorderCommand {
    Start {
        device_name: Option<String>,
        model: ModelHandle,
        response: mpsc::Sender<Result<(), String>>,
    },
    Stop {
        response: mpsc::Sender<Result<f32, String>>,
    },
    IsRecording {
        response: mpsc::Sender<bool>,
    },
}

#[derive(Clone)]
struct RecorderHandle {
    tx: mpsc::Sender<RecorderCommand>,
}

impl RecorderHandle {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || recorder_worker(rx));
        Self { tx }
    }

    fn start(&self, device_name: Option<String>, model: ModelHandle) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(RecorderCommand::Start {
                device_name,
                model,
                response: tx,
            })
            .map_err(|_| "Recorder worker is unavailable.".to_string())?;
        rx.recv()
            .map_err(|_| "Recorder worker did not respond.".to_string())?
    }

    fn stop(&self) -> Result<f32, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(RecorderCommand::Stop { response: tx })
            .map_err(|_| "Recorder worker is unavailable.".to_string())?;
        rx.recv()
            .map_err(|_| "Recorder worker did not respond.".to_string())?
    }

    fn is_recording(&self) -> Result<bool, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(RecorderCommand::IsRecording { response: tx })
            .map_err(|_| "Recorder worker is unavailable.".to_string())?;
        rx.recv()
            .map_err(|_| "Recorder worker did not respond.".to_string())
    }
}

fn mono_f32(data: &[f32], channels: usize) -> Vec<f32> {
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn mono_i16(data: &[i16], channels: usize) -> Vec<f32> {
    data.chunks(channels)
        .map(|frame| {
            frame
                .iter()
                .map(|sample| *sample as f32 / 32768.0)
                .sum::<f32>()
                / channels as f32
        })
        .collect()
}

fn mono_u16(data: &[u16], channels: usize) -> Vec<f32> {
    data.chunks(channels)
        .map(|frame| {
            frame
                .iter()
                .map(|sample| (*sample as f32 / 65535.0) * 2.0 - 1.0)
                .sum::<f32>()
                / channels as f32
        })
        .collect()
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    model: ModelHandle,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let rate = config.sample_rate.0;
    let error_model = model.clone();
    let err = move |error| error_model.fault(format!("Microphone disconnected or failed: {error}"));
    match format {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _| model.push(mono_f32(data, channels), rate),
                err,
                None,
            )
            .map_err(|e| format!("Failed to open microphone: {e}")),
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _| model.push(mono_i16(data, channels), rate),
                err,
                None,
            )
            .map_err(|e| format!("Failed to open microphone: {e}")),
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                config,
                move |data: &[u16], _| model.push(mono_u16(data, channels), rate),
                err,
                None,
            )
            .map_err(|e| format!("Failed to open microphone: {e}")),
        other => Err(format!("Unsupported microphone format: {other:?}")),
    }
}

fn recorder_worker(rx: mpsc::Receiver<RecorderCommand>) {
    let mut active: Option<(cpal::Stream, Instant)> = None;
    while let Ok(command) = rx.recv() {
        match command {
            RecorderCommand::Start {
                device_name,
                model,
                response,
            } => {
                let result = (|| {
                    if active.is_some() {
                        return Err("Already recording.".to_string());
                    }
                    let device = resolve_input_device(device_name)?;
                    let supported = device
                        .default_input_config()
                        .map_err(|e| format!("Could not read microphone format: {e}"))?;
                    let stream = build_input_stream(
                        &device,
                        &supported.config(),
                        supported.sample_format(),
                        model,
                    )?;
                    stream
                        .play()
                        .map_err(|e| format!("Failed to start microphone: {e}"))?;
                    active = Some((stream, Instant::now()));
                    Ok(())
                })();
                let _ = response.send(result);
            }
            RecorderCommand::Stop { response } => {
                let result = active
                    .take()
                    .map(|(stream, started)| {
                        drop(stream);
                        started.elapsed().as_secs_f32()
                    })
                    .ok_or_else(|| "Not currently recording.".to_string());
                let _ = response.send(result);
            }
            RecorderCommand::IsRecording { response } => {
                let _ = response.send(active.is_some());
            }
        }
    }
}

struct AppState {
    recorder: RecorderHandle,
    model: ModelHandle,
    config: Mutex<AppConfig>,
    status: Mutex<String>,
    config_path: PathBuf,
    model_dir: PathBuf,
    runtime_dir: PathBuf,
    operation: Mutex<()>,
    session_id: AtomicUsize,
    active_config: Mutex<Option<AppConfig>>,
    destination: Mutex<Result<Option<focus::Destination>, String>>,
    last_transcript: Mutex<Option<TranscriptEntry>>,
    history: Mutex<Vec<TranscriptEntry>>,
    report: Mutex<PerformanceReport>,
    download_lock: Mutex<()>,
}

impl AppState {
    fn new() -> Result<Self, String> {
        let app_dir = app_data_dir()?;
        let config_path = app_dir.join("config.json");
        let model_dir = app_dir.join("models");
        let runtime_dir = app_dir.join("runtime");
        fs::create_dir_all(&model_dir)
            .map_err(|e| format!("Failed to create model folder: {e}"))?;
        fs::create_dir_all(&runtime_dir)
            .map_err(|e| format!("Failed to create runtime folder: {e}"))?;
        let config = normalize_config(load_config_from_disk(&config_path)?);
        save_config_to_disk(&config_path, &config)?;
        let history_path = app_dir.join("history.json");
        let history: Vec<TranscriptEntry> = if config.history_enabled && history_path.exists() {
            let bytes = fs::read(&history_path)
                .map_err(|e| format!("Cannot read transcript history: {e}"))?;
            serde_json::from_slice(&bytes)
                .map_err(|e| format!("Cannot parse transcript history: {e}"))?
        } else {
            Vec::new()
        };
        Ok(Self {
            session_id: AtomicUsize::new(0),
            operation: Mutex::new(()),
            active_config: Mutex::new(None),
            destination: Mutex::new(Ok(None)),
            last_transcript: Mutex::new(None),
            history: Mutex::new(history.into_iter().take(config.history_limit).collect()),
            report: Mutex::new(PerformanceReport::default()),
            download_lock: Mutex::new(()),
            recorder: RecorderHandle::new(),
            model: ModelHandle::new(),
            config: Mutex::new(config),
            status: Mutex::new("Idle".to_string()),
            config_path,
            model_dir,
            runtime_dir,
        })
    }
}

fn app_data_dir() -> Result<PathBuf, String> {
    let base =
        BaseDirs::new().ok_or_else(|| "Could not find local app data folder.".to_string())?;
    let dir = base.data_local_dir().join("Ts-STT");
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create app data folder: {e}"))?;
    Ok(dir)
}

fn load_config_from_disk(path: &Path) -> Result<AppConfig, String> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let data = fs::read_to_string(path).map_err(|e| format!("Failed to read config: {e}"))?;
    serde_json::from_str(&data).map_err(|e| format!("Failed to parse config: {e}"))
}

fn save_config_to_disk(path: &Path, config: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create config folder: {e}"))?;
    }
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to encode config: {e}"))?;
    features::atomic_write(path, data.as_bytes()).map_err(|e| format!("Failed to save config: {e}"))
}

fn normalize_config(mut config: AppConfig) -> AppConfig {
    config.mode = "toggle".to_string();
    if !config.silence_timeout_secs.is_finite() {
        config.silence_timeout_secs = 60.0;
    }
    config.silence_timeout_secs = config.silence_timeout_secs.clamp(2.0, 600.0);
    config.history_limit = config.history_limit.clamp(1, 100);
    config.replacements.truncate(200);
    for rule in &mut config.replacements {
        rule.heard = rule.heard.trim().chars().take(200).collect();
        rule.replacement = rule.replacement.chars().take(2000).collect();
    }
    config.replacements.retain(|r| !r.heard.is_empty());
    if !config.vad_enabled {
        config.auto_stop_enabled = false;
    }
    if !config.streaming || config.output_mode != "type" {
        config.live_typing = false;
    }
    config.theme = if config.theme.eq_ignore_ascii_case("light") {
        "light".to_string()
    } else {
        "dark".to_string()
    };
    config.output_mode = match config.output_mode.to_lowercase().as_str() {
        "clipboard" => "clipboard".to_string(),
        "type" => "type".to_string(),
        _ => "paste".to_string(),
    };
    if config.language.eq_ignore_ascii_case("en") || language_id(&config.language).is_none() {
        config.language = "en-US".to_string();
    }
    if !matches!(config.live_update_ms, 560 | 1120 | 1680) {
        config.live_update_ms = 560;
    }
    if !matches!(config.key_delay_ms, 0 | 8 | 16 | 32) {
        config.key_delay_ms = 0;
    }
    if !matches!(
        config.model_keep.as_str(),
        "always" | "5min" | "15min" | "after_use"
    ) {
        config.model_keep = "always".to_string();
    }
    if config
        .model_path
        .as_ref()
        .is_some_and(|path| path.to_lowercase().ends_with(".bin"))
    {
        config.model_path = None;
    }
    config
}

fn resolve_model_path(config: &AppConfig, model_dir: &Path) -> PathBuf {
    config
        .model_path
        .as_ref()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| model_dir.join(MODEL_FOLDER))
}

fn runtime_dll(runtime_dir: &Path) -> PathBuf {
    runtime_dir
        .join(format!(
            "onnxruntime-genai-{GENAI_VERSION}-{RUNTIME_PLATFORM}"
        ))
        .join("lib")
        .join(GENAI_LIB_NAME)
}

#[cfg(target_os = "macos")]
fn base_lib_name() -> String {
    format!("libonnxruntime.{ONNX_RUNTIME_VERSION}.dylib")
}

#[cfg(not(target_os = "macos"))]
fn base_lib_name() -> String {
    "onnxruntime.dll".to_string()
}

fn base_runtime_dll(runtime_dir: &Path) -> PathBuf {
    runtime_dll(runtime_dir)
        .parent()
        .expect("runtime DLL has a parent")
        .join(base_lib_name())
}

fn installation_exists(model_path: &Path, runtime_path: &Path) -> bool {
    runtime_path.is_file()
        && runtime_path
            .parent()
            .is_some_and(|parent| parent.join(base_lib_name()).is_file())
        && MODEL_FILES
            .iter()
            .all(|file| model_path.join(file).is_file())
}

#[cfg(not(target_os = "macos"))]
fn install_base_runtime(source_lib: &Path, destination_lib: &Path) -> Result<(), String> {
    for file_name in ["onnxruntime.dll", "onnxruntime_providers_shared.dll"] {
        fs::copy(source_lib.join(file_name), destination_lib.join(file_name))
            .map_err(|e| format!("Failed to install {file_name}: {e}"))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_base_runtime(source_lib: &Path, destination_lib: &Path) -> Result<(), String> {
    // Copy every libonnxruntime* dylib (including the versioned one the GenAI
    // dylib links against via @rpath/@loader_path) beside the GenAI library.
    let entries = fs::read_dir(source_lib)
        .map_err(|e| format!("Failed to read ONNX Runtime lib folder: {e}"))?;
    let mut copied = 0usize;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with("libonnxruntime") && name.ends_with(".dylib") {
            fs::copy(entry.path(), destination_lib.join(&file_name))
                .map_err(|e| format!("Failed to install {name}: {e}"))?;
            copied += 1;
        }
    }
    if copied == 0 {
        return Err("No libonnxruntime dylibs found in the downloaded archive.".to_string());
    }
    Ok(())
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| format!("Failed to open archive: {e}"))?;
    let name = archive.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Invalid archive: {e}"))?;
        zip.extract(destination)
            .map_err(|e| format!("Failed to extract archive: {e}"))
    } else {
        let tar = flate2::read::GzDecoder::new(file);
        tar::Archive::new(tar)
            .unpack(destination)
            .map_err(|e| format!("Failed to extract archive: {e}"))
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_download_progress(
    app: &AppHandle,
    stage: &str,
    item: &str,
    item_index: usize,
    item_count: usize,
    bytes_downloaded: u64,
    bytes_total: Option<u64>,
    message: impl Into<String>,
) {
    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            stage: stage.to_string(),
            item: item.to_string(),
            item_index,
            item_count,
            bytes_downloaded,
            bytes_total,
            message: message.into(),
        },
    );
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn file_matches(path: &Path, expected: &str) -> bool {
    file_hash(path).is_ok_and(|actual| actual == expected)
}
fn archive_checksum(name: &str) -> Result<&'static str, String> {
    match name {
        "onnxruntime-genai-0.14.0-win-x64.zip" => Ok("8a303e52dc7be8fb2a5331929af451a25ac59774102d7fd09ef673adc85c5ebf"),
        "onnxruntime-genai-0.14.0-osx-arm64.tar.gz" => Ok("56583c98e3939d2cfd5a3812471be44017ce2752776d389015ff583a8d758312"),
        "onnxruntime-win-x64-1.24.1.zip" => Ok("c72736c16442cad417ee4f45e3e2fbd10778e70a0fc1cb0e86f028d766e525af"),
        "onnxruntime-osx-arm64-1.24.1.tgz" => Ok("c2969315cd9ce0f5fa04f6b53ff72cb92f87f7dcf38e88cacfa40c8f983fbba9"),
        _ => Err(format!("No verified runtime archive is available for {name}. This release supports Windows x64 and Apple Silicon.")),
    }
}
fn runtime_receipt_valid(runtime_dir: &Path) -> bool {
    let Ok(data) = fs::read(runtime_dir.join("verified-runtime.json")) else {
        return false;
    };
    let Ok(entries) = serde_json::from_slice::<std::collections::BTreeMap<String, String>>(&data)
    else {
        return false;
    };
    let Some(lib) = runtime_dll(runtime_dir).parent().map(Path::to_path_buf) else {
        return false;
    };
    entries.contains_key(GENAI_LIB_NAME)
        && entries.contains_key(&base_lib_name())
        && entries
            .iter()
            .all(|(name, hash)| file_matches(&lib.join(name), hash))
}
fn save_runtime_receipt(runtime_dir: &Path) -> Result<(), String> {
    let dll = runtime_dll(runtime_dir);
    let lib = dll.parent().ok_or("Missing runtime directory")?;
    let mut entries = std::collections::BTreeMap::new();
    for entry in fs::read_dir(lib).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".dll") || name.ends_with(".dylib") {
            entries.insert(name, file_hash(&entry.path())?);
        }
    }
    features::atomic_write(
        &runtime_dir.join("verified-runtime.json"),
        &serde_json::to_vec(&entries).map_err(|e| e.to_string())?,
    )
}

#[allow(clippy::too_many_arguments)]
fn download_file(
    app: &AppHandle,
    url: &str,
    path: &Path,
    item: &str,
    item_index: usize,
    item_count: usize,
    expected_hash: &str,
) -> Result<(), String> {
    if file_matches(path, expected_hash) {
        emit_download_progress(
            app,
            "skipped",
            item,
            item_index,
            item_count,
            0,
            None,
            format!("{item} already downloaded"),
        );
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create download folder: {e}"))?;
    }
    let temporary = path.with_extension("part");
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(1800))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("Download failed for {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download failed for {url}: {e}"))?;
    let bytes_total = response.content_length();
    let mut file = File::create(&temporary)
        .map_err(|e| format!("Failed to create {}: {e}", temporary.display()))?;
    let mut bytes_downloaded = 0u64;
    let mut last_emitted = 0u64;
    let mut buffer = [0u8; 128 * 1024];
    emit_download_progress(
        app,
        "downloading",
        item,
        item_index,
        item_count,
        bytes_downloaded,
        bytes_total,
        format!("Downloading {item}"),
    );
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("Failed while downloading {item}: {e}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("Failed to save {}: {e}", temporary.display()))?;
        bytes_downloaded += read as u64;
        if bytes_downloaded.saturating_sub(last_emitted) >= 1024 * 1024
            || bytes_total.is_some_and(|total| bytes_downloaded >= total)
        {
            emit_download_progress(
                app,
                "downloading",
                item,
                item_index,
                item_count,
                bytes_downloaded,
                bytes_total,
                format!("Downloading {item}"),
            );
            last_emitted = bytes_downloaded;
        }
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    if bytes_total.is_some_and(|total| total != bytes_downloaded)
        || !file_matches(&temporary, expected_hash)
    {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "Download verification failed for {item}. Retry Repair Nemotron."
        ));
    }
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    fs::rename(&temporary, path).map_err(|e| format!("Failed to finish {}: {e}", path.display()))
}

fn download_installation(
    app: &AppHandle,
    model_path: &Path,
    runtime_dir: &Path,
) -> Result<(), String> {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../model-manifest.json")).map_err(|e| e.to_string())?;
    let revision = manifest["revision"]
        .as_str()
        .ok_or("Missing model revision")?;
    let item_count = MODEL_FILES.len() + 2;
    fs::create_dir_all(model_path).map_err(|e| format!("Failed to create model folder: {e}"))?;
    for (index, file) in MODEL_FILES.iter().enumerate() {
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/{revision}/{file}");
        download_file(
            app,
            &url,
            &model_path.join(file),
            file,
            index + 1,
            item_count,
            manifest["files"][file]
                .as_str()
                .ok_or("Missing model checksum")?,
        )?;
    }

    let dll = runtime_dll(runtime_dir);
    if !runtime_receipt_valid(runtime_dir) {
        let runtime_item = format!("ONNX Runtime GenAI {GENAI_VERSION}");
        let archive_name =
            format!("onnxruntime-genai-{GENAI_VERSION}-{RUNTIME_PLATFORM}.{GENAI_ARCHIVE_EXT}");
        let archive = runtime_dir.join(&archive_name);
        let url = format!(
            "https://github.com/microsoft/onnxruntime-genai/releases/download/v{GENAI_VERSION}/{archive_name}"
        );
        download_file(
            app,
            &url,
            &archive,
            &runtime_item,
            item_count - 1,
            item_count,
            archive_checksum(&archive_name)?,
        )?;
        emit_download_progress(
            app,
            "extracting",
            &runtime_item,
            item_count - 1,
            item_count,
            0,
            None,
            "Extracting native ONNX runtime",
        );
        extract_archive(&archive, runtime_dir)
            .map_err(|e| format!("Failed to extract runtime: {e}"))?;
        fs::remove_file(archive).map_err(|e| format!("Failed to remove runtime archive: {e}"))?;
    }

    let base_dll = base_runtime_dll(runtime_dir);
    if !runtime_receipt_valid(runtime_dir) || !base_dll.exists() {
        let runtime_item = format!("ONNX Runtime {ONNX_RUNTIME_VERSION}");
        let archive_name =
            format!("onnxruntime-{RUNTIME_PLATFORM}-{ONNX_RUNTIME_VERSION}.{ORT_ARCHIVE_EXT}");
        let archive = runtime_dir.join(&archive_name);
        let url = format!(
            "https://github.com/microsoft/onnxruntime/releases/download/v{ONNX_RUNTIME_VERSION}/{archive_name}"
        );
        download_file(
            app,
            &url,
            &archive,
            &runtime_item,
            item_count,
            item_count,
            archive_checksum(&archive_name)?,
        )?;
        emit_download_progress(
            app,
            "extracting",
            &runtime_item,
            item_count,
            item_count,
            0,
            None,
            "Installing matching private ONNX Runtime",
        );
        extract_archive(&archive, runtime_dir)
            .map_err(|e| format!("Failed to extract ONNX Runtime: {e}"))?;
        let source_lib = runtime_dir
            .join(format!(
                "onnxruntime-{RUNTIME_PLATFORM}-{ONNX_RUNTIME_VERSION}"
            ))
            .join("lib");
        let destination_lib = dll
            .parent()
            .ok_or_else(|| "ONNX Runtime GenAI library folder is missing.".to_string())?;
        install_base_runtime(&source_lib, destination_lib)?;
        fs::remove_file(archive)
            .map_err(|e| format!("Failed to remove ONNX Runtime archive: {e}"))?;
    }
    save_runtime_receipt(runtime_dir)?;
    emit_download_progress(
        app,
        "complete",
        "Nemotron",
        item_count,
        item_count,
        0,
        None,
        "Nemotron is ready",
    );
    Ok(())
}

fn list_microphones() -> Result<Vec<String>, String> {
    let devices = cpal::default_host()
        .input_devices()
        .map_err(|e| format!("Failed to query microphones: {e}"))?;
    let mut names = Vec::new();
    for (index, device) in devices.enumerate() {
        names.push(format!(
            "{index}: {}",
            device
                .name()
                .unwrap_or_else(|_| "Unknown Device".to_string())
        ));
    }
    Ok(names)
}

fn resolve_input_device(selection: Option<String>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Some(selected) = selection.filter(|value| !value.eq_ignore_ascii_case("default")) {
        let name = selected
            .split_once(':')
            .map(|(_, name)| name.trim())
            .unwrap_or(&selected);
        let mut matches = host
            .input_devices()
            .map_err(|e| e.to_string())?
            .filter(|device| device.name().is_ok_and(|n| n == name));
        if let Some(device) = matches.next() {
            if matches.next().is_some() {
                return Err("Several microphones have this name. Choose Default and select the desired microphone in your system settings.".into());
            }
            return Ok(device);
        }
        return Err(format!(
            "Microphone '{name}' is unavailable. Reconnect it or choose another input."
        ));
    }
    host.default_input_device()
        .ok_or_else(|| "No microphone found.".into())
}

fn set_status(state: &AppState, status: &str) {
    if let Ok(mut value) = state.status.lock() {
        *value = status.to_string();
    }
}

fn status(state: &AppState) -> String {
    state
        .status
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| "Idle".to_string())
}

#[cfg(test)]
fn clean_language_tags(text: &str) -> String {
    let tags = Regex::new(r"\s*<[a-z]{2}(?:-[A-Z]{2})?>\s*").expect("valid regex");
    tags.replace_all(text, " ").trim().to_string()
}

fn set_clipboard(text: &str) -> Result<(), String> {
    Clipboard::new()
        .map_err(|e| format!("Clipboard unavailable: {e}"))?
        .set_text(text.to_string())
        .map_err(|e| format!("Failed to copy transcript: {e}"))
}

#[cfg(target_os = "macos")]
fn paste_shortcut() -> Result<(), String> {
    // Give the pasteboard a moment to update before sending Cmd+V.
    thread::sleep(Duration::from_millis(35));
    let first = std::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "tell application \"System Events\" to keystroke \"v\" using command down",
        ])
        .output()
        .map_err(|e| format!("Failed to run macOS paste shortcut: {e}"))?;
    if first.status.success() {
        return Ok(());
    }

    // Fallback for some keyboard layouts/input methods in release builds.
    let second = std::process::Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "tell application \"System Events\" to key code 9 using command down",
        ])
        .output()
        .map_err(|e| format!("Failed to run macOS paste fallback: {e}"))?;
    if second.status.success() {
        return Ok(());
    }

    let first_err = String::from_utf8_lossy(&first.stderr).trim().to_string();
    let second_err = String::from_utf8_lossy(&second.stderr).trim().to_string();
    Err(format!(
        "macOS paste failed. script1_status={} script1_err='{}' script2_status={} script2_err='{}'",
        first.status, first_err, second.status, second_err
    ))
}

#[cfg(not(target_os = "macos"))]
fn paste_shortcut() -> Result<(), String> {
    let mut enigo = Enigo::new();
    enigo.key_down(Key::Control);
    enigo.key_click(Key::Layout('v'));
    enigo.key_up(Key::Control);
    Ok(())
}

fn output_text(
    config: &AppConfig,
    text: &str,
    destination: Result<Option<focus::Destination>, String>,
) -> Result<(), String> {
    if config.output_mode == "clipboard" {
        return set_clipboard(text);
    }
    let target = destination?;
    if target.as_ref().is_some_and(|d| !d.unchanged()) {
        return Err("Insertion paused because the destination changed. Your transcript is available to copy.".into());
    }
    if config.output_mode == "type" {
        OutputWorker::new(config.key_delay_ms, false, Ok(target), None).finish(text.to_string())
    } else {
        set_clipboard(text)?;
        if target.as_ref().is_some_and(|d| !d.unchanged()) {
            return Err("Destination changed before paste. Transcript copied to clipboard.".into());
        }
        paste_shortcut()
    }
}

fn start_inner(
    app: &AppHandle,
    state: &AppState,
    device_name: Option<String>,
) -> Result<(), String> {
    if state.recorder.is_recording()? {
        return Err("Already recording.".into());
    }
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.")?
        .clone();
    let model_path = resolve_model_path(&config, &state.model_dir);
    let runtime = runtime_dll(&state.runtime_dir);
    if !installation_exists(&model_path, &runtime) {
        return Err("Nemotron is not installed. Open Settings and click Download Nemotron.".into());
    }
    focus::configure_hotkey(&config.hotkey);
    let destination = focus::Destination::capture(
        config.protect_destination && config.output_mode != "clipboard",
    );
    if let Err(message) = &destination {
        let _ = app.emit("dictation-warning", message);
    }
    *state.destination.lock().unwrap() = destination.clone();
    *state.active_config.lock().unwrap() = Some(config.clone());
    set_status(state, "Loading Nemotron");
    let session_id = state.session_id.fetch_add(1, Ordering::SeqCst) + 1;
    state.model.start(
        model_path,
        runtime,
        config.clone(),
        app.clone(),
        session_id,
        destination,
    )?;
    if let Err(error) = state
        .recorder
        .start(device_name.or(config.mic_device), state.model.clone())
    {
        let _ = state.model.finish();
        *state.active_config.lock().unwrap() = None;
        set_status(state, "Idle");
        return Err(error);
    }
    set_status(state, "Listening");
    Ok(())
}

fn stop_inner(state: &AppState) -> Result<TranscriptionResponse, String> {
    state.session_id.fetch_add(1, Ordering::SeqCst);
    let seconds = state.recorder.stop()?;
    set_status(state, "Finishing");
    let config = state
        .active_config
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| state.config.lock().unwrap().clone());
    let result = state.model.finish()?;
    *state.report.lock().unwrap() = result.report;
    let mut warning = result.warning;
    let text = result.text;
    // Retain successful recognition before any output side effect can fail.
    if !text.is_empty() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let entry = TranscriptEntry {
            id: now.as_nanos().to_string(),
            created_at: now.as_secs(),
            text: text.clone(),
            seconds,
        };
        *state.last_transcript.lock().unwrap() = Some(entry.clone());
        if config.history_enabled {
            let mut history = state.history.lock().unwrap();
            history.insert(0, entry);
            history.truncate(config.history_limit);
            if let Err(e) =
                features::save_history(&state.config_path.with_file_name("history.json"), &history)
            {
                warning = Some(format!(
                    "Transcript retained in memory, but history could not be saved: {e}"
                ));
            }
        }
        // Never automatically insert a transcript known to be incomplete.
        if warning.is_none() && !(config.live_typing && config.output_mode == "type") {
            if let Err(e) = output_text(&config, &text, state.destination.lock().unwrap().clone()) {
                warning = Some(e);
            }
        }
    }
    Ok(TranscriptionResponse {
        raw_text: result.raw,
        final_text: text,
        seconds,
        warning,
    })
}

#[tauri::command]
fn get_initial_data(state: State<'_, AppState>) -> Result<InitialData, String> {
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let model_path = resolve_model_path(&config, &state.model_dir);
    Ok(InitialData {
        config,
        input_devices: list_microphones()?,
        model_exists: installation_exists(&model_path, &runtime_dll(&state.runtime_dir)),
        model_path: model_path.display().to_string(),
        status: status(&state),
        history: state.history.lock().unwrap().clone(),
        last_transcript: state.last_transcript.lock().unwrap().clone(),
        protected_insertion_supported: focus::supported(),
    })
}

#[tauri::command]
fn load_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    state
        .config
        .lock()
        .map(|value| value.clone())
        .map_err(|_| "Config lock poisoned.".to_string())
}

#[tauri::command]
fn save_config(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Operation lock poisoned.")?;
    if state.recorder.is_recording()? {
        return Err("Stop dictation before saving settings.".into());
    }
    let config = normalize_config(config);
    let mut history = state.history.lock().unwrap();
    if !config.history_enabled {
        let path = state.config_path.with_file_name("history.json");
        if path.exists() {
            fs::remove_file(path).map_err(|e| format!("Cannot clear history: {e}"))?;
        }
        history.clear();
    } else {
        history.truncate(config.history_limit);
        features::save_history(&state.config_path.with_file_name("history.json"), &history)?;
    }
    save_config_to_disk(&state.config_path, &config)?;
    *state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())? = config.clone();
    Ok(config)
}

#[tauri::command]
fn list_input_devices() -> Result<Vec<String>, String> {
    list_microphones()
}

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> Result<String, String> {
    Ok(status(&state))
}

#[tauri::command]
async fn start_recording(app: AppHandle, device_name: Option<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        start_inner(&app, &state, device_name)?;
        Ok(status(&state))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn stop_recording_and_process(app: AppHandle) -> Result<TranscriptionResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        let result = stop_inner(&state);
        set_status(&state, "Idle");
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn toggle_recording_and_process(app: AppHandle) -> Result<ToggleResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        if !state.recorder.is_recording()? {
            start_inner(&app, &state, None)?;
            return Ok(ToggleResponse {
                is_recording: true,
                status: status(&state),
                transcript: None,
                note: None,
            });
        }
        let result = stop_inner(&state);
        set_status(&state, "Idle");
        let transcript = result?;
        let note = transcript
            .final_text
            .is_empty()
            .then(|| "No speech detected.".into());
        Ok(ToggleResponse {
            is_recording: false,
            status: status(&state),
            transcript: Some(transcript),
            note,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_transcript_history(state: State<'_, AppState>) -> Vec<TranscriptEntry> {
    state.history.lock().unwrap().clone()
}

fn transcript_for(state: &AppState, id: Option<String>) -> Result<String, String> {
    if let Some(id) = id {
        state
            .history
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.text.clone())
            .ok_or_else(|| "Transcript no longer exists.".into())
    } else {
        state
            .last_transcript
            .lock()
            .unwrap()
            .as_ref()
            .map(|e| e.text.clone())
            .ok_or_else(|| "No transcript available.".into())
    }
}

#[tauri::command]
fn copy_transcript(state: State<'_, AppState>, id: Option<String>) -> Result<(), String> {
    set_clipboard(&transcript_for(&state, id)?)
}

#[tauri::command]
async fn reinsert_transcript(app: AppHandle, id: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        if state.recorder.is_recording()? {
            return Err("Stop dictation before reinserting.".into());
        }
        let text = transcript_for(&state, id)?;
        let mut config = state.config.lock().unwrap().clone();
        config.output_mode = "paste".into();
        // Explicit countdown lets the user choose the destination before capturing it.
        thread::sleep(Duration::from_secs(3));
        let destination = focus::Destination::capture(config.protect_destination);
        output_text(&config, &text, destination)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn clear_transcript_history(state: State<'_, AppState>) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another operation is in progress.")?;
    let path = state.config_path.with_file_name("history.json");
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    state.history.lock().unwrap().clear();
    *state.last_transcript.lock().unwrap() = None;
    Ok(())
}

#[tauri::command]
fn model_exists(state: State<'_, AppState>) -> Result<bool, String> {
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    Ok(installation_exists(
        &resolve_model_path(&config, &state.model_dir),
        &runtime_dll(&state.runtime_dir),
    ))
}

#[tauri::command]
async fn download_model(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let model_path = resolve_model_path(&config, &state.model_dir);
    let runtime_dir = state.runtime_dir.clone();
    let output = model_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _download = state
            .download_lock
            .try_lock()
            .map_err(|_| "A download is already running.")?;
        let _operation = state
            .operation
            .try_lock()
            .map_err(|_| "Another operation is in progress.")?;
        if state.recorder.is_recording()? {
            return Err("Stop dictation before repairing the installation.".into());
        }
        state.model.unload()?;
        download_installation(&app, &model_path, &runtime_dir)
    })
    .await
    .map_err(|e| format!("Download task failed: {e}"))??;
    Ok(output.display().to_string())
}

#[tauri::command]
fn copy_performance_report(state: State<'_, AppState>) -> Result<String, String> {
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let model_path = resolve_model_path(&config, &state.model_dir);
    let metrics = state.report.lock().unwrap();
    let report = format!(
        "T's STT\nbackend=Nemotron 3.5 ASR ONNX INT4\nstatus={}\nlanguage={}\nmodel_path={}\nload_ms={}\naudio_seconds={:.2}\ninference_ms={}\nfinish_ms={}\nmax_queue_ms={}\nfirst_text_ms={:?}\nreal_time_factor={:.3}\noverflowed={}",
        status(&state), config.language,model_path.display(),metrics.load_ms,metrics.audio_seconds,
        metrics.inference_ms,metrics.finish_ms,metrics.max_queue_ms,metrics.first_text_ms,metrics.real_time_factor(),metrics.overflowed);
    set_clipboard(&report)?;
    Ok("Diagnostics copied to clipboard.".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new().expect("failed to initialize application state"))
        .setup(|app| {
            let handle = app.handle().clone();
            app.listen("dictation-auto-stop", move |event| {
                let Ok(expected) = serde_json::from_str::<usize>(event.payload()) else {
                    return;
                };
                let handle = handle.clone();
                thread::spawn(move || {
                    let state = handle.state::<AppState>();
                    let Ok(_operation) = state.operation.lock() else {
                        return;
                    };
                    if state.session_id.load(Ordering::SeqCst) != expected
                        || !state.recorder.is_recording().unwrap_or(false)
                    {
                        return;
                    }
                    let result = stop_inner(&state);
                    set_status(&state, "Idle");
                    match result {
                        Ok(transcript) => {
                            let _ = handle.emit("dictation-completed", transcript);
                        }
                        Err(error) => {
                            let _ = handle.emit("dictation-warning", error);
                        }
                    }
                });
            });
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_initial_data,
            get_transcript_history,
            copy_transcript,
            reinsert_transcript,
            clear_transcript_history,
            load_config,
            save_config,
            list_input_devices,
            get_status,
            start_recording,
            stop_recording_and_process,
            toggle_recording_and_process,
            model_exists,
            download_model,
            copy_performance_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_features_are_off_and_no_vocabulary_is_seeded() {
        let c: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(!c.auto_stop_enabled && !c.vad_enabled && !c.history_enabled);
        assert!(
            !c.replacements_enabled
                && !c.live_preview
                && !c.stable_live_text
                && !c.protect_destination
        );
        assert!(c.replacements.is_empty());
    }
    #[test]
    fn silence_timeout_and_incompatible_options_are_normalized() {
        let c = normalize_config(AppConfig {
            auto_stop_enabled: true,
            vad_enabled: false,
            silence_timeout_secs: f32::NAN,
            streaming: false,
            live_typing: true,
            ..AppConfig::default()
        });
        assert!(!c.auto_stop_enabled && !c.live_typing);
        assert_eq!(c.silence_timeout_secs, 60.0);
    }
    #[test]
    fn checksum_rejects_corrupt_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("model");
        fs::write(&file, b"model weights").unwrap();
        let expected = file_hash(&file).unwrap();
        assert!(file_matches(&file, &expected));
        fs::write(&file, b"broken weights").unwrap();
        assert!(!file_matches(&file, &expected));
    }

    #[test]
    fn old_whisper_config_migrates_to_nemotron_defaults() {
        let config = normalize_config(AppConfig {
            language: "en".to_string(),
            model_path: Some("C:\\models\\ggml-base.en.bin".to_string()),
            ..AppConfig::default()
        });
        assert_eq!(config.language, "en-US");
        assert_eq!(config.model_path, None);
    }

    #[test]
    fn language_tags_are_removed_everywhere() {
        assert_eq!(
            clean_language_tags("<en-US> Hello. <en-US> How are you?"),
            "Hello. How are you?"
        );
    }

    #[test]
    fn streaming_resampler_keeps_audio_across_chunks() {
        let mut resampler = StreamingResampler::new();
        let first = resampler.push(&vec![0.25; 4_800], 48_000);
        let second = resampler.push(&vec![0.25; 4_800], 48_000);
        assert!((first.len() + second.len()) >= 3_199);
        assert!((first.len() + second.len()) <= 3_200);
        assert!(first
            .iter()
            .chain(second.iter())
            .all(|sample| (*sample - 0.25).abs() < f32::EPSILON));
    }
}
