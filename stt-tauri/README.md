# T's STT Desktop App

The production Windows desktop app for local Nemotron streaming dictation.

Current version: **0.4.0**

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

```bash
cd stt-tauri
npm install
npm run tauri -- dev
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
%LOCALAPPDATA%\stt-tauri-target\release\bundle\nsis\
```

Rust checks:

```powershell
$env:RUSTC="$env:USERPROFILE\.cargo\bin\rustc.exe"
$env:RUSTDOC="$env:USERPROFILE\.cargo\bin\rustdoc.exe"
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test
& "$env:USERPROFILE\.cargo\bin\cargo.exe" clippy -- -D warnings
```

Windows MSI installer (requires WiX toolset):

```powershell
npm run build:win:msi
```

macOS bundles (run on macOS):

```bash
npm run build:mac
```

On Apple Silicon the app downloads the `osx-arm64` builds of
ONNX Runtime GenAI and ONNX Runtime instead of the Windows ones, and loads
`libonnxruntime-genai.dylib`. Dictation output uses an AppleScript Cmd+V
paste; grant Microphone and Accessibility permissions when prompted.

Linux and Intel Mac runtime downloads are unavailable for the pinned releases.
The downloader reports this explicitly.

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


## Optional dictation controls

New options are off by default; the existing INT4 model and 560 ms chunk stay in use.
Save settings while recording is stopped. Settings are captured at the start of each dictation.

- **Protect the destination (Windows):** watches the foreground window and physical
  keyboard/mouse editing input. Switching apps, clicking, or manually typing pauses
  insertion for the rest of that dictation. Use the configured hotkey to start and
  stop, then copy the completed transcript if insertion was paused. This is a
  conservative input guard, not an accessibility integration with every text editor.
  Other platforms can use Clipboard mode or leave protection off.
- **Live transcript preview:** shows partial text in the overlay without requiring
  live typing. The compact window expands for errors, preview, and recovery actions.
- **Hold back unfinished words:** leaves the last two whitespace-separated words
  out of live insertion until more speech arrives; stopping inserts the full text.
- **Filter long silences:** enables the runtime's existing CPU Silero VAD. It skips
  prolonged silence but keeps recording, so pauses to think remain possible.
- **Stop after silence:** a separate opt-in switch requiring silence filtering.
  Choose 2–600 seconds. It only arms after recognized text appears, and the runtime's
  detection/context grace period adds to the selected timeout. Leave it off for
  arbitrarily long thinking pauses. Auto-stop is handled by the backend.
- **Phrase replacements:** add, edit, or remove your own literal phrase pairs.
  The list starts empty and has a separate enable switch. Matches ignore case,
  respect word boundaries, prefer longer phrases, and do not cascade. These are
  text replacements, not model fine-tuning. Spoken punctuation also uses whole-word
  matching. The same formatting is applied to live and final output.
- **Local transcript history:** optional text-only storage, with 1–100 entries.
  No audio is recorded to disk. Disabling and saving deletes stored history.
  The last completed transcript is retained in memory even with history off.
  Use **Copy last**, **Reinsert in 3 seconds**, or **Clear transcripts**. Reinsertion
  lets you switch to the intended field during the countdown.

Typing runs on a separate worker with a coalescing update slot. Audio input has a
bounded queue; overflow stops recording and marks the recovered text incomplete.
Audio always processes during capture, even when live output is disabled, avoiding
an unbounded recording buffer. Errors appear in the compact overlay. Performance
reports measure load time, first-text delay, inference/finalization time, maximum
queued audio, and real-time factor; they do not include transcript content.

## Verified downloads and platform checks

Repair checks the pinned model revision against SHA-256 hashes in
`src-tauri/model-manifest.json`. It replaces damaged files and validates complete
transfers before installing them. Runtime archive hashes are pinned to the GitHub
release asset digests for GenAI 0.14.0 and ONNX Runtime 1.24.1. A local receipt checks
installed runtime libraries for accidental corruption; it is not a tamper-proof
signature. Existing runtime installations without a receipt are reinstalled once
when Repair is used. Recording must be stopped before repair.

Verified runtime archives are available for Windows x64 and Apple Silicon.
The pinned releases do not provide the Intel Mac artifacts previously assumed by
this app. Intel Mac and Linux users receive an explicit unsupported-runtime error.

## Validation

The **Desktop checks** workflow builds the frontend and runs Rust unit tests and
Clippy on Windows. Model loading is an explicitly ignored integration test because
it requires the model and runtime. Run it on a configured machine with:

```powershell
$env:TS_STT_TEST_MODEL_DIR="C:\path\to\nemotron-model"
$env:TS_STT_TEST_RUNTIME_DLL="C:\path\to\onnxruntime-genai.dll"
cargo test --manifest-path src-tauri/Cargo.toml loads_installed_model_when_paths_are_provided -- --ignored
```

Manual acceptance checks before merging:

1. Enable protected live typing, dictate, then click another field and type manually.
   Insertion must pause; the completed transcript must still be available to copy.
2. Start/stop via the configured hotkey without moving focus. Final text should be
   inserted, including the last two words when that option is enabled.
3. With silence stopping off, pause beyond the timeout and confirm recording stays
   active. With it on, speak and then pause; confirm automatic completion and recovery.
4. Disconnect the microphone and simulate an output failure; verify a visible error
   and retained partial/completed text. Repair a damaged model file and retry.

The UI smoke test uses a mocked Tauri bridge to check settings, optional defaults,
replacement editing, transcript recovery, and compact error rendering. It does not
simulate microphone hardware or native text insertion:

```bash
npm ci
npx playwright install chromium
npm run test:ui
```

Windows CI uses its installed Edge browser via `TS_STT_BROWSER_CHANNEL=msedge`.
