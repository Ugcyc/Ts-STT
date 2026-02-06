import logging
from dataclasses import dataclass
from typing import Optional

import numpy as np
from faster_whisper import WhisperModel

logger = logging.getLogger(__name__)


@dataclass
class TranscriptionResult:
    final_text: str
    partial_text: Optional[str] = None


class SpeechToTextEngine:
    def __init__(self, model_size: str = "small", language: str = "en", prefer_gpu: bool = True):
        self.model_size = model_size
        self.language = language
        self.prefer_gpu = prefer_gpu
        self.model = self._load_model()

    def _load_model(self):
        if self.prefer_gpu:
            try:
                logger.info("Loading Whisper model on GPU (cuda)...")
                return WhisperModel(self.model_size, device="cuda", compute_type="float16")
            except Exception as exc:  # noqa: BLE001
                logger.warning("GPU init failed, falling back to CPU: %s", exc)
        logger.info("Loading Whisper model on CPU.")
        return WhisperModel(self.model_size, device="cpu", compute_type="int8")

    def transcribe(self, audio: np.ndarray, sample_rate: int = 16000) -> TranscriptionResult:
        """Run a blocking transcription on the provided audio data.

        TODO: Add streaming partial results so UI can display live text.
        """
        if audio.ndim > 1:
            audio = np.mean(audio, axis=1)
        # faster-whisper expects float32 values in range [-1, 1]
        audio = audio.astype(np.float32)
        segments, _info = self.model.transcribe(
            audio,
            language=self.language,
            beam_size=1,
            vad_filter=False,
        )
        text = "".join(segment.text for segment in segments).strip()
        return TranscriptionResult(final_text=text)
