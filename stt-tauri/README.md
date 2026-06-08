# T's STT Desktop App

The production Windows desktop app for local Nemotron streaming dictation.

Current version: **0.3.2**

## Features

- NVIDIA Nemotron 3.5 ASR Streaming 0.6B, ONNX INT4
- Native Rust integration with Microsoft ONNX Runtime GenAI
- Continuous microphone streaming with live resampling to 16 kHz
- Global toggle hotkey, `Ctrl+Shift+Space` by default
- English US by default with multilingual locale selection
- Native punctuation and capitalization
- Language tags such as `<en-US>` removed automatically
- Visible downloads with per-file byte progress and repair support

Whisper, GGML, and a Python service are not used by the production app.

## Output And Streaming

Output modes:

- `paste`: copy the final transcript and paste it
- `clipboard`: copy the final transcript only
- `type`: type directly into the focused application

With `type`, Streaming, and Live Typing enabled, partial results are written
while you speak and corrected in place as Nemotron revises recent words.

Live typing responsiveness:

- **Fast:** publish partial text every 560 ms
- **Balanced:** publish partial text every 1120 ms
- **Stable:** publish partial text every 1680 ms with fewer visible corrections

The ONNX model itself always processes fixed 560 ms chunks. The responsiveness
setting only controls how frequently partial text is exposed.

## Model And Runtime

The app currently supports:

- Model: `onnx-community/nemotron-3.5-asr-streaming-0.6b-onnx-int4`
- ONNX Runtime GenAI: `0.14.0`
- ONNX Runtime: `1.24.1`
- Platform: Windows x64

The Download/Repair button installs the model and both matching runtime
components in:

```text
%LOCALAPPDATA%\Ts-STT\
```

The app loads its private ONNX Runtime explicitly. This prevents Windows
System32, WebView2, GPU drivers, or another application from supplying an
incompatible `onnxruntime.dll`.

Community FP16 and INT8 exports currently use a different file/runtime contract
and are not selectable in this app.

## First Run

1. Open Settings.
2. Click **Download Nemotron**.
3. Wait for the model and runtime progress to complete.
4. Select microphone, output, streaming, and language options.
5. Click **Save**.
6. Toggle recording with `Ctrl+Shift+Space` or the microphone button.

Everything runs locally after the initial download.

## Run For Development

Requirements:

- Windows x64
- Node.js and npm
- Rust MSVC toolchain
- Microsoft C++ Build Tools and normal Tauri Windows prerequisites

Commands:

```powershell
cd stt-tauri
npm install
npm run tauri dev
```

The npm Tauri commands prepend `%USERPROFILE%\.cargo\bin` because this machine
also has a conflicting Chocolatey Rust installation.

## Build And Test

Frontend build:

```powershell
npm run build
```

Windows NSIS installer:

```powershell
npm run build:win:nsis
```

Installer output:

```text
%LOCALAPPDATA%\stt-tauri-target\x86_64-pc-windows-msvc\release\bundle\nsis\
```

Rust checks:

```powershell
$env:RUSTC="$env:USERPROFILE\.cargo\bin\rustc.exe"
$env:RUSTDOC="$env:USERPROFILE\.cargo\bin\rustdoc.exe"
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test
& "$env:USERPROFILE\.cargo\bin\cargo.exe" clippy -- -D warnings
```

## Running The Installer Elsewhere

The target computer does not need Python, Node.js, Rust, or Visual Studio.

It needs:

- Windows x64
- Microsoft Edge WebView2 Runtime, normally already present on Windows 10/11
- Internet access for the first Nemotron/runtime download

After downloading, transcription works offline.

## Troubleshooting

### Old Whisper Settings Still Appear

Close the running app completely and install the newest version. Versions
before `0.3.0` contained the old Whisper settings UI.

### Nemotron Download Was Interrupted

Open Settings and click **Repair Nemotron**. Existing complete files are kept,
and missing or incomplete files are downloaded again.

### ONNX Runtime GenAI Failed To Load

Install version `0.3.2` or newer, then click **Repair Nemotron**. Older builds
could accidentally load an incompatible `onnxruntime.dll` from System32.

### Live Typing Changes Recent Words

This is expected for streaming ASR. Use Balanced or Stable responsiveness for
fewer visible corrections, and keep focus in the same text field until
dictation stops.

## Architecture

- React + TypeScript + Vite: settings and compact overlay
- Tauri v2: desktop shell and command bridge
- `cpal`: microphone capture
- Rust worker threads: recording, resampling, and streaming inference
- ONNX Runtime GenAI C API: Nemotron streaming processor and decoder
- `enigo` and `arboard`: typing, paste, and clipboard output

The native runtime is loaded dynamically from the managed app-data directory.
