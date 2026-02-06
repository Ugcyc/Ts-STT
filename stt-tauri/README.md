# T's STT (Tauri v2)

Local desktop speech-to-text app (no Python service).

## Current Features

- Whisper transcription via `whisper-rs` (local model files).
- Global hotkey toggle recording (`Ctrl+Shift+Space` by default).
- Output modes:
  - `clipboard` (copy only)
  - `paste` (copy + paste shortcut)
- Theme support: `dark` / `light`.
- Model memory management:
  - preload model when listening starts
  - configurable idle unload timeout
  - `0` means unload immediately after transcription

## Stack

- Tauri v2
- React + TypeScript + Vite
- `cpal` for mic capture
- `whisper-rs` for STT
- `arboard` / `enigo` for clipboard/paste behavior

## Dev Run

```powershell
cd stt-tauri
npm install
npm run tauri dev
```

## Build Commands

Run from `stt-tauri/`.

- Build app + bundles for current OS:
```powershell
npm run tauri build
```

- Windows NSIS installer (`.exe`):
```powershell
npm run tauri build -- --bundles nsis
```

- Windows raw app executable only (no installer):
```powershell
cargo build --release --manifest-path src-tauri/Cargo.toml
```

- Windows MSI installer (requires WiX toolset):
```powershell
npm run tauri build -- --bundles msi
```

- macOS bundles (run on macOS):
```bash
npm run tauri build -- --bundles app,dmg
```

- Linux bundles (run on Linux):
```bash
npm run tauri build -- --bundles deb,appimage,rpm
```

## Build Outputs

- App binary:
  - `src-tauri/target/release/stt-tauri.exe` (Windows)
  - equivalent binary for macOS/Linux in `src-tauri/target/release/`
- Installers/bundles:
  - `src-tauri/target/release/bundle/`

## First Launch

1. Open settings and click `Download` to fetch a Whisper model.
2. Choose microphone, output mode, punctuation mode, theme.
3. Save.
4. Toggle recording via hotkey or mic button.

## Version

Current app version: `0.2.0`.
