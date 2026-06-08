mod nemotron;

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{mpsc, Mutex},
    thread,
    time::Instant,
};

use arboard::Clipboard;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use directories::BaseDirs;
use enigo::{Enigo, Key, KeyboardControllable};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use nemotron::{language_id, NemotronModel, NemotronSession};

const MODEL_REPO: &str = "onnx-community/nemotron-3.5-asr-streaming-0.6b-onnx-int4";
const MODEL_FOLDER: &str = "nemotron-3.5-asr-streaming-0.6b-onnx-int4";
const GENAI_VERSION: &str = "0.14.0";
const ONNX_RUNTIME_VERSION: &str = "1.24.1";
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
    spoken_punctuation: bool,
    silence_timeout_secs: f32,
    log_transcripts: bool,
    model_path: Option<String>,
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
            spoken_punctuation: false,
            silence_timeout_secs: 60.0,
            log_transcripts: false,
            model_path: None,
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
}

#[derive(Debug, Serialize)]
struct TranscriptionResponse {
    raw_text: String,
    final_text: String,
    seconds: f32,
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
    Start {
        model_path: PathBuf,
        runtime_dll: PathBuf,
        language: String,
        streaming: bool,
        live_typing: bool,
        live_update_ms: u32,
        response: mpsc::Sender<Result<(), String>>,
    },
    Audio {
        samples: Vec<f32>,
        sample_rate: u32,
    },
    Finish {
        response: mpsc::Sender<Result<String, String>>,
    },
}

#[derive(Clone)]
struct ModelHandle {
    tx: mpsc::Sender<ModelCommand>,
}

impl ModelHandle {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || model_worker(rx));
        Self { tx }
    }

    fn start(
        &self,
        model_path: PathBuf,
        runtime_dll: PathBuf,
        language: String,
        streaming: bool,
        live_typing: bool,
        live_update_ms: u32,
    ) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(ModelCommand::Start {
                model_path,
                runtime_dll,
                language,
                streaming,
                live_typing,
                live_update_ms,
                response: tx,
            })
            .map_err(|_| "Nemotron worker is unavailable.".to_string())?;
        rx.recv()
            .map_err(|_| "Nemotron worker did not respond.".to_string())?
    }

    fn push(&self, samples: Vec<f32>, sample_rate: u32) {
        let _ = self.tx.send(ModelCommand::Audio {
            samples,
            sample_rate,
        });
    }

    fn finish(&self) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(ModelCommand::Finish { response: tx })
            .map_err(|_| "Nemotron worker is unavailable.".to_string())?;
        rx.recv()
            .map_err(|_| "Nemotron worker did not respond.".to_string())?
    }
}

struct StreamingResampler {
    source_rate: u32,
    buffer: Vec<f32>,
    position: f64,
}

impl StreamingResampler {
    fn new() -> Self {
        Self {
            source_rate: 0,
            buffer: Vec::new(),
            position: 0.0,
        }
    }

    fn push(&mut self, samples: &[f32], source_rate: u32) -> Vec<f32> {
        if samples.is_empty() || source_rate == 0 {
            return Vec::new();
        }
        if self.source_rate != source_rate {
            self.source_rate = source_rate;
            self.buffer.clear();
            self.position = 0.0;
        }
        if source_rate == 16_000 {
            return samples.to_vec();
        }

        self.buffer.extend_from_slice(samples);
        let step = source_rate as f64 / 16_000.0;
        let mut output = Vec::new();
        while self.position + 1.0 < self.buffer.len() as f64 {
            let index = self.position.floor() as usize;
            let fraction = (self.position - index as f64) as f32;
            output.push(
                self.buffer[index] + (self.buffer[index + 1] - self.buffer[index]) * fraction,
            );
            self.position += step;
        }
        let consumed = self.position.floor() as usize;
        if consumed > 0 {
            self.buffer.drain(..consumed);
            self.position -= consumed as f64;
        }
        output
    }
}

