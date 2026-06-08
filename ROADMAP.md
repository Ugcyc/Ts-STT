# Roadmap

This is a working idea list, not a release commitment.

## Likely Next

- System tray controls and start-with-Windows option
- Better compact overlay feedback while downloading and transcribing
- Live transcript preview inside the app
- Cancel/pause download controls and stronger interrupted-download validation
- Automatic update checks and signed release builds
- Configurable hotkey modes, including push-to-talk where live typing is safe

## Model And Performance

- Evaluate supported GPU execution providers
- Benchmark CPU usage, memory, latency, and real-time factor
- Investigate compatible FP16/INT8 ONNX Runtime GenAI exports
- Add selectable native inference latency only when compatible model exports
  exist
- Explore VAD/end-of-speech behavior and automatic stop

## Dictation Experience

- Custom vocabulary and phrase replacements
- Spoken editing commands
- Per-application output profiles
- Transcript history and replay tools
- Optional post-processing for formatting and tone

## Engineering

- Split the Rust backend into smaller recorder, downloader, config, and output
  modules
- Add downloader/runtime integrity checks
- Add installer smoke tests
- Add release automation and generated checksums
