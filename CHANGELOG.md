# Changelog

## Unreleased

- Add macOS support to the Nemotron backend: downloads osx-arm64/osx-x64
  ONNX Runtime GenAI and ONNX Runtime archives, extracts tar.gz/tgz, and
  loads `libonnxruntime-genai.dylib`.

## 0.4.0

- Redesign the settings menu into grouped sections with toggle switches, a
  press-to-record hotkey capture, and a segmented output-mode picker.
- Show unsaved-changes state on the Save button.
- Rename live typing responsiveness to Typing delay.
- Add a Typing speed option controlling how fast keystrokes are sent
  (Instant, Fast, Medium, Slow).
- Add a Keep model in memory option (Always, 15 min, 5 min, Only while
  dictating) that unloads Nemotron from RAM when idle.
- Start recording immediately while the model loads in the background.
- Improve compact capsule contrast on light backgrounds with text/icon
  shadows and an outline ring.

## 0.3.2

- Install matching ONNX Runtime `1.24.1` beside ONNX Runtime GenAI.
- Isolate native DLL loading from incompatible Windows System32 copies.
- Add a real model-load integration test.

## 0.3.1

- Add Fast, Balanced, and Stable live typing responsiveness.
- Clarify the currently supported INT4 quantization and fixed 560 ms model
  inference chunk.

## 0.3.0

- Replace the production Whisper backend with Nemotron 3.5 ASR Streaming.
- Add native Rust ONNX Runtime GenAI integration.
- Add continuous streaming inference and optional live typing.
- Add per-file model/runtime download progress and repair behavior.
- Remove Whisper/GGML settings and migrate old configuration.

## Python Prototype

- Make Nemotron ONNX the default backend.
- Add streaming transcription, live typing, English US default, and language
  tag cleanup.