fn model_worker(rx: mpsc::Receiver<ModelCommand>) {
    let mut cached: Option<(PathBuf, PathBuf, NemotronModel)> = None;
    let mut session: Option<NemotronSession> = None;
    let mut resampler = StreamingResampler::new();
    let mut delayed_audio = Vec::new();
    let mut streaming = true;
    let mut live_typer: Option<LiveTyper> = None;
    let mut live_update_samples = 8_960usize;
    let mut samples_since_live_update = 0usize;

    while let Ok(command) = rx.recv() {
        match command {
            ModelCommand::Start {
                model_path,
                runtime_dll,
                language,
                streaming: use_streaming,
                live_typing,
                live_update_ms,
                response,
            } => {
                let result = (|| {
                    if language_id(&language).is_none() {
                        return Err(format!("Unsupported Nemotron language: {language}"));
                    }
                    let reload = cached
                        .as_ref()
                        .map(|(m, r, _)| m != &model_path || r != &runtime_dll)
                        .unwrap_or(true);
                    if reload {
                        cached = Some((
                            model_path.clone(),
                            runtime_dll.clone(),
                            NemotronModel::load(&runtime_dll, &model_path)?,
                        ));
                    }
                    let model = &cached.as_ref().expect("model was just loaded").2;
                    session = Some(model.start_session(&language)?);
                    resampler = StreamingResampler::new();
                    delayed_audio.clear();
                    streaming = use_streaming;
                    live_typer = live_typing.then(LiveTyper::new);
                    live_update_samples = (live_update_ms as usize * 16).max(8_960);
                    samples_since_live_update = 0;
                    Ok(())
                })();
                let _ = response.send(result);
            }
            ModelCommand::Audio {
                samples,
                sample_rate,
            } => {
                let audio_16k = resampler.push(&samples, sample_rate);
                if audio_16k.is_empty() {
                    continue;
                }
                if streaming {
                    samples_since_live_update += audio_16k.len();
                    if let Some(active) = session.as_mut() {
                        match active.push(&audio_16k) {
                            Ok(partial) => {
                                if samples_since_live_update >= live_update_samples {
                                    if let Some(typer) = live_typer.as_mut() {
                                        typer.update(&partial);
                                    }
                                    samples_since_live_update %= live_update_samples;
                                }
                            }
                            Err(err) => eprintln!("Nemotron streaming error: {err}"),
                        }
                    }
                } else {
                    delayed_audio.extend(audio_16k);
                }
            }
            ModelCommand::Finish { response } => {
                let result = (|| {
                    let active = session
                        .as_mut()
                        .ok_or_else(|| "No Nemotron session is active.".to_string())?;
                    if !streaming && !delayed_audio.is_empty() {
                        active.push(&delayed_audio)?;
                    }
                    let text = active.finish()?;
                    if let Some(typer) = live_typer.as_mut() {
                        typer.update(&text);
                    }
                    session = None;
                    delayed_audio.clear();
                    live_typer = None;
                    Ok(text)
                })();
                let _ = response.send(result);
            }
        }
    }
}

struct LiveTyper {
    typed: String,
}

impl LiveTyper {
    fn new() -> Self {
        Self {
            typed: String::new(),
        }
    }

