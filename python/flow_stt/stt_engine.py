import logging
import re
from dataclasses import dataclass
from typing import Optional

import numpy as np

logger = logging.getLogger(__name__)

NEMOTRON_MODEL_NAME = "onnx-community/nemotron-3.5-asr-streaming-0.6b-onnx-int4"
NEMOTRON_LANGUAGE_IDS = {
    "en": 0, "en-US": 0, "en-GB": 1, "es-ES": 2, "es": 3, "es-US": 3,
    "zh-CN": 4, "hi": 6, "hi-IN": 6, "ar": 7, "ar-AR": 7, "fr": 8,
    "fr-FR": 8, "de": 9, "de-DE": 9, "ja": 10, "ja-JP": 10, "ru": 11,
    "ru-RU": 11, "pt-BR": 12, "pt": 13, "pt-PT": 13, "ko": 14,
    "ko-KR": 14, "it": 15, "it-IT": 15, "nl": 16, "nl-NL": 16, "pl": 17,
    "pl-PL": 17, "tr": 18, "tr-TR": 18, "uk": 19, "uk-UA": 19, "ro": 20,
    "ro-RO": 20, "el": 21, "el-GR": 21, "cs": 22, "cs-CZ": 22, "hu": 23,
    "hu-HU": 23, "sv": 24, "sv-SE": 24, "da": 25, "da-DK": 25, "fi": 26,
    "fi-FI": 26, "sk": 28, "sk-SK": 28, "hr": 29, "hr-HR": 29, "bg": 30,
    "bg-BG": 30, "lt": 31, "lt-LT": 31, "th": 32, "th-TH": 32, "vi": 33,
    "vi-VN": 33, "et": 60, "et-EE": 60, "lv": 61, "lv-LV": 61, "sl": 62,
    "sl-SI": 62, "he": 64, "he-IL": 64, "fr-CA": 100, "auto": 101, "mt": 102,
    "mt-MT": 102, "nb": 103, "nb-NO": 103, "nn": 104, "nn-NO": 104,
}
_LANGUAGE_TAG_RE = re.compile(r"\s*<[a-z]{2}(?:-[A-Z]{2})?>\s*")


@dataclass
class TranscriptionResult:
    final_text: str
    partial_text: Optional[str] = None


class NemotronStreamingSession:
    chunk_samples = 8960  # Fixed by the ONNX model's genai_config.json.

    def __init__(self, model, target_lang: str, strip_language_tags: bool, og_module=None):
        if og_module is None:
            import onnxruntime_genai as og_module

        self.strip_language_tags = strip_language_tags
        self.processor = og_module.StreamingProcessor(model)
        self.processor.set_option("use_vad", "false")
        tokenizer = og_module.Tokenizer(model)
        self.tokenizer_stream = tokenizer.create_stream()
        params = og_module.GeneratorParams(model)
        self.generator = og_module.Generator(model, params)
        self.generator.set_runtime_option("lang_id", str(NEMOTRON_LANGUAGE_IDS[target_lang]))
        self._buffer = np.zeros(0, dtype=np.float32)
        self._text = ""

    @property
    def partial_text(self) -> str:
        return self._clean_text(self._text)

    def push(self, audio: np.ndarray) -> str:
        audio = _mono_float32(audio)
        if audio.size == 0:
            return self.partial_text
        self._buffer = np.concatenate((self._buffer, audio))
        while self._buffer.size >= self.chunk_samples:
            chunk = self._buffer[:self.chunk_samples]
            self._buffer = self._buffer[self.chunk_samples:]
            self._process(chunk)
        return self.partial_text

    def finish(self) -> TranscriptionResult:
        if self._buffer.size:
            self._process(self._buffer)
            self._buffer = np.zeros(0, dtype=np.float32)
        inputs = self.processor.flush()
        if inputs is not None:
            self.generator.set_inputs(inputs)
            self._text += SpeechToTextEngine._decode_nemotron_tokens(self.generator, self.tokenizer_stream)
        return TranscriptionResult(final_text=self._clean_text(self._text))

    def _process(self, chunk: np.ndarray) -> None:
        inputs = self.processor.process(chunk)
        if inputs is not None:
            self.generator.set_inputs(inputs)
            self._text += SpeechToTextEngine._decode_nemotron_tokens(self.generator, self.tokenizer_stream)

    def _clean_text(self, text: str) -> str:
        if self.strip_language_tags:
            text = _LANGUAGE_TAG_RE.sub(" ", text)
            text = re.sub(r"\s+([,\.!?])", r"\1", text)
            text = re.sub(r"\s{2,}", " ", text)
        return text.strip()


