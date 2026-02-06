use std::{
    collections::VecDeque,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use arboard::Clipboard;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use directories::BaseDirs;
#[cfg(not(target_os = "macos"))]
use enigo::{Enigo, Key, KeyboardControllable};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::State;
use whisper_rs::{print_system_info, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct AppConfig {
    hotkey: String,
    mode: String,
    theme: String,
    output_mode: String,
    mic_device: Option<String>,
    model_size: String,
    language: String,
    spoken_punctuation: bool,
    silence_timeout_secs: f32,
    model_idle_unload_secs: f32,
    log_transcripts: bool,
    model_path: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: "ctrl+shift+space".to_string(),
            mode: "toggle".to_string(),
            theme: "dark".to_string(),
            output_mode: "clipboard".to_string(),
            mic_device: None,
            model_size: "base".to_string(),
            language: "en".to_string(),
            spoken_punctuation: true,
            silence_timeout_secs: 60.0,
            model_idle_unload_secs: 300.0,
            log_transcripts: false,
            model_path: None,
        }
    }
}

#[derive(Clone)]
struct PerfLogger {
    inner: Arc<Mutex<PerfLog>>,
}

struct PerfLog {
    app_start: Instant,
    sequence: u64,
    capacity: usize,
    entries: VecDeque<String>,
}

impl PerfLogger {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(PerfLog {
                app_start: Instant::now(),
                sequence: 0,
                capacity,
                entries: VecDeque::new(),
            })),
        }
    }

    fn info(&self, event: &str, details: impl Into<String>) {
        self.push("INFO", event, details.into());
    }

    fn error(&self, event: &str, details: impl Into<String>) {
        self.push("ERROR", event, details.into());
    }

    fn push(&self, level: &str, event: &str, details: String) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.sequence += 1;
            let elapsed_ms = guard.app_start.elapsed().as_millis();
            let wall_epoch_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let details_suffix = if details.trim().is_empty() {
                String::new()
            } else {
                format!(" | {}", details.trim())
            };
            let line = format!(
                "#{:04} +{}ms @{} [{}] {}{}",
                guard.sequence, elapsed_ms, wall_epoch_ms, level, event, details_suffix
            );
            guard.entries.push_back(line);
            while guard.entries.len() > guard.capacity {
                let _ = guard.entries.pop_front();
            }
        }
    }

    fn report(
        &self,
        status: &str,
        config: &AppConfig,
        model_path: &Path,
        model_exists: bool,
    ) -> String {
        let (uptime_ms, lines, kept, cap) = if let Ok(guard) = self.inner.lock() {
            (
                guard.app_start.elapsed().as_millis(),
                guard.entries.iter().cloned().collect::<Vec<String>>(),
                guard.entries.len(),
                guard.capacity,
            )
        } else {
            (
                0,
                vec!["perf log lock unavailable".to_string()],
                0,
                0,
            )
        };
        let wall_epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut output = Vec::new();
        output.push("=== T's STT Performance Report ===".to_string());
        output.push(format!("build_profile={}", build_profile()));
        output.push(format!("build_target={}", build_target_text()));
        output.push(format!(
            "platform={} arch={}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        output.push(format!("whisper_system_info={}", whisper_system_info_text()));
        output.push(format!("process_id={}", current_process_id_text()));
        output.push(format!("current_thread_id={}", current_thread_id_text()));
        output.push(format!("logical_cpu_count={}", num_cpus::get()));
        output.push(format!(
            "available_parallelism={}",
            available_parallelism_text()
        ));
        output.push(format!(
            "process_priority_class={}",
            current_process_priority_class_text()
        ));
        output.push(format!(
            "current_thread_priority={}",
            current_thread_priority_text()
        ));
        output.push(format!(
            "process_power_throttling={}",
            current_process_power_throttling_text()
        ));
        output.push(format!(
            "current_thread_power_throttling={}",
            current_thread_power_throttling_text()
        ));
        output.push(format!(
            "process_memory_snapshot={}",
            current_process_memory_text()
        ));
        output.push(format!(
            "process_cpu_time_ms={}",
            format_optional_u64(current_process_cpu_time_ms())
        ));
        output.push(format!("generated_epoch_ms={}", wall_epoch_ms));
        output.push(format!("uptime_ms={}", uptime_ms));
        output.push(format!("status={}", status));
        output.push(format!("output_mode={}", config.output_mode));
        output.push(format!("language={}", config.language));
        output.push(format!("model_size={}", config.model_size));
        output.push(format!("spoken_punctuation={}", config.spoken_punctuation));
        output.push(format!(
            "model_idle_unload_secs={}",
            config.model_idle_unload_secs
        ));
        output.push(format!(
            "mic_device={}",
            config
                .mic_device
                .clone()
                .unwrap_or_else(|| "Default".to_string())
        ));
        output.push(format!("model_path={}", model_path.display()));
        output.push(format!("model_exists={}", model_exists));
        output.push(format!("entries_kept={}/{}", kept, cap));
        output.push(String::new());
        output.push("--- Events ---".to_string());
        output.extend(lines);
        output.join("\n")
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

enum RecorderCommand {
    Start {
        device_name: Option<String>,
        response: mpsc::Sender<Result<(), String>>,
    },
    Stop {
        response: mpsc::Sender<Result<(Vec<f32>, u32), String>>,
    },
    IsRecording {
        response: mpsc::Sender<bool>,
    },
}

enum ModelCommand {
    Preload {
        model_path: PathBuf,
        idle_unload_secs: f32,
        pin_until_transcribe: bool,
    },
    Transcribe {
        audio_16k: Vec<f32>,
        model_path: PathBuf,
        language: String,
        idle_unload_secs: f32,
        response: mpsc::Sender<Result<String, String>>,
    },
}

#[derive(Clone)]
struct RecorderHandle {
    tx: mpsc::Sender<RecorderCommand>,
}

impl RecorderHandle {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel::<RecorderCommand>();
        thread::spawn(move || recorder_worker(rx));
        Self { tx }
    }

    fn start(&self, device_name: Option<String>) -> Result<(), String> {
        let (response_tx, response_rx) = mpsc::channel();
        self.tx
            .send(RecorderCommand::Start {
                device_name,
                response: response_tx,
            })
            .map_err(|_| "Recorder worker is unavailable.".to_string())?;
        response_rx
            .recv()
            .map_err(|_| "Recorder worker did not respond.".to_string())?
    }

    fn stop(&self) -> Result<(Vec<f32>, u32), String> {
        let (response_tx, response_rx) = mpsc::channel();
        self.tx
            .send(RecorderCommand::Stop {
                response: response_tx,
            })
            .map_err(|_| "Recorder worker is unavailable.".to_string())?;
        response_rx
            .recv()
            .map_err(|_| "Recorder worker did not respond.".to_string())?
    }

    fn is_recording(&self) -> Result<bool, String> {
        let (response_tx, response_rx) = mpsc::channel();
        self.tx
            .send(RecorderCommand::IsRecording {
                response: response_tx,
            })
            .map_err(|_| "Recorder worker is unavailable.".to_string())?;
        response_rx
            .recv()
            .map_err(|_| "Recorder worker did not respond.".to_string())
    }
}

#[derive(Clone)]
struct ModelHandle {
    tx: mpsc::Sender<ModelCommand>,
}

impl ModelHandle {
    fn new(perf: PerfLogger) -> Self {
        let (tx, rx) = mpsc::channel::<ModelCommand>();
        thread::spawn(move || model_worker(rx, perf));
        Self { tx }
    }

    fn transcribe(
        &self,
        audio_16k: Vec<f32>,
        model_path: PathBuf,
        language: String,
        idle_unload_secs: f32,
    ) -> Result<String, String> {
        let (response_tx, response_rx) = mpsc::channel();
        self.tx
            .send(ModelCommand::Transcribe {
                audio_16k,
                model_path,
                language,
                idle_unload_secs,
                response: response_tx,
            })
            .map_err(|_| "Model worker is unavailable.".to_string())?;
        response_rx
            .recv()
            .map_err(|_| "Model worker did not respond.".to_string())?
    }

    fn preload(
        &self,
        model_path: PathBuf,
        idle_unload_secs: f32,
        pin_until_transcribe: bool,
    ) -> Result<(), String> {
        self.tx
            .send(ModelCommand::Preload {
                model_path,
                idle_unload_secs,
                pin_until_transcribe,
            })
            .map_err(|_| "Model worker is unavailable.".to_string())
    }
}

struct AppState {
    recorder: RecorderHandle,
    model: ModelHandle,
    perf: PerfLogger,
    config: Mutex<AppConfig>,
    status: Mutex<String>,
    config_path: PathBuf,
    model_dir: PathBuf,
}

impl AppState {
    fn new() -> Result<Self, String> {
        let app_dir = app_data_dir()?;
        let config_path = app_dir.join("config.json");
        let model_dir = app_dir.join("models");
        fs::create_dir_all(&model_dir)
            .map_err(|e| format!("Failed to create model directory: {e}"))?;

        let config = load_config_from_disk(&config_path)?;
        save_config_to_disk(&config_path, &config)?;
        let perf = PerfLogger::new(400);
        perf.info(
            "startup",
            format!(
                "build_profile={} target={} os={} arch={} config_path={} model_dir={} whisper_system={} logical_cpus={} available_parallelism={}",
                build_profile(),
                build_target_text(),
                std::env::consts::OS,
                std::env::consts::ARCH,
                config_path.display(),
                model_dir.display(),
                whisper_system_info_text(),
                num_cpus::get(),
                available_parallelism_text()
            ),
        );

        Ok(Self {
            recorder: RecorderHandle::new(),
            model: ModelHandle::new(perf.clone()),
            perf,
            config: Mutex::new(config),
            status: Mutex::new("Idle".to_string()),
            config_path,
            model_dir,
        })
    }
}

fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "dev"
    } else {
        "release"
    }
}