    fn update(&mut self, text: &str) {
        if text == self.typed {
            return;
        }
        let prefix = self
            .typed
            .chars()
            .zip(text.chars())
            .take_while(|(left, right)| left == right)
            .count();
        let remove = self.typed.chars().count().saturating_sub(prefix);
        let replacement: String = text.chars().skip(prefix).collect();
        let mut enigo = Enigo::new();
        for _ in 0..remove {
            enigo.key_click(Key::Backspace);
        }
        if !replacement.is_empty() {
            enigo.key_sequence(&replacement);
        }
        self.typed = text.to_string();
    }
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
    let err = |error| eprintln!("Audio stream error: {error}");
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
        Ok(Self {
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
    fs::write(path, data).map_err(|e| format!("Failed to save config: {e}"))
}

fn normalize_config(mut config: AppConfig) -> AppConfig {
    config.mode = "toggle".to_string();
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
        .join(format!("onnxruntime-genai-{GENAI_VERSION}-win-x64"))
        .join("lib")
        .join("onnxruntime-genai.dll")
}

fn base_runtime_dll(runtime_dir: &Path) -> PathBuf {
    runtime_dll(runtime_dir)
        .parent()
        .expect("runtime DLL has a parent")
        .join("onnxruntime.dll")
}

fn installation_exists(model_path: &Path, runtime_path: &Path) -> bool {
    runtime_path.is_file()
        && runtime_path
            .parent()
            .is_some_and(|parent| parent.join("onnxruntime.dll").is_file())
        && MODEL_FILES
            .iter()
            .all(|file| model_path.join(file).is_file())
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

fn download_file(
    app: &AppHandle,
    url: &str,
    path: &Path,
    item: &str,
    item_index: usize,
    item_count: usize,
) -> Result<(), String> {
    if path.exists() {
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
    let mut response = reqwest::blocking::get(url)
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
    fs::rename(&temporary, path).map_err(|e| format!("Failed to finish {}: {e}", path.display()))
}

fn download_installation(
    app: &AppHandle,
    model_path: &Path,
    runtime_dir: &Path,
) -> Result<(), String> {
    let item_count = MODEL_FILES.len() + 2;
    fs::create_dir_all(model_path).map_err(|e| format!("Failed to create model folder: {e}"))?;
    for (index, file) in MODEL_FILES.iter().enumerate() {
        let url = format!("https://huggingface.co/{MODEL_REPO}/resolve/main/{file}");
        download_file(
            app,
            &url,
            &model_path.join(file),
            file,
            index + 1,
            item_count,
        )?;
    }

    let dll = runtime_dll(runtime_dir);
    if !dll.exists() {
        let runtime_item = format!("ONNX Runtime GenAI {GENAI_VERSION}");
        let archive = runtime_dir.join(format!("onnxruntime-genai-{GENAI_VERSION}-win-x64.zip"));
        let url = format!(
            "https://github.com/microsoft/onnxruntime-genai/releases/download/v{GENAI_VERSION}/onnxruntime-genai-{GENAI_VERSION}-win-x64.zip"
        );
        download_file(
            app,
            &url,
            &archive,
            &runtime_item,
            item_count - 1,
            item_count,
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
        let file =
            File::open(&archive).map_err(|e| format!("Failed to open runtime archive: {e}"))?;
        let mut zip =
            zip::ZipArchive::new(file).map_err(|e| format!("Invalid runtime archive: {e}"))?;
        zip.extract(runtime_dir)
            .map_err(|e| format!("Failed to extract runtime: {e}"))?;
        fs::remove_file(archive).map_err(|e| format!("Failed to remove runtime archive: {e}"))?;
    }

    let base_dll = base_runtime_dll(runtime_dir);
    if !base_dll.exists() {
        let runtime_item = format!("ONNX Runtime {ONNX_RUNTIME_VERSION}");
        let archive = runtime_dir.join(format!("onnxruntime-win-x64-{ONNX_RUNTIME_VERSION}.zip"));
        let url = format!(
            "https://github.com/microsoft/onnxruntime/releases/download/v{ONNX_RUNTIME_VERSION}/onnxruntime-win-x64-{ONNX_RUNTIME_VERSION}.zip"
        );
        download_file(app, &url, &archive, &runtime_item, item_count, item_count)?;
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
        let file = File::open(&archive)
            .map_err(|e| format!("Failed to open ONNX Runtime archive: {e}"))?;
        let mut zip =
            zip::ZipArchive::new(file).map_err(|e| format!("Invalid ONNX Runtime archive: {e}"))?;
        zip.extract(runtime_dir)
            .map_err(|e| format!("Failed to extract ONNX Runtime: {e}"))?;
        let source_lib = runtime_dir
            .join(format!("onnxruntime-win-x64-{ONNX_RUNTIME_VERSION}"))
            .join("lib");
        let destination_lib = dll
            .parent()
            .ok_or_else(|| "ONNX Runtime GenAI library folder is missing.".to_string())?;
        for file_name in ["onnxruntime.dll", "onnxruntime_providers_shared.dll"] {
            fs::copy(source_lib.join(file_name), destination_lib.join(file_name))
                .map_err(|e| format!("Failed to install {file_name}: {e}"))?;
        }
        fs::remove_file(archive)
            .map_err(|e| format!("Failed to remove ONNX Runtime archive: {e}"))?;
    }
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
        if let Some((index, _)) = selected.split_once(':') {
            if let Ok(index) = index.trim().parse::<usize>() {
                if let Ok(mut devices) = host.input_devices() {
                    if let Some(device) = devices.nth(index) {
                        return Ok(device);
                    }
                }
            }
        }
    }
    host.default_input_device()
        .ok_or_else(|| "No microphone found.".to_string())
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

fn clean_language_tags(text: &str) -> String {
    let tags = Regex::new(r"\s*<[a-z]{2}(?:-[A-Z]{2})?>\s*").expect("valid regex");
    tags.replace_all(text, " ").trim().to_string()
}

fn post_process_text(text: &str, spoken_punctuation: bool) -> String {
    let mut output = clean_language_tags(text);
    if spoken_punctuation {
        for (words, symbol) in [
            ("new paragraph", "\n\n"),
            ("new line", "\n"),
            ("comma", ","),
            ("period", "."),
            ("question mark", "?"),
            ("exclamation mark", "!"),
        ] {
            output = output.replace(words, symbol);
        }
    }
    let punctuation = Regex::new(r"\s+([,.!?])").expect("valid regex");
    let spaces = Regex::new(r"[ \t]{2,}").expect("valid regex");
    output = punctuation.replace_all(&output, "$1").to_string();
    spaces.replace_all(&output, " ").trim().to_string()
}

fn set_clipboard(text: &str) -> Result<(), String> {
    Clipboard::new()
        .map_err(|e| format!("Clipboard unavailable: {e}"))?
        .set_text(text.to_string())
        .map_err(|e| format!("Failed to copy transcript: {e}"))
}

fn output_text(config: &AppConfig, text: &str) -> Result<(), String> {
    match config.output_mode.as_str() {
        "clipboard" => set_clipboard(text),
        "type" => {
            if !config.live_typing {
                Enigo::new().key_sequence(text);
            }
            Ok(())
        }
        _ => {
            set_clipboard(text)?;
            let mut enigo = Enigo::new();
            enigo.key_down(Key::Control);
            enigo.key_click(Key::Layout('v'));
            enigo.key_up(Key::Control);
            Ok(())
        }
    }
}

fn start_inner(state: &AppState, device_name: Option<String>) -> Result<(), String> {
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let model_path = resolve_model_path(&config, &state.model_dir);
    let runtime = runtime_dll(&state.runtime_dir);
    if !installation_exists(&model_path, &runtime) {
        return Err(
            "Nemotron is not installed. Open Settings and click Download Nemotron.".to_string(),
        );
    }
    set_status(state, "Loading Nemotron");
    state.model.start(
        model_path,
        runtime,
        config.language.clone(),
        config.streaming,
        config.live_typing && config.output_mode == "type",
        config.live_update_ms,
    )?;
    state
        .recorder
        .start(device_name.or(config.mic_device), state.model.clone())?;
    set_status(state, "Listening");
    Ok(())
}

fn stop_inner(state: &AppState) -> Result<TranscriptionResponse, String> {
    let seconds = state.recorder.stop()?;
    set_status(state, "Finishing");
    let raw_text = state.model.finish()?;
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let final_text = post_process_text(&raw_text, config.spoken_punctuation);
    if !final_text.is_empty() {
        output_text(&config, &final_text)?;
    }
    Ok(TranscriptionResponse {
        raw_text,
        final_text,
        seconds,
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
    let config = normalize_config(config);
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
fn start_recording(
    state: State<'_, AppState>,
    device_name: Option<String>,
) -> Result<String, String> {
    start_inner(&state, device_name)?;
    Ok(status(&state))
}

#[tauri::command]
async fn stop_recording_and_process(
    state: State<'_, AppState>,
) -> Result<TranscriptionResponse, String> {
    let result = stop_inner(&state);
    set_status(&state, "Idle");
    result
}

#[tauri::command]
async fn toggle_recording_and_process(
    state: State<'_, AppState>,
) -> Result<ToggleResponse, String> {
    if !state.recorder.is_recording()? {
        start_inner(&state, None)?;
        return Ok(ToggleResponse {
            is_recording: true,
            status: status(&state),
            transcript: None,
            note: None,
        });
    }
    let transcript = stop_inner(&state);
    set_status(&state, "Idle");
    let transcript = transcript?;
    let note = transcript
        .final_text
        .is_empty()
        .then(|| "No speech detected.".to_string());
    Ok(ToggleResponse {
        is_recording: false,
        status: status(&state),
        transcript: Some(transcript),
        note,
    })
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
    let report = format!(
        "T's STT\nbackend=Nemotron 3.5 ASR ONNX INT4\nstatus={}\nlanguage={}\nstreaming={}\nlive_typing={}\nlive_update_ms={}\nmodel_path={}\nruntime={}\ninstalled={}",
        status(&state),
        config.language,
        config.streaming,
        config.live_typing,
        config.live_update_ms,
        model_path.display(),
        runtime_dll(&state.runtime_dir).display(),
        installation_exists(&model_path, &runtime_dll(&state.runtime_dir)),
    );
    set_clipboard(&report)?;
    Ok("Diagnostics copied to clipboard.".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new().expect("failed to initialize application state"))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            get_initial_data,
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