class SpeechToTextEngine:
    def __init__(
        self,
        model_size: str = "small",
        language: str = "en",
        prefer_gpu: bool = True,
        backend: str = "whisper",
        nemotron_model_name: str = NEMOTRON_MODEL_NAME,
        nemotron_target_lang: str = "en-US",
        nemotron_latency_ms: int = 320,
        nemotron_strip_language_tags: bool = True,
    ):
        self.model_size = model_size
        self.language = language
        self.prefer_gpu = prefer_gpu
        self.backend = backend
        self.nemotron_model_name = nemotron_model_name
        self.nemotron_target_lang = nemotron_target_lang
        self.nemotron_latency_ms = nemotron_latency_ms
        self.nemotron_strip_language_tags = nemotron_strip_language_tags
        self.model = self._load_model()

    @property
    def supports_streaming(self) -> bool:
        return self.backend == "nemotron"

    def start_streaming(self, sample_rate: int = 16000) -> NemotronStreamingSession:
        if not self.supports_streaming:
            raise RuntimeError(f"{self.backend} does not support streaming sessions")
        if sample_rate != 16000:
            raise ValueError(f"Nemotron expects 16 kHz audio; received {sample_rate} Hz")
        return NemotronStreamingSession(
            self.model,
            target_lang=self.nemotron_target_lang,
            strip_language_tags=self.nemotron_strip_language_tags,
        )

    def _load_model(self):
        if self.backend == "nemotron":
            return self._load_nemotron_model()
        if self.backend != "whisper":
            raise ValueError(f"Unsupported STT backend: {self.backend}")

        from faster_whisper import WhisperModel

        if self.prefer_gpu:
            try:
                logger.info("Loading Whisper model on GPU (cuda)...")
                return WhisperModel(self.model_size, device="cuda", compute_type="float16")
            except Exception as exc:  # noqa: BLE001
                logger.warning("GPU init failed, falling back to CPU: %s", exc)
        logger.info("Loading Whisper model on CPU.")
        return WhisperModel(self.model_size, device="cpu", compute_type="int8")

    def _load_nemotron_model(self):
        try:
            from huggingface_hub import snapshot_download
            import onnxruntime_genai as og
        except ImportError as exc:
            raise RuntimeError(
                "Nemotron ONNX requires ONNX Runtime GenAI. Install "
                "python/requirements-nemotron.txt in this virtual environment."
            ) from exc

        if self.nemotron_target_lang not in NEMOTRON_LANGUAGE_IDS:
            raise ValueError(
                f"Unsupported Nemotron language: {self.nemotron_target_lang}. "
                "Use a supported locale or auto."
            )

        logger.info("Downloading/loading Nemotron ONNX model %s...", self.nemotron_model_name)
        model_path = snapshot_download(repo_id=self.nemotron_model_name)
        config = og.Config(model_path)
        config.clear_providers()  # CPU works reliably on native Windows without CUDA setup.
        model = og.Model(config)
        self.nemotron_model_path = model_path
        logger.info("Loaded Nemotron ONNX model on CPU (fixed 560ms streaming context).")
        return model

    def transcribe(self, audio: np.ndarray, sample_rate: int = 16000) -> TranscriptionResult:
        """Run a blocking transcription on the provided audio data.

        TODO: Add streaming partial results so UI can display live text.
        """
        audio = _mono_float32(audio)
        if self.backend == "nemotron":
            return self._transcribe_nemotron(audio, sample_rate)

        # faster-whisper expects float32 values in range [-1, 1]
        segments, _info = self.model.transcribe(
            audio,
            language=self.language,
            beam_size=1,
            vad_filter=False,
        )
        text = "".join(segment.text for segment in segments).strip()
        return TranscriptionResult(final_text=text)

    def _transcribe_nemotron(self, audio: np.ndarray, sample_rate: int) -> TranscriptionResult:
        if sample_rate != 16000:
            raise ValueError(f"Nemotron expects 16 kHz audio; received {sample_rate} Hz")

        session = self.start_streaming(sample_rate)
        session.push(audio)
        return session.finish()

    @staticmethod
    def _decode_nemotron_tokens(generator, tokenizer_stream) -> str:
        text = ""
        while not generator.is_done():
            generator.generate_next_token()
            tokens = generator.get_next_tokens()
            if len(tokens) > 0:
                text += tokenizer_stream.decode(tokens[0])
        return text


def _mono_float32(audio: np.ndarray) -> np.ndarray:
    if audio.ndim > 1:
        audio = np.mean(audio, axis=1)
    return audio.astype(np.float32, copy=False)
