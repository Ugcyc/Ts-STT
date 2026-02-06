import time

from flow_stt.audio_capture import AudioCapture
from flow_stt.config import ConfigManager
from flow_stt.postprocess import TextPostProcessor
from flow_stt.stt_engine import SpeechToTextEngine


def main():
    cfg = ConfigManager().config
    print("Recording from microphone for ~4 seconds. Speak now...")
    capturer = AudioCapture(
        device=cfg.mic_device,
        sample_rate=16000,
        silence_timeout=None,
    )
    data = capturer.record_blocking(4.0)
    print("Transcribing...")
    engine = SpeechToTextEngine(
        model_size=cfg.model_size,
        language=cfg.language,
        prefer_gpu=cfg.prefer_gpu,
    )
    result = engine.transcribe(data, sample_rate=capturer.sample_rate)
    processed = TextPostProcessor(enable_spoken_punctuation=cfg.spoken_punctuation).process(result.final_text)
    print("Result:", processed.final_text)


if __name__ == "__main__":
    main()