fn whisper_system_info_text() -> String {
    print_system_info().trim().to_string()
}

fn build_target_text() -> &'static str {
    option_env!("TARGET").unwrap_or("unknown")
}

fn available_parallelism_text() -> String {
    std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn current_process_id_text() -> String {
    std::process::id().to_string()
}

#[cfg(windows)]
fn current_thread_id_text() -> String {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
        fn GetCurrentThread() -> *mut c_void;
    }
    let tid = unsafe { GetCurrentThreadId() };
    let handle = unsafe { GetCurrentThread() } as usize;
    format!("{tid}(0x{handle:x})")
}

#[cfg(not(windows))]
fn current_thread_id_text() -> String {
    format!("{:?}", std::thread::current().id())
}

#[derive(Clone, Copy)]
struct ProcessMemorySnapshot {
    working_set_bytes: u64,
    pagefile_bytes: u64,
    page_fault_count: u32,
}

fn format_bytes_mib(value: u64) -> f64 {
    value as f64 / (1024.0 * 1024.0)
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "na".to_string())
}

fn optional_u64_delta(before: Option<u64>, after: Option<u64>) -> Option<u64> {
    match (before, after) {
        (Some(b), Some(a)) if a >= b => Some(a - b),
        _ => None,
    }
}

#[cfg(windows)]
#[derive(Clone, Copy)]
#[repr(C)]
struct ProcessPowerThrottlingState {
    version: u32,
    control_mask: u32,
    state_mask: u32,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
#[repr(C)]
struct ThreadPowerThrottlingState {
    version: u32,
    control_mask: u32,
    state_mask: u32,
}

#[cfg(windows)]
const PROCESS_POWER_THROTTLING_CURRENT_VERSION: u32 = 1;
#[cfg(windows)]
const PROCESS_POWER_THROTTLING_EXECUTION_SPEED: u32 = 0x1;
#[cfg(windows)]
const PROCESS_POWER_THROTTLING_CLASS: u32 = 4;
#[cfg(windows)]
const THREAD_POWER_THROTTLING_CURRENT_VERSION: u32 = 1;
#[cfg(windows)]
const THREAD_POWER_THROTTLING_EXECUTION_SPEED: u32 = 0x1;
#[cfg(windows)]
const THREAD_POWER_THROTTLING_CLASS: u32 = 3;

#[cfg(windows)]
fn format_throttling_text(control_mask: u32, state_mask: u32, execution_speed_flag: u32) -> String {
    let execution_speed = if (state_mask & execution_speed_flag) != 0 {
        "on"
    } else {
        "off"
    };
    format!(
        "control_mask={control_mask:#x} state_mask={state_mask:#x} execution_speed={execution_speed}"
    )
}

#[cfg(windows)]
fn current_process_priority_class_text() -> String {
    use std::ffi::c_void;
    const IDLE_PRIORITY_CLASS: u32 = 0x0000_0040;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
    const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
    const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
    const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
    const REALTIME_PRIORITY_CLASS: u32 = 0x0000_0100;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetPriorityClass(h_process: *mut c_void) -> u32;
    }
    let class = unsafe { GetPriorityClass(GetCurrentProcess()) };
    let label = match class {
        IDLE_PRIORITY_CLASS => "IDLE",
        BELOW_NORMAL_PRIORITY_CLASS => "BELOW_NORMAL",
        NORMAL_PRIORITY_CLASS => "NORMAL",
        ABOVE_NORMAL_PRIORITY_CLASS => "ABOVE_NORMAL",
        HIGH_PRIORITY_CLASS => "HIGH",
        REALTIME_PRIORITY_CLASS => "REALTIME",
        0 => "UNKNOWN",
        _ => "OTHER",
    };
    format!("{label}({class:#x})")
}

#[cfg(not(windows))]
fn current_process_priority_class_text() -> String {
    "unsupported".to_string()
}

#[cfg(windows)]
fn current_process_power_throttling_text() -> String {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessInformation(
            h_process: *mut c_void,
            process_information_class: u32,
            process_information: *mut c_void,
            process_information_size: u32,
        ) -> i32;
    }
    let mut state = ProcessPowerThrottlingState {
        version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        control_mask: 0,
        state_mask: 0,
    };
    let ok = unsafe {
        GetProcessInformation(
            GetCurrentProcess(),
            PROCESS_POWER_THROTTLING_CLASS,
            (&mut state as *mut ProcessPowerThrottlingState).cast::<c_void>(),
            std::mem::size_of::<ProcessPowerThrottlingState>() as u32,
        )
    };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        format!("unavailable({})", err)
    } else {
        format_throttling_text(
            state.control_mask,
            state.state_mask,
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        )
    }
}

#[cfg(not(windows))]
fn current_process_power_throttling_text() -> String {
    "unsupported".to_string()
}

#[cfg(windows)]
fn current_thread_priority_text() -> String {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn GetThreadPriority(h_thread: *mut c_void) -> i32;
    }
    let value = unsafe { GetThreadPriority(GetCurrentThread()) };
    format!("{value}")
}

