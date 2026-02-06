import logging
import threading
import time

import numpy as np
import sounddevice as sd

from .audio_capture import AudioCapture
from .config import ConfigManager
from .postprocess import TextPostProcessor
from .stt_engine import SpeechToTextEngine
from .integration import get_integration
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

        self.postprocessor = TextPostProcessor(enable_spoken_punctuation=self.cfg.spoken_punctuation)
        self.stt_engine = SpeechToTextEngine(
            model_size=self.cfg.model_size,
            language=self.cfg.language,
            prefer_gpu=self.cfg.prefer_gpu,
        )
        self.integration = get_integration(self.cfg.output_mode, self.cfg.auto_paste_clipboard)
        self.audio = AudioCapture(
            device=self.cfg.mic_device,
            sample_rate=16000,
            silence_timeout=self.cfg.silence_timeout_secs,
            on_silence=self._on_silence_timeout,
        )

        self.ui = StatusUI(on_settings_saved=self._reload_config) if self.cfg.enable_ui else None

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
        self.cfg = self.cfg_manager.load()
        self.postprocessor.enable_spoken_punctuation = self.cfg.spoken_punctuation
        self.integration.output_mode = self.cfg.output_mode
        self.integration.auto_paste_clipboard = self.cfg.auto_paste_clipboard
        self.audio.device = self.cfg.mic_device
        self.audio.silence_timeout = self.cfg.silence_timeout_secs
        if (
            self.stt_engine.model_size != self.cfg.model_size
            or self.stt_engine.language != self.cfg.language
            or self.stt_engine.prefer_gpu != self.cfg.prefer_gpu
        ):
            self.stt_engine = SpeechToTextEngine(
                model_size=self.cfg.model_size,
                language=self.cfg.language,
                prefer_gpu=self.cfg.prefer_gpu,
            )
        self._register_hotkeys()

    def _toggle_listening(self):
        if self._listening:
            self.stop_listening()
        else:
            self.start_listening()

    def start_listening(self):
        with self._lock:
            if self._listening:
                return
            self._listening = True
        self._set_status("Listening...")
        self.audio.start()

    def stop_listening(self):
        with self._lock:
            if not self._listening:
                return
            self._listening = False
        self.audio.stop()
        audio = self.audio.get_audio()
        if audio.size == 0:
            self._set_status("Idle")
            return
        self._last_audio = audio.copy()
        threading.Thread(target=self._transcribe_and_output, args=(audio,), daemon=True).start()

    def _on_silence_timeout(self):
        if self._listening:
            self.stop_listening()

    def _transcribe_and_output(self, audio):
        self._set_status("Transcribing...")
        started = time.perf_counter()
        try:
            result = self.stt_engine.transcribe(audio, sample_rate=self.audio.sample_rate)
            processed = self.postprocessor.process(result.final_text)
            self.integration.output_text(processed.final_text)
            elapsed = time.perf_counter() - started
            logger.info("Transcription took %.2fs", elapsed)
            if self.cfg.log_transcripts:
                logger.info("Transcript: %s", processed.final_text)
        except Exception as exc:  # noqa: BLE001
            logger.error("Transcription failed: %s", exc)
        finally:
            self._set_status("Idle")

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
