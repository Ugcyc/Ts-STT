# T's STT

Local, streaming speech-to-text powered by NVIDIA Nemotron 3.5 ASR.

The project contains:

- `stt-tauri/`: the production Windows desktop app built with React, Rust, and
  Tauri.
- `python/`: the original prototype and a useful test harness for experiments.

## Current Release

Tauri app version: **0.3.2**

The production app is fully local after its first model/runtime download. It
uses the INT4 ONNX export of Nemotron 3.5 ASR Streaming and types, pastes, or
copies transcripts into other applications.

Highlights:

- Native Rust streaming inference with no Python service
- English US by default, with multilingual locale selection
- Optional live typing with configurable responsiveness
- Visible model/runtime download progress with repair support
- Automatic private ONNX Runtime installation to avoid Windows DLL conflicts

## Quick Start

### Run The Production App

```powershell
cd stt-tauri
npm install
npm run tauri dev
```

Open Settings and click **Download Nemotron** on first run.

### Build The Windows Installer

```powershell
cd stt-tauri
npm run build:win:nsis
```

The installer is written under:

```text
%LOCALAPPDATA%\stt-tauri-target\x86_64-pc-windows-msvc\release\bundle\nsis\
```

See [stt-tauri/README.md](stt-tauri/README.md) for setup, architecture,
troubleshooting, and build details.

## Python Test App

```powershell
cd python
.\.venv\Scripts\python.exe -m flow_stt
```

See [python/README.md](python/README.md) for Python setup details.

## Project Notes

- [CHANGELOG.md](CHANGELOG.md) records completed releases.
- [ROADMAP.md](ROADMAP.md) collects likely future work without committing to a
  specific schedule.
