import logging
import threading
import time
from queue import Queue

import numpy as np
import sounddevice as sd

from .audio_capture import AudioCapture
from .config import ConfigManager
from .postprocess import TextPostProcessor
from .stt_engine import SpeechToTextEngine
from .integration import get_integration
from .live_typing import LiveTypingSession
from .ui import StatusUI


logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)


class DictationApp:
    def __init__(self):
        self.cfg_manager = ConfigManager()
        self.cfg = self.cfg_manager.config

        self._listening = False
        self._lock = threading.Lock()
        self._last_audio: np.ndarray | None = None
        self._stream_queue: Queue[np.ndarray | None] | None = None
        self._stream_thread: threading.Thread | None = None

        self.postprocessor = TextPostProcessor(enable_spoken_punctuation=self.cfg.spoken_punctuation)
        try:
            self.stt_engine = self._create_stt_engine(self.cfg)
        except Exception as exc:  # noqa: BLE001
            if self.cfg.stt_backend == "whisper":
                raise
            logger.error(
                "Could not load %s backend; reverting to Whisper: %s",
                self.cfg.stt_backend,
                exc,
            )
            self.cfg = self.cfg_manager.update(stt_backend="whisper")
            self.stt_engine = self._create_stt_engine(self.cfg)
        self.integration = get_integration(self.cfg.output_mode, self.cfg.auto_paste_clipboard)
        self.audio = AudioCapture(
            device=self.cfg.mic_device,
            sample_rate=16000,
            silence_timeout=self.cfg.silence_timeout_secs,
            on_silence=self._on_silence_timeout,
            on_audio=self._on_audio_chunk,
        )

        self.ui = StatusUI(on_settings_saved=self._reload_config) if self.cfg.enable_ui else None

    @staticmethod
    def _create_stt_engine(cfg):
        return SpeechToTextEngine(
            model_size=cfg.model_size,
            language=cfg.language,
            prefer_gpu=cfg.prefer_gpu,
            backend=cfg.stt_backend,
            nemotron_model_name=cfg.nemotron_model_name,
            nemotron_target_lang=cfg.nemotron_target_lang,
            nemotron_latency_ms=cfg.nemotron_latency_ms,
            nemotron_strip_language_tags=cfg.nemotron_strip_language_tags,
        )

    def _set_status(self, status: str):
        if self.ui:
            self.ui.set_status(status)
        else:
            logger.info("Status: %s", status)

    def _register_hotkeys(self):
        # Clear both to be safe when switching platforms/configs.
        try:
            self.integration.clear_hotkeys()
        except AttributeError:
            pass
        if self.cfg.mode == "toggle":
            self.integration.register_hotkey_toggle(self.cfg.hotkey, self._toggle_listening)
        else:
            self.integration.register_hotkey_push_to_talk(self.cfg.hotkey, self.start_listening, self.stop_listening)
        if self.cfg.replay_hotkey:
            # Only WindowsIntegration supports register_hotkey_action; map to toggle for other platforms.
            try:
                self.integration.register_hotkey_action(self.cfg.replay_hotkey, self.replay_last_recording)  # type: ignore[attr-defined]
            except AttributeError:
                self.integration.register_hotkey_toggle(self.cfg.replay_hotkey, self.replay_last_recording)

    def _reload_config(self):
        logger.info("Reloading config from disk.")
        cfg = self.cfg_manager.load()
        if (
            self.stt_engine.backend != cfg.stt_backend
            or self.stt_engine.model_size != cfg.model_size
            or self.stt_engine.language != cfg.language
            or self.stt_engine.prefer_gpu != cfg.prefer_gpu
            or self.stt_engine.nemotron_model_name != cfg.nemotron_model_name
            or self.stt_engine.nemotron_target_lang != cfg.nemotron_target_lang
            or self.stt_engine.nemotron_latency_ms != cfg.nemotron_latency_ms
            or self.stt_engine.nemotron_strip_language_tags != cfg.nemotron_strip_language_tags
        ):
            try:
                new_engine = self._create_stt_engine(cfg)
            except Exception:
                self.cfg_manager.update(stt_backend=self.stt_engine.backend)
                raise
            self.stt_engine = new_engine
        self.cfg = cfg
        self.postprocessor.enable_spoken_punctuation = cfg.spoken_punctuation
        self.integration.output_mode = cfg.output_mode
        self.integration.auto_paste_clipboard = cfg.auto_paste_clipboard
        self.audio.device = cfg.mic_device
        self.audio.silence_timeout = cfg.silence_timeout_secs
        self._register_hotkeys()

    def _toggle_listening(self):
        if self._listening:
            self.stop_listening()
        else:
            self.start_listening()

    def start_listening(self):
        with self._lock:
            if self._listening or (self._stream_thread and self._stream_thread.is_alive()):
                return
            self._listening = True
        try:
            if self.stt_engine.supports_streaming:
                session = self.stt_engine.start_streaming(sample_rate=self.audio.sample_rate)
                self._stream_queue = Queue()
                self._stream_thread = threading.Thread(
                    target=self._run_streaming_session,
                    args=(session, self._stream_queue),
                    daemon=True,
                )
                self._stream_thread.start()
            self._set_status("Listening...")
            self.audio.start()
        except Exception:
            with self._lock:
                self._listening = False
            if self._stream_queue is not None:
                self._stream_queue.put(None)
            self._stream_queue = None
            self._stream_thread = None
            self._set_status("Idle")
            raise

    def stop_listening(self):
        with self._lock:
            if not self._listening:
                return
            self._listening = False
        self.audio.stop()
        audio = self.audio.get_audio()
        if audio.size == 0:
            if self._stream_queue is not None:
                self._stream_queue.put(None)
            self._set_status("Idle")
            return
        self._last_audio = audio.copy()
        if self._stream_queue is not None:
            self._set_status("Transcribing...")
            self._stream_queue.put(None)
        else:
            threading.Thread(target=self._transcribe_and_output, args=(audio,), daemon=True).start()

    def _on_audio_chunk(self, audio: np.ndarray):
        queue = self._stream_queue
        if queue is not None:
            queue.put(audio)

    def _run_streaming_session(self, session, queue: Queue[np.ndarray | None]):
        started = time.perf_counter()
        live_typing = None
        if (
            self.cfg.live_typing
            and self.cfg.mode == "toggle"
            and self.integration.output_mode == "type"
        ):
            live_typing = LiveTypingSession(self.integration)
        try:
            while True:
                chunk = queue.get()
                if chunk is None:
                    break
                partial_text = session.push(chunk)
                if live_typing and partial_text:
                    live_typing.update(self.postprocessor.process(partial_text).final_text)
            result = session.finish()
            if live_typing:
                final_text = self.postprocessor.process(result.final_text).final_text
                live_typing.update(final_text)
                self._log_result(final_text, started, "Streaming transcription")
            else:
                self._output_result(result.final_text, started, "Streaming transcription")
        except Exception as exc:  # noqa: BLE001
            logger.error("Streaming transcription failed: %s", exc)
        finally:
            self._stream_queue = None
            self._stream_thread = None
            self._set_status("Idle")

    def _on_silence_timeout(self):
        if self._listening:
            self.stop_listening()

    def _transcribe_and_output(self, audio):
        self._set_status("Transcribing...")
        started = time.perf_counter()
        try:
            result = self.stt_engine.transcribe(audio, sample_rate=self.audio.sample_rate)
            self._output_result(result.final_text, started, "Transcription")
        except Exception as exc:  # noqa: BLE001
            logger.error("Transcription failed: %s", exc)
        finally:
            self._set_status("Idle")

    def _output_result(self, text: str, started: float, label: str):
        processed = self.postprocessor.process(text)
        if processed.final_text:
            self.integration.output_text(processed.final_text)
        self._log_result(processed.final_text, started, label)

    def _log_result(self, text: str, started: float, label: str):
        elapsed = time.perf_counter() - started
        logger.info("%s took %.2fs", label, elapsed)
        if self.cfg.log_transcripts:
            logger.info("Transcript: %s", text)

    def replay_last_recording(self):
        if self._last_audio is None or self._last_audio.size == 0:
            logger.info("No recording to replay yet.")
            return
        logger.info("Replaying last recording.")
        sd.play(self._last_audio, samplerate=self.audio.sample_rate)
        sd.wait()

    def run(self):
        logger.info("Starting Flow STT. Hotkey=%s, mode=%s", self.cfg.hotkey, self.cfg.mode)
        if self.ui:
            self.ui.start()
        self._register_hotkeys()
        self._set_status("Idle")
        try:
            while True:
                time.sleep(0.5)
        except KeyboardInterrupt:
            logger.info("Exiting.")


def main():
    app = DictationApp()
    app.run()


if __name__ == "__main__":
    main()