#[cfg(not(windows))]
fn current_thread_priority_text() -> String {
    "unsupported".to_string()
}

#[cfg(windows)]
fn current_thread_power_throttling_text() -> String {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn GetThreadInformation(
            h_thread: *mut c_void,
            thread_information_class: u32,
            thread_information: *mut c_void,
            thread_information_size: u32,
        ) -> i32;
    }
    let mut state = ThreadPowerThrottlingState {
        version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        control_mask: 0,
        state_mask: 0,
    };
    let ok = unsafe {
        GetThreadInformation(
            GetCurrentThread(),
            THREAD_POWER_THROTTLING_CLASS,
            (&mut state as *mut ThreadPowerThrottlingState).cast::<c_void>(),
            std::mem::size_of::<ThreadPowerThrottlingState>() as u32,
        )
    };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        format!("unavailable({})", err)
    } else {
        format_throttling_text(
            state.control_mask,
            state.state_mask,
            THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        )
    }
}

#[cfg(not(windows))]
fn current_thread_power_throttling_text() -> String {
    "unsupported".to_string()
}

#[cfg(windows)]
#[derive(Clone, Copy)]
#[repr(C)]
struct FileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[repr(C)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

#[cfg(windows)]
fn filetime_to_100ns(value: FileTime) -> u64 {
    ((value.high as u64) << 32) | value.low as u64
}

#[cfg(windows)]
fn current_thread_cpu_time_ms() -> Option<u64> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn GetThreadTimes(
            h_thread: *mut c_void,
            creation_time: *mut FileTime,
            exit_time: *mut FileTime,
            kernel_time: *mut FileTime,
            user_time: *mut FileTime,
        ) -> i32;
    }
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = FileTime { low: 0, high: 0 };
    let mut kernel = FileTime { low: 0, high: 0 };
    let mut user = FileTime { low: 0, high: 0 };
    let ok = unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        None
    } else {
        let total_100ns = filetime_to_100ns(kernel) + filetime_to_100ns(user);
        Some(total_100ns / 10_000)
    }
}

#[cfg(not(windows))]
fn current_thread_cpu_time_ms() -> Option<u64> {
    None
}

#[cfg(windows)]
fn current_process_cpu_time_ms() -> Option<u64> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            h_process: *mut c_void,
            creation_time: *mut FileTime,
            exit_time: *mut FileTime,
            kernel_time: *mut FileTime,
            user_time: *mut FileTime,
        ) -> i32;
    }
    let mut creation = FileTime { low: 0, high: 0 };
    let mut exit = FileTime { low: 0, high: 0 };
    let mut kernel = FileTime { low: 0, high: 0 };
    let mut user = FileTime { low: 0, high: 0 };
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        None
    } else {
        let total_100ns = filetime_to_100ns(kernel) + filetime_to_100ns(user);
        Some(total_100ns / 10_000)
    }
}

#[cfg(not(windows))]
fn current_process_cpu_time_ms() -> Option<u64> {
    None
}

#[cfg(windows)]
fn current_process_memory_snapshot() -> Option<ProcessMemorySnapshot> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }
    let mut counters = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
    };
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if ok == 0 {
        None
    } else {
        Some(ProcessMemorySnapshot {
            working_set_bytes: counters.working_set_size as u64,
            pagefile_bytes: counters.pagefile_usage as u64,
            page_fault_count: counters.page_fault_count,
        })
    }
}

#[cfg(not(windows))]
fn current_process_memory_snapshot() -> Option<ProcessMemorySnapshot> {
    None
}

fn current_process_memory_text() -> String {
    if let Some(snapshot) = current_process_memory_snapshot() {
        format!(
            "ws_mib={:.1} pagefile_mib={:.1} page_faults={}",
            format_bytes_mib(snapshot.working_set_bytes),
            format_bytes_mib(snapshot.pagefile_bytes),
            snapshot.page_fault_count
        )
    } else {
        "unavailable".to_string()
    }
}

#[cfg(windows)]
fn set_current_process_high_qos() -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn SetProcessInformation(
            h_process: *mut c_void,
            process_information_class: u32,
            process_information: *mut c_void,
            process_information_size: u32,
        ) -> i32;
    }
    let mut state = ProcessPowerThrottlingState {
        version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        control_mask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        state_mask: 0,
    };
    let ok = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            PROCESS_POWER_THROTTLING_CLASS,
            (&mut state as *mut ProcessPowerThrottlingState).cast::<c_void>(),
            std::mem::size_of::<ProcessPowerThrottlingState>() as u32,
        )
    };
    if ok == 0 {
        Err(format!(
            "SetProcessInformation(ProcessPowerThrottling) failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn set_current_process_high_qos() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn set_current_thread_high_qos() -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn SetThreadInformation(
            h_thread: *mut c_void,
            thread_information_class: u32,
            thread_information: *mut c_void,
            thread_information_size: u32,
        ) -> i32;
    }
    let mut state = ThreadPowerThrottlingState {
        version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
        control_mask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
        state_mask: 0,
    };
    let ok = unsafe {
        SetThreadInformation(
            GetCurrentThread(),
            THREAD_POWER_THROTTLING_CLASS,
            (&mut state as *mut ThreadPowerThrottlingState).cast::<c_void>(),
            std::mem::size_of::<ThreadPowerThrottlingState>() as u32,
        )
    };
    if ok == 0 {
        Err(format!(
            "SetThreadInformation(ThreadPowerThrottling) failed: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn set_current_thread_high_qos() -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn set_current_thread_priority(value: i32) -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn SetThreadPriority(h_thread: *mut c_void, n_priority: i32) -> i32;
    }
    let ok = unsafe { SetThreadPriority(GetCurrentThread(), value) };
    if ok == 0 {
        Err("SetThreadPriority failed".to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn set_current_thread_priority(_value: i32) -> Result<(), String> {
    Ok(())
}

fn app_data_dir() -> Result<PathBuf, String> {
    let base_dirs = BaseDirs::new().ok_or("Failed to resolve user data directory.".to_string())?;
    let dir = base_dirs.data_local_dir().join("flow_stt_tauri");
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create app data directory: {e}"))?;
    Ok(dir)
}

fn normalize_idle_unload_secs(value: f32) -> f32 {
    if !value.is_finite() {
        return 300.0;
    }
    if value < 0.0 {
        return 0.0;
    }
    value
}

fn normalize_output_mode(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "clipboard" => "clipboard".to_string(),
        "paste" => "paste".to_string(),
        _ => "clipboard".to_string(),
    }
}

fn normalize_mode(_value: &str) -> String {
    "toggle".to_string()
}

fn normalize_theme(value: &str) -> String {
    match value.trim().to_lowercase().as_str() {
        "light" => "light".to_string(),
        _ => "dark".to_string(),
    }
}

fn load_config_from_disk(path: &Path) -> Result<AppConfig, String> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }

    let text = fs::read_to_string(path).map_err(|e| format!("Failed to read config: {e}"))?;
    let mut config: AppConfig = serde_json::from_str(&text).unwrap_or_default();
    config.mode = normalize_mode(&config.mode);
    config.theme = normalize_theme(&config.theme);
    config.output_mode = normalize_output_mode(&config.output_mode);
    config.model_idle_unload_secs = normalize_idle_unload_secs(config.model_idle_unload_secs);
    Ok(config)
}

fn save_config_to_disk(path: &Path, config: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;
    fs::write(path, json).map_err(|e| format!("Failed to write config: {e}"))
}

fn set_status(state: &AppState, new_status: &str) {
    let mut previous = None;
    if let Ok(mut status) = state.status.lock() {
        previous = Some(status.clone());
        *status = new_status.to_string();
    }
    if previous.as_deref() != Some(new_status) {
        state.perf.info(
            "status_change",
            format!(
                "{} -> {}",
                previous.unwrap_or_else(|| "unknown".to_string()),
                new_status
            ),
        );
    }
}

fn get_status_text(state: &AppState) -> String {
    state
        .status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|_| "Idle".to_string())
}

fn list_microphones() -> Result<Vec<String>, String> {
    let host = cpal::default_host();
    let devices = host
        .input_devices()
        .map_err(|e| format!("Failed to query input devices: {e}"))?;

    let mut entries = Vec::new();
    for (idx, dev) in devices.enumerate() {
        let name = dev.name().unwrap_or_else(|_| "Unknown Device".to_string());
        entries.push(format!("{idx}: {name}"));
    }

    if entries.is_empty() {
        entries.push("Default".to_string());
    }
    Ok(entries)
}

fn resolve_input_device(selection: Option<String>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let default_device = host
        .default_input_device()
        .ok_or("No input device found.".to_string())?;

    let selected = match selection {
        Some(s) if !s.trim().is_empty() && !s.eq_ignore_ascii_case("default") => s,
        _ => return Ok(default_device),
    };

    if let Some((idx_raw, _rest)) = selected.split_once(':') {
        if let Ok(idx) = idx_raw.trim().parse::<usize>() {
            if let Ok(mut devices) = host.input_devices() {
                if let Some(device) = devices.nth(idx) {
                    return Ok(device);
                }
            }
        }
    }

    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                if name.eq_ignore_ascii_case(selected.trim()) {
                    return Ok(device);
                }
            }
        }
    }

    Ok(default_device)
}

