# Flow STT (local Windows dictation)

Local, Windows-only speech-to-text dictation similar to Wispr Flow. Hold or toggle a global hotkey, speak, and the text is typed into the focused application, copied to clipboard, or pasted in one go.

## Features
- Local Whisper inference via `faster-whisper` (no cloud calls).
- Global hotkey (default `Ctrl+Shift+Space`) with push-to-talk or toggle mode.
- Lightweight spoken punctuation rules and sentence capitalization.
- Output to active window typing, clipboard, or paste-in-one-go.
- Minimal Tk overlay with status + quick settings editor.
- Cross-platform hotkeys/typing via `pynput` on macOS/Linux; Windows keeps the keyboard backend.

## Installation
1. Install Python 3.11+ on Windows.
2. Install dependencies:
   ```powershell
   pip install -r requirements.txt
   ```
   The first run of `faster-whisper` will download the chosen model to a local cache.
3. (Recommended) Run your terminal as Administrator so the global hotkey and key injection work reliably with the `keyboard` library.

## Running
```powershell
python -m flow_stt
# or
python app.py
```

Keep focus on any text field, hold/tap the hotkey, speak, and the text appears. The Tk overlay shows states: Idle → Listening → Transcribing.

## Configuration
Config is stored at `%USERPROFILE%\AppData\Local\flow_stt\config.json`. Default values:
```json
{
  "hotkey": "ctrl+shift+space",
  "mode": "push_to_talk",
  "output_mode": "type",
  "mic_device": null,
  "model_size": "small",
  "language": "en",
  "spoken_punctuation": true,
  "auto_paste_clipboard": false,
  "silence_timeout_secs": 60.0,
  "enable_ui": true,
  "log_transcripts": false,
  "prefer_gpu": true,
  "replay_hotkey": "ctrl+alt+r"
}
```
You can edit this file directly or use the Settings button in the overlay window to change hotkey, mode, output, mic device, model size, etc. After saving, hotkeys reload automatically.
`output_mode` options: `type` (simulate typing), `clipboard` (copy only, optionally auto-paste), `paste` (copy + paste immediately in one action).

## Spoken punctuation rules
Deterministic replacements:
- `comma` → `,`
- `period` / `full stop` → `.`
- `question mark` → `?`
- `exclamation mark` / `exclamation point` → `!`
- `new line` / `newline` → `\n`
- `new paragraph` → `\n\n`

## Test script
Run a small end-to-end microphone check:
```powershell
python test_transcription.py
```
It records ~4 seconds, runs transcription + punctuation cleanup, and prints the text to stdout.

## Notes
- Everything runs locally; no audio is uploaded.
- Silence timeout is long by default (60s); capture stops immediately when you release/untoggle, or after a minute of silence.
- GPU is used when available (CUDA build); falls back to CPU automatically.
- `ctrl+alt+r` replays the last recorded audio for debugging.
- TODO: Add streaming partial results and a system tray icon; add richer spoken punctuation rules.
- macOS/Linux: hotkeys and typing use `pynput`. On macOS you must grant microphone + accessibility/input-monitoring permissions; on Wayland some environments may block global hotkeys—use clipboard mode if typing is restricted.
