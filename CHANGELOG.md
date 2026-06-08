# Changelog

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