fn append_f32_mono(data: &[f32], channels: usize, sink: &Arc<Mutex<Vec<f32>>>) {
    if channels == 0 || data.is_empty() {
        return;
    }
    let mut mono = Vec::with_capacity(data.len() / channels + 1);
    for frame in data.chunks(channels) {
        let mut sum = 0.0f32;
        for sample in frame {
            sum += *sample;
        }
        mono.push(sum / channels as f32);
    }

    if let Ok(mut guard) = sink.lock() {
        guard.extend_from_slice(&mono);
    }
}

fn append_i16_mono(data: &[i16], channels: usize, sink: &Arc<Mutex<Vec<f32>>>) {
    if channels == 0 || data.is_empty() {
        return;
    }
    let mut mono = Vec::with_capacity(data.len() / channels + 1);
    for frame in data.chunks(channels) {
        let mut sum = 0.0f32;
        for sample in frame {
            sum += *sample as f32 / i16::MAX as f32;
        }
        mono.push(sum / channels as f32);
    }

    if let Ok(mut guard) = sink.lock() {
        guard.extend_from_slice(&mono);
    }
}

fn append_u16_mono(data: &[u16], channels: usize, sink: &Arc<Mutex<Vec<f32>>>) {
    if channels == 0 || data.is_empty() {
        return;
    }
    let mut mono = Vec::with_capacity(data.len() / channels + 1);
    for frame in data.chunks(channels) {
        let mut sum = 0.0f32;
        for sample in frame {
            let centered = (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0;
            sum += centered;
        }
        mono.push(sum / channels as f32);
    }

    if let Ok(mut guard) = sink.lock() {
        guard.extend_from_slice(&mono);
    }
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    sink: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let err_fn = |err| {
        eprintln!("Audio stream error: {err}");
    };

    match sample_format {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _| append_f32_mono(data, channels, &sink),
                err_fn,
                None,
            )
            .map_err(|e| format!("Failed to open f32 input stream: {e}")),
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _| append_i16_mono(data, channels, &sink),
                err_fn,
                None,
            )
            .map_err(|e| format!("Failed to open i16 input stream: {e}")),
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                config,
                move |data: &[u16], _| append_u16_mono(data, channels, &sink),
                err_fn,
                None,
            )
            .map_err(|e| format!("Failed to open u16 input stream: {e}")),
        other => Err(format!("Unsupported input format: {other:?}")),
    }
}

fn recorder_worker(rx: mpsc::Receiver<RecorderCommand>) {
    let mut active: Option<(cpal::Stream, Arc<Mutex<Vec<f32>>>, u32)> = None;

    while let Ok(command) = rx.recv() {
        match command {
            RecorderCommand::Start {
                device_name,
                response,
            } => {
                if active.is_some() {
                    let _ = response.send(Err("Already recording.".to_string()));
                    continue;
                }

                let result = (|| {
                    let device = resolve_input_device(device_name)?;
                    let supported = device
                        .default_input_config()
                        .map_err(|e| format!("Could not read default input format: {e}"))?;
                    let sample_format = supported.sample_format();
                    let stream_config = supported.config();

                    let samples = Arc::new(Mutex::new(Vec::new()));
                    let sink = Arc::clone(&samples);
                    let stream = build_input_stream(&device, &stream_config, sample_format, sink)?;
                    stream
                        .play()
                        .map_err(|e| format!("Failed to start recording stream: {e}"))?;

                    active = Some((stream, samples, stream_config.sample_rate.0));
                    Ok(())
                })();

                let _ = response.send(result);
            }
            RecorderCommand::Stop { response } => {
                let result = (|| {
                    if let Some((stream, samples, sample_rate)) = active.take() {
                        drop(stream);
                        let data = samples
                            .lock()
                            .map(|guard| guard.clone())
                            .map_err(|_| "Audio buffer lock poisoned.".to_string())?;
                        Ok((data, sample_rate))
                    } else {
                        Err("Not currently recording.".to_string())
                    }
                })();
                let _ = response.send(result);
            }
            RecorderCommand::IsRecording { response } => {
                let _ = response.send(active.is_some());
            }
        }
    }
}

fn ensure_context_loaded(
    cached: &mut Option<(PathBuf, WhisperContext)>,
    model_path: &Path,
    perf: &PerfLogger,
    reason: &str,
) -> Result<bool, String> {
    let should_reload = cached
        .as_ref()
        .map(|(loaded_path, _)| loaded_path != model_path)
        .unwrap_or(true);
    if !should_reload {
        perf.info(
            "model_cache_hit",
            format!("reason={} path={}", reason, model_path.display()),
        );
        return Ok(false);
    }

    let load_started = Instant::now();
    let model_size_bytes = fs::metadata(model_path).map(|m| m.len()).unwrap_or(0);
    perf.info(
        "model_load_start",
        format!(
            "reason={} path={} size_bytes={}",
            reason,
            model_path.display(),
            model_size_bytes
        ),
    );
    let model = model_path
        .to_str()
        .ok_or("Model path contains unsupported characters.".to_string())?;
    let context = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .map_err(|e| format!("Failed to load Whisper model: {e}"))?;
    *cached = Some((model_path.to_path_buf(), context));
    perf.info(
        "model_load_done",
        format!(
            "reason={} path={} size_bytes={} elapsed_ms={}",
            reason,
            model_path.display(),
            model_size_bytes,
            load_started.elapsed().as_millis()
        ),
    );
    Ok(true)
}

