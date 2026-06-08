# Flow STT Python Test App

The Python implementation is the prototype and test harness for the production
Tauri app. It defaults to local Nemotron 3.5 ASR Streaming through ONNX Runtime
GenAI.

## Setup

```powershell
cd python
.\.venv\Scripts\python.exe -m pip install -r requirements.txt
.\.venv\Scripts\python.exe -m pip install -r requirements-nemotron.txt
```

The first Nemotron run downloads
`onnx-community/nemotron-3.5-asr-streaming-0.6b-onnx-int4` from Hugging Face.
Inference is local after that download.

## Run

```powershell
.\.venv\Scripts\python.exe -m flow_stt
```

Nemotron processes microphone audio continuously in fixed 560 ms chunks.
English US (`en-US`) is the default language, and emitted language tags such as
`<en-US>` are removed.

For live dictation, set:

- Mode: `toggle`
- Output: `type`
- Streaming live typing: enabled

Keep focus in the same text field until dictation stops because revised partial
results are corrected with backspaces and replacement text.

## Local Hosting Notes

- The integrated route is the community ONNX INT4 conversion through
  Microsoft's ONNX Runtime GenAI. It works on native Windows CPU.
- NVIDIA NeMo is the full reference Python runtime and is primarily suited to a
  supported Linux environment.
- NVIDIA Speech NIM is the container/API deployment route, but it is not used
  by this desktop app.

## Configuration

Configuration is stored at
`%USERPROFILE%\AppData\Local\flow_stt\config.json`. Important defaults:

```json
{
  "mode": "toggle",
  "output_mode": "type",
  "stt_backend": "nemotron",
  "nemotron_model_name": "onnx-community/nemotron-3.5-asr-streaming-0.6b-onnx-int4",
  "nemotron_target_lang": "en-US",
  "nemotron_latency_ms": 560,
  "nemotron_strip_language_tags": true,
  "live_typing": true
}
```

Use `python test_transcription.py` for a short microphone transcription test.
