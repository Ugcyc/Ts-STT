# Ts-STT

Speech-to-text project with two local implementations:

- `python/`: Flow-style Windows dictation app using `faster-whisper`.
- `stt-tauri/`: Tauri desktop app (React + Rust) using `whisper-rs`.

## Repository Layout

```text
.
|- python/
|- stt-tauri/
`- .gitignore
```

## Quick Start

### Python App (`python/`)

Requirements:
- Python 3.11+

Commands:

```powershell
cd python
pip install -r requirements.txt
python -m flow_stt
```

Alternative run command:

```powershell
python app.py
```

### Tauri App (`stt-tauri/`)

Requirements:
- Node.js + npm
- Rust toolchain (`cargo`)
- Tauri prerequisites for Windows

Commands:

```powershell
cd stt-tauri
npm install
npm run tauri dev
```

## Build

From `stt-tauri/`:

```powershell
npm run tauri build
```

## Notes

- Both apps are local-first STT workflows.
- See `python/README.md` for Flow STT details.
- See `stt-tauri/README.md` for Tauri app details and build variants.