fn idle_timeout_duration(idle_unload_secs: f32) -> Option<Duration> {
    if !idle_unload_secs.is_finite() {
        return Some(Duration::from_secs(300));
    }
    if idle_unload_secs < 0.0 {
        return Some(Duration::ZERO);
    }
    Some(Duration::from_secs_f32(idle_unload_secs))
}

fn model_worker(rx: mpsc::Receiver<ModelCommand>, perf: PerfLogger) {
    #[cfg(windows)]
    const THREAD_PRIORITY_NORMAL: i32 = 0;
    #[cfg(windows)]
    const THREAD_PRIORITY_HIGHEST: i32 = 2;

    let mut cached: Option<(PathBuf, WhisperContext)> = None;
    let mut last_used: Option<Instant> = None;
    let mut idle_timeout: Option<Duration> = Some(Duration::from_secs(300));
    let mut pinned_until_transcribe = false;
    perf.info(
        "model_worker_start",
        format!(
            "ready=true pid={} tid={} process_priority={} thread_priority={} process_qos={} thread_qos={} logical_cpus={} available_parallelism={} memory={}",
            current_process_id_text(),
            current_thread_id_text(),
            current_process_priority_class_text(),
            current_thread_priority_text(),
            current_process_power_throttling_text(),
            current_thread_power_throttling_text(),
            num_cpus::get(),
            available_parallelism_text(),
            current_process_memory_text()
        ),
    );
    #[cfg(windows)]
    {
        if let Err(err) = set_current_process_high_qos() {
            perf.error("model_worker_process_qos_set", format!("error={err}"));
        } else {
            perf.info(
                "model_worker_process_qos_set",
                format!("state={}", current_process_power_throttling_text()),
            );
        }
        if let Err(err) = set_current_thread_high_qos() {
            perf.error("model_worker_thread_qos_set", format!("error={err}"));
        } else {
            perf.info(
                "model_worker_thread_qos_set",
                format!("state={}", current_thread_power_throttling_text()),
            );
        }
    }

    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(ModelCommand::Preload {
                model_path,
                idle_unload_secs,
                pin_until_transcribe,
            }) => {
                idle_timeout = idle_timeout_duration(idle_unload_secs);
                perf.info(
                    "model_preload_request",
                    format!(
                        "path={} exists={} idle_unload_secs={} pin_until_transcribe={}",
                        model_path.display(),
                        model_path.exists(),
                        idle_unload_secs,
                        pin_until_transcribe
                    ),
                );
                if pin_until_transcribe {
                    pinned_until_transcribe = true;
                }

                if let Err(err) = ensure_context_loaded(&mut cached, &model_path, &perf, "preload") {
                    pinned_until_transcribe = false;
                    perf.error("model_preload_failed", format!("error={err}"));
                    eprintln!("Model preload failed: {err}");
                } else {
                    last_used = Some(Instant::now());
                }
            }
            Ok(ModelCommand::Transcribe {
                audio_16k,
                model_path,
                language,
                idle_unload_secs,
                response,
            }) => {
                idle_timeout = idle_timeout_duration(idle_unload_secs);
                perf.info(
                    "model_transcribe_request",
                    format!(
                        "path={} exists={} language={} samples={} audio_ms={:.1} idle_unload_secs={} process_priority={} thread_priority={} process_qos={} thread_qos={} pid={} tid={} logical_cpus={} available_parallelism={} memory={} proc_cpu_ms={} thread_cpu_ms={}",
                        model_path.display(),
                        model_path.exists(),
                        language,
                        audio_16k.len(),
                        (audio_16k.len() as f64 / 16000.0) * 1000.0,
                        idle_unload_secs,
                        current_process_priority_class_text(),
                        current_thread_priority_text(),
                        current_process_power_throttling_text(),
                        current_thread_power_throttling_text(),
                        current_process_id_text(),
                        current_thread_id_text(),
                        num_cpus::get(),
                        available_parallelism_text(),
                        current_process_memory_text(),
                        format_optional_u64(current_process_cpu_time_ms()),
                        format_optional_u64(current_thread_cpu_time_ms())
                    ),
                );
                #[cfg(windows)]
                {
                    if let Err(err) = set_current_thread_high_qos() {
                        perf.error("model_thread_qos_set", format!("error={err}"));
                    } else {
                        perf.info(
                            "model_thread_qos_set",
                            format!("state={}", current_thread_power_throttling_text()),
                        );
                    }
                    if let Err(err) = set_current_thread_priority(THREAD_PRIORITY_HIGHEST) {
                        perf.error("model_thread_priority_set", format!("error={err}"));
                    } else {
                        perf.info(
                            "model_thread_priority_set",
                            format!("value={} (highest)", THREAD_PRIORITY_HIGHEST),
                        );
                    }
                }

                let result = (|| {
                    ensure_context_loaded(&mut cached, &model_path, &perf, "transcribe")?;

                    let (_path, context) = cached
                        .as_ref()
                        .ok_or("Whisper model cache is unavailable.".to_string())?;
                    let (text, metrics) = transcribe_with_context(context, &audio_16k, &language)?;
                    let audio_ms = (metrics.audio_samples as f64 / 16000.0) * 1000.0;
                    let infer_rtf = if audio_ms > 0.0 {
                        metrics.whisper_full_ms as f64 / audio_ms
                    } else {
                        0.0
                    };
                    let samples_per_sec = if metrics.whisper_full_ms > 0 {
                        (metrics.audio_samples as f64 * 1000.0) / metrics.whisper_full_ms as f64
                    } else {
                        0.0
                    };
                    let ws_delta_mib = match (metrics.memory_before, metrics.memory_after) {
                        (Some(before), Some(after)) => {
                            format_bytes_mib(after.working_set_bytes)
                                - format_bytes_mib(before.working_set_bytes)
                        }
                        _ => 0.0,
                    };
                    let pagefile_delta_mib = match (metrics.memory_before, metrics.memory_after) {
                        (Some(before), Some(after)) => {
                            format_bytes_mib(after.pagefile_bytes)
                                - format_bytes_mib(before.pagefile_bytes)
                        }
                        _ => 0.0,
                    };
                    let page_fault_delta = match (metrics.memory_before, metrics.memory_after) {
                        (Some(before), Some(after)) if after.page_fault_count >= before.page_fault_count => {
                            after.page_fault_count - before.page_fault_count
                        }
                        _ => 0,
                    };
                    let thread_cpu_ms = metrics.thread_cpu_delta_ms.unwrap_or(0);
                    let process_cpu_ms = metrics.process_cpu_delta_ms.unwrap_or(0);
                    let thread_cpu_util = if metrics.whisper_full_ms > 0 {
                        (thread_cpu_ms as f64 / metrics.whisper_full_ms as f64) * 100.0
                    } else {
                        0.0
                    };
                    let process_cpu_util = if metrics.whisper_full_ms > 0 {
                        (process_cpu_ms as f64 / metrics.whisper_full_ms as f64) * 100.0
                    } else {
                        0.0
                    };
                    perf.info(
                        "model_transcribe_done",
                        format!(
                            "samples={} threads={} avail_parallelism={} segments={} state_ms={} infer_ms={} collect_ms={} infer_rtf={:.2} samples_per_sec={:.0} thread_cpu_ms={} process_cpu_ms={} thread_cpu_pct={:.1} process_cpu_pct={:.1} mem_before={} mem_after={} ws_delta_mib={:.1} pagefile_delta_mib={:.1} page_fault_delta={} text_chars={}",
                            metrics.audio_samples,
                            metrics.threads,
                            metrics
                                .available_parallelism
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "na".to_string()),
                            metrics.n_segments,
                            metrics.create_state_ms,
                            metrics.whisper_full_ms,
                            metrics.collect_segments_ms,
                            infer_rtf,
                            samples_per_sec,
                            format_optional_u64(metrics.thread_cpu_delta_ms),
                            format_optional_u64(metrics.process_cpu_delta_ms),
                            thread_cpu_util,
                            process_cpu_util,
                            metrics
                                .memory_before
                                .map(|m| format!(
                                    "ws_mib={:.1},pf_mib={:.1},faults={}",
                                    format_bytes_mib(m.working_set_bytes),
                                    format_bytes_mib(m.pagefile_bytes),
                                    m.page_fault_count
                                ))
                                .unwrap_or_else(|| "na".to_string()),
                            metrics
                                .memory_after
                                .map(|m| format!(
                                    "ws_mib={:.1},pf_mib={:.1},faults={}",
                                    format_bytes_mib(m.working_set_bytes),
                                    format_bytes_mib(m.pagefile_bytes),
                                    m.page_fault_count
                                ))
                                .unwrap_or_else(|| "na".to_string()),
                            ws_delta_mib,
                            pagefile_delta_mib,
                            page_fault_delta,
                            text.len()
                        ),
                    );
                    last_used = Some(Instant::now());
                    Ok(text)
                })();
                if let Err(err) = &result {
                    perf.error("model_transcribe_failed", format!("error={err}"));
                }
                #[cfg(windows)]
                {
                    if let Err(err) = set_current_thread_priority(THREAD_PRIORITY_NORMAL) {
                        perf.error("model_thread_priority_restore", format!("error={err}"));
                    } else {
                        perf.info(
                            "model_thread_priority_restore",
                            format!("value={} (normal)", THREAD_PRIORITY_NORMAL),
                        );
                    }
                }
                pinned_until_transcribe = false;
                if idle_timeout.is_some_and(|d| d.is_zero()) {
                    perf.info(
                        "model_unload",
                        "reason=idle_unload_secs_zero immediate=true".to_string(),
                    );
                    cached = None;
                    last_used = None;
                }

                let _ = response.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let (Some(timeout), Some(last_active)) = (idle_timeout, last_used) {
                    if pinned_until_transcribe {
                        continue;
                    }
                    if cached.is_some() && last_active.elapsed() >= timeout {
                        perf.info(
                            "model_unload",
                            format!(
                                "reason=idle_timeout timeout_ms={} idle_ms={}",
                                timeout.as_millis(),
                                last_active.elapsed().as_millis()
                            ),
                        );
                        cached = None;
                        last_used = None;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                perf.info("model_worker_stop", "channel_disconnected=true".to_string());
                break;
            }
        }
    }
}

fn start_recording_inner(state: &AppState, device_name: Option<String>) -> Result<(), String> {
    let started = Instant::now();
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let selected = device_name.or(config.mic_device.clone());
    let device_label = selected
        .clone()
        .unwrap_or_else(|| "Default".to_string());
    state.recorder.start(selected)?;

    let model_path = resolve_model_path(&config, &state.model_dir);
    if model_path.exists() {
        state.perf.info(
            "model_preload_schedule",
            format!(
                "path={} idle_unload_secs={}",
                model_path.display(),
                config.model_idle_unload_secs
            ),
        );
        if let Err(err) = state
            .model
            .preload(model_path, config.model_idle_unload_secs, true)
        {
            state
                .perf
                .error("model_preload_schedule", format!("error={err}"));
            eprintln!("Model preload scheduling failed: {err}");
        }
    } else {
        state.perf.info(
            "model_preload_skip",
            format!("missing_model_path={}", model_path.display()),
        );
    }

    set_status(state, "Listening");
    state.perf.info(
        "start_recording",
        format!(
            "device={} elapsed_ms={}",
            device_label,
            started.elapsed().as_millis()
        ),
    );
    Ok(())
}

fn stop_recording_inner(state: &AppState) -> Result<(Vec<f32>, u32, AppConfig, PathBuf), String> {
    let started = Instant::now();
    let (audio, sample_rate) = state.recorder.stop()?;
    let audio_secs = if sample_rate == 0 {
        0.0
    } else {
        audio.len() as f32 / sample_rate as f32
    };
    state.perf.info(
        "stop_recording",
        format!(
            "samples={} sample_rate={} audio_secs={:.3} elapsed_ms={}",
            audio.len(),
            sample_rate,
            audio_secs,
            started.elapsed().as_millis()
        ),
    );

    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();

    Ok((audio, sample_rate, config, state.model_dir.clone()))
}

fn model_file_name(config: &AppConfig) -> String {
    let size = config.model_size.trim().to_lowercase();
    match size.as_str() {
        "tiny" | "base" | "small" | "medium" => {
            if config.language.trim().eq_ignore_ascii_case("en") {
                format!("ggml-{size}.en.bin")
            } else {
                format!("ggml-{size}.bin")
            }
        }
        "large" | "large-v3" => "ggml-large-v3.bin".to_string(),
        _ => {
            if config.language.trim().eq_ignore_ascii_case("en") {
                "ggml-base.en.bin".to_string()
            } else {
                "ggml-base.bin".to_string()
            }
        }
    }
}

fn resolve_model_path(config: &AppConfig, model_dir: &Path) -> PathBuf {
    if let Some(path) = config.model_path.as_ref() {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            let custom = PathBuf::from(trimmed);
            if custom.is_absolute() {
                return custom;
            }
            return model_dir.join(custom);
        }
    }

    model_dir.join(model_file_name(config))
}

fn download_model_blocking(config: &AppConfig, model_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(model_dir).map_err(|e| format!("Failed to create model directory: {e}"))?;
    let file_name = model_file_name(config);
    let path = model_dir.join(&file_name);
    if path.exists() {
        return Ok(path);
    }

    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file_name}");
    let mut response = reqwest::blocking::get(&url)
        .map_err(|e| format!("Failed to download model: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Model download failed: {e}"))?;

    let mut file = File::create(&path).map_err(|e| format!("Failed to create model file: {e}"))?;
    let _bytes = response
        .copy_to(&mut file)
        .map_err(|e| format!("Failed to save downloaded model: {e}"))?;

    Ok(path)
}

fn resample_to_16k(input: &[f32], in_rate: u32) -> Vec<f32> {
    if in_rate == 16_000 || input.is_empty() {
        return input.to_vec();
    }

    let ratio = 16_000.0f32 / in_rate as f32;
    let out_len = ((input.len() as f32) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);

    for i in 0..out_len {
        let src_pos = i as f32 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = src_pos - idx as f32;
        let s0 = *input.get(idx).unwrap_or(&0.0);
        let s1 = *input.get(idx + 1).unwrap_or(&s0);
        out.push(s0 + (s1 - s0) * frac);
    }

    out
}

struct TranscribeMetrics {
    audio_samples: usize,
    threads: i32,
    available_parallelism: Option<usize>,
    n_segments: i32,
    create_state_ms: u128,
    whisper_full_ms: u128,
    collect_segments_ms: u128,
    thread_cpu_delta_ms: Option<u64>,
    process_cpu_delta_ms: Option<u64>,
    memory_before: Option<ProcessMemorySnapshot>,
    memory_after: Option<ProcessMemorySnapshot>,
}

fn transcribe_with_context(
    context: &WhisperContext,
    audio: &[f32],
    language: &str,
) -> Result<(String, TranscribeMetrics), String> {
    let create_state_started = Instant::now();
    let mut state = context
        .create_state()
        .map_err(|e| format!("Failed to initialize Whisper state: {e}"))?;
    let create_state_ms = create_state_started.elapsed().as_millis();

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    let selected_language = language.trim();
    if !selected_language.is_empty() {
        params.set_language(Some(selected_language));
    }
    let threads = num_cpus::get().min(8) as i32;
    params.set_n_threads(threads);
    let available_parallelism = std::thread::available_parallelism().ok().map(|n| n.get());

    let thread_cpu_before = current_thread_cpu_time_ms();
    let process_cpu_before = current_process_cpu_time_ms();
    let memory_before = current_process_memory_snapshot();

    let whisper_started = Instant::now();
    state
        .full(params, audio)
        .map_err(|e| format!("Whisper transcription failed: {e}"))?;
    let whisper_full_ms = whisper_started.elapsed().as_millis();
    let thread_cpu_after = current_thread_cpu_time_ms();
    let process_cpu_after = current_process_cpu_time_ms();
    let memory_after = current_process_memory_snapshot();

    let collect_started = Instant::now();
    let n_segments = state
        .full_n_segments()
        .map_err(|e| format!("Failed to read transcription segments: {e}"))?;

    let mut text = String::new();
    for i in 0..n_segments {
        if let Ok(segment) = state.full_get_segment_text(i) {
            text.push_str(&segment);
        }
    }
    let collect_segments_ms = collect_started.elapsed().as_millis();

    Ok((
        text.trim().to_string(),
        TranscribeMetrics {
            audio_samples: audio.len(),
            threads,
            available_parallelism,
            n_segments,
            create_state_ms,
            whisper_full_ms,
            collect_segments_ms,
            thread_cpu_delta_ms: optional_u64_delta(thread_cpu_before, thread_cpu_after),
            process_cpu_delta_ms: optional_u64_delta(process_cpu_before, process_cpu_after),
            memory_before,
            memory_after,
        },
    ))
}

fn matches_rule(words: &[&str], index: usize, keys: &[&str]) -> bool {
    if index + keys.len() > words.len() {
        return false;
    }
    for (offset, key) in keys.iter().enumerate() {
        if !words[index + offset].eq_ignore_ascii_case(key) {
            return false;
        }
    }
    true
}

fn apply_spoken_punctuation(text: &str) -> String {
    const RULES: [(&[&str], &str); 9] = [
        (&["new", "paragraph"], "\n\n"),
        (&["new", "line"], "\n"),
        (&["newline"], "\n"),
        (&["comma"], ","),
        (&["period"], "."),
        (&["full", "stop"], "."),
        (&["question", "mark"], "?"),
        (&["exclamation", "mark"], "!"),
        (&["exclamation", "point"], "!"),
    ];

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut output: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < words.len() {
        let mut matched = false;
        for (keys, symbol) in RULES {
            if matches_rule(&words, i, keys) {
                output.push(symbol.to_string());
                i += keys.len();
                matched = true;
                break;
            }
        }
        if !matched {
            output.push(words[i].to_string());
            i += 1;
        }
    }

    output.join(" ")
}

fn clean_spaces(text: &str) -> String {
    let re_punct = Regex::new(r"\s+([,\.\!\?])").expect("regex should compile");
    let re_space_newline = Regex::new(r"\s+\n").expect("regex should compile");
    let re_newline_space = Regex::new(r"\n\s+").expect("regex should compile");
    let re_multi_space = Regex::new(r"\s{2,}").expect("regex should compile");

    let mut out = re_punct.replace_all(text, "$1").to_string();
    out = re_space_newline.replace_all(&out, "\n").to_string();
    out = re_newline_space.replace_all(&out, "\n").to_string();
    out = re_multi_space.replace_all(&out, " ").to_string();
    out.trim().to_string()
}

fn capitalize_sentences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut capitalize_next = true;
    for ch in text.chars() {
        if capitalize_next && ch.is_alphabetic() {
            for uc in ch.to_uppercase() {
                out.push(uc);
            }
            capitalize_next = false;
        } else {
            out.push(ch);
        }
        if matches!(ch, '.' | '!' | '?' | '\n') {
            capitalize_next = true;
        }
    }
    out.trim().to_string()
}

fn post_process_text(text: &str, spoken_punctuation: bool) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    let mut cleaned = text.trim().to_string();
    if spoken_punctuation {
        cleaned = apply_spoken_punctuation(&cleaned);
    }
    cleaned = clean_spaces(&cleaned);
    capitalize_sentences(&cleaned)
}

fn set_clipboard_text(text: &str) -> Result<(), String> {
    let mut clipboard =
        Clipboard::new().map_err(|e| format!("Clipboard initialization failed: {e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| format!("Failed to copy text to clipboard: {e}"))
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
        first.status,
        first_err,
        second.status,
        second_err
    ))
}

#[cfg(not(target_os = "macos"))]
fn paste_shortcut() -> Result<(), String> {
    let mut enigo = Enigo::new();
    let modifier = Key::Control;
    enigo.key_down(modifier);
    enigo.key_click(Key::Layout('v'));
    enigo.key_up(modifier);
    Ok(())
}

fn output_text(config: &AppConfig, text: &str) -> Result<(), String> {
    let mode = config.output_mode.trim().to_lowercase();
    match mode.as_str() {
        "clipboard" => {
            set_clipboard_text(text)?;
        }
        "paste" => {
            set_clipboard_text(text)?;
            paste_shortcut()?;
        }
        _ => {
            set_clipboard_text(text)?;
        }
    }
    Ok(())
}

fn process_audio(
    audio: Vec<f32>,
    sample_rate: u32,
    config: AppConfig,
    model_dir: PathBuf,
    model: ModelHandle,
    perf: PerfLogger,
) -> Result<TranscriptionResponse, String> {
    let total_started = Instant::now();
    if audio.is_empty() {
        perf.info("process_audio", "empty_audio=true".to_string());
        return Ok(TranscriptionResponse {
            raw_text: String::new(),
            final_text: String::new(),
            seconds: 0.0,
        });
    }

    let seconds = audio.len() as f32 / sample_rate as f32;
    let model_path = resolve_model_path(&config, &model_dir);
    if !model_path.exists() {
        perf.error(
            "process_audio_model_path",
            format!("missing_model_path={}", model_path.display()),
        );
        return Err(format!(
            "Model not found at {}. Download one from the app first.",
            model_path.display()
        ));
    }

    let resample_started = Instant::now();
    let mono_16k = resample_to_16k(&audio, sample_rate);
    let resample_ms = resample_started.elapsed().as_millis();
    perf.info(
        "resample",
        format!(
            "in_rate={} in_samples={} out_samples={} elapsed_ms={}",
            sample_rate,
            audio.len(),
            mono_16k.len(),
            resample_ms
        ),
    );
    let transcribe_started = Instant::now();
    let raw_text = model.transcribe(
        mono_16k,
        model_path,
        config.language.clone(),
        config.model_idle_unload_secs,
    )?;
    let transcribe_ms = transcribe_started.elapsed().as_millis();
    perf.info(
        "transcribe",
        format!(
            "audio_secs={:.3} raw_chars={} elapsed_ms={}",
            seconds,
            raw_text.len(),
            transcribe_ms
        ),
    );
    let post_started = Instant::now();
    let final_text = post_process_text(&raw_text, config.spoken_punctuation);
    let post_ms = post_started.elapsed().as_millis();

    let mut output_ms = 0;
    if !final_text.is_empty() {
        let output_started = Instant::now();
        output_text(&config, &final_text)?;
        output_ms = output_started.elapsed().as_millis();
    }
    perf.info(
        "process_audio_done",
        format!(
            "final_chars={} post_ms={} output_ms={} total_ms={}",
            final_text.len(),
            post_ms,
            output_ms,
            total_started.elapsed().as_millis()
        ),
    );

    Ok(TranscriptionResponse {
        raw_text,
        final_text,
        seconds,
    })
}

#[tauri::command]
fn list_input_devices() -> Result<Vec<String>, String> {
    list_microphones()
}

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> Result<String, String> {
    Ok(get_status_text(&state))
}

#[tauri::command]
fn load_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())
        .map(|cfg| cfg.clone())
}

#[tauri::command]
fn save_config(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig, String> {
    let mut normalized = config;
    normalized.mode = normalize_mode(&normalized.mode);
    normalized.theme = normalize_theme(&normalized.theme);
    normalized.output_mode = normalize_output_mode(&normalized.output_mode);
    normalized.model_idle_unload_secs =
        normalize_idle_unload_secs(normalized.model_idle_unload_secs);
    save_config_to_disk(&state.config_path, &normalized)?;
    let mut guard = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?;
    *guard = normalized.clone();
    state.perf.info(
        "save_config",
        format!(
            "theme={} output_mode={} language={} model_size={} model_path={}",
            normalized.theme,
            normalized.output_mode,
            normalized.language,
            normalized.model_size,
            normalized
                .model_path
                .clone()
                .unwrap_or_else(|| "default".to_string())
        ),
    );
    Ok(normalized)
}

#[tauri::command]
fn get_initial_data(state: State<'_, AppState>) -> Result<InitialData, String> {
    let cfg = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let model_path = resolve_model_path(&cfg, &state.model_dir);
    state.perf.info(
        "get_initial_data",
        format!(
            "model_path={} exists={}",
            model_path.display(),
            model_path.exists()
        ),
    );
    Ok(InitialData {
        config: cfg,
        input_devices: list_microphones()?,
        model_path: model_path.display().to_string(),
        model_exists: model_path.exists(),
        status: get_status_text(&state),
    })
}

#[tauri::command]
fn start_recording(
    state: State<'_, AppState>,
    device_name: Option<String>,
) -> Result<String, String> {
    if let Err(err) = start_recording_inner(&state, device_name) {
        state
            .perf
            .error("start_recording", format!("error={err}"));
        return Err(err);
    }
    Ok(get_status_text(&state))
}

#[tauri::command]
async fn stop_recording_and_process(
    state: State<'_, AppState>,
) -> Result<TranscriptionResponse, String> {
    let command_started = Instant::now();
    let (audio, sample_rate, config, model_dir) = match stop_recording_inner(&state) {
        Ok(payload) => payload,
        Err(err) => {
            set_status(&state, "Idle");
            state
                .perf
                .error("stop_recording_and_process", format!("stop_error={err}"));
            return Err(err);
        }
    };
    set_status(&state, "Transcribing");
    let model = state.model.clone();
    let perf = state.perf.clone();

    let result = tauri::async_runtime::spawn_blocking(move || {
        process_audio(audio, sample_rate, config, model_dir, model, perf)
    })
    .await
    .map_err(|e| format!("Transcription task failed: {e}"))
    .and_then(|inner| inner);

    set_status(&state, "Idle");
    let result = result?;
    state.perf.info(
        "stop_recording_and_process",
        format!(
            "final_chars={} total_ms={}",
            result.final_text.len(),
            command_started.elapsed().as_millis()
        ),
    );
    Ok(result)
}

#[tauri::command]
async fn toggle_recording_and_process(
    state: State<'_, AppState>,
) -> Result<ToggleResponse, String> {
    let command_started = Instant::now();
    let is_recording = state.recorder.is_recording()?;

    if !is_recording {
        if let Err(err) = start_recording_inner(&state, None) {
            state
                .perf
                .error("toggle_recording_and_process", format!("start_error={err}"));
            return Err(err);
        }
        state.perf.info(
            "toggle_recording_and_process",
            format!("action=start elapsed_ms={}", command_started.elapsed().as_millis()),
        );
        return Ok(ToggleResponse {
            is_recording: true,
            status: get_status_text(&state),
            transcript: None,
            note: None,
        });
    }

    let (audio, sample_rate, config, model_dir) = match stop_recording_inner(&state) {
        Ok(payload) => payload,
        Err(err) => {
            set_status(&state, "Idle");
            state
                .perf
                .error("toggle_recording_and_process", format!("stop_error={err}"));
            return Err(err);
        }
    };
    set_status(&state, "Transcribing");
    let model = state.model.clone();
    let perf = state.perf.clone();

    let transcript = tauri::async_runtime::spawn_blocking(move || {
        process_audio(audio, sample_rate, config, model_dir, model, perf)
    })
    .await
    .map_err(|e| format!("Transcription task failed: {e}"))
    .and_then(|inner| inner);

    set_status(&state, "Idle");
    let transcript = transcript?;
    let note = if transcript.final_text.is_empty() {
        Some("No speech detected.".to_string())
    } else {
        None
    };
    state.perf.info(
        "toggle_recording_and_process",
        format!(
            "action=stop final_chars={} elapsed_ms={}",
            transcript.final_text.len(),
            command_started.elapsed().as_millis()
        ),
    );

    Ok(ToggleResponse {
        is_recording: false,
        status: get_status_text(&state),
        transcript: Some(transcript),
        note,
    })
}

#[tauri::command]
fn model_exists(state: State<'_, AppState>) -> Result<bool, String> {
    let cfg = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let path = resolve_model_path(&cfg, &state.model_dir);
    Ok(path.exists())
}

#[tauri::command]
async fn download_model(state: State<'_, AppState>) -> Result<String, String> {
    let started = Instant::now();
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let model_dir = state.model_dir.clone();

    let path =
        tauri::async_runtime::spawn_blocking(move || download_model_blocking(&config, &model_dir))
            .await
            .map_err(|e| format!("Model download task failed: {e}"))??;

    let mut cfg = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    cfg.model_path = Some(path.display().to_string());
    save_config_to_disk(&state.config_path, &cfg)?;
    if let Ok(mut guard) = state.config.lock() {
        *guard = cfg;
    }
    let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    state.perf.info(
        "download_model",
        format!(
            "path={} size_bytes={} elapsed_ms={}",
            path.display(),
            size_bytes,
            started.elapsed().as_millis()
        ),
    );

    Ok(path.display().to_string())
}

#[tauri::command]
fn copy_performance_report(state: State<'_, AppState>) -> Result<String, String> {
    let config = state
        .config
        .lock()
        .map_err(|_| "Config lock poisoned.".to_string())?
        .clone();
    let model_path = resolve_model_path(&config, &state.model_dir);
    let report = state.perf.report(
        &get_status_text(&state),
        &config,
        &model_path,
        model_path.exists(),
    );
    set_clipboard_text(&report)?;
    state.perf.info(
        "copy_performance_report",
        format!("chars={} lines={}", report.len(), report.lines().count()),
    );
    Ok("Performance report copied to clipboard.".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new().expect("failed to initialize application state");
    tauri::Builder::default()
        .manage(state)
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
        .expect("error while running tauri application");
}
