import logging
import time
from queue import Queue
from threading import Event, Thread
from typing import Callable, List, Optional

import numpy as np
import sounddevice as sd


logger = logging.getLogger(__name__)


def list_input_devices() -> List[str]:
    devices = []
    for idx, device in enumerate(sd.query_devices()):
        if device.get("max_input_channels", 0) > 0:
            devices.append(f"{idx}: {device['name']}")
    return devices


class AudioCapture:
    def __init__(
        self,
        device: Optional[str] = None,
        sample_rate: int = 16000,
        channels: int = 1,
        block_size: int = 2048,
        silence_timeout: Optional[float] = 60.0,
        silence_threshold: float = 0.015,
        on_silence: Optional[Callable[[], None]] = None,
    ):
        self.device = device
        self.sample_rate = sample_rate
        self.channels = channels
        self.block_size = block_size
        self.silence_timeout = silence_timeout
        self.silence_threshold = silence_threshold
        self.on_silence = on_silence

        self._stream: Optional[sd.InputStream] = None
        self._queue: Queue[np.ndarray] = Queue()
        self._stop_event = Event()
        self._listening = False
        self._last_voice_time = time.time()

    def start(self) -> None:
        if self._listening:
            return
        self._queue = Queue()
        self._stop_event.clear()
        self._last_voice_time = time.time()
        self._stream = sd.InputStream(
            samplerate=self.sample_rate,
            channels=self.channels,
            blocksize=self.block_size,
            dtype="float32",
            device=self._resolve_device(),
            callback=self._callback,
        )
        self._stream.start()
        self._listening = True
        if self.silence_timeout:
            Thread(target=self._silence_watchdog, daemon=True).start()

    def stop(self) -> None:
        if not self._listening:
            return
        self._stop_event.set()
        if self._stream:
            try:
                self._stream.stop()
                self._stream.close()
            except Exception as exc:  # noqa: BLE001
                logger.warning("Failed to close audio stream: %s", exc)
        self._stream = None
        self._listening = False

    def is_listening(self) -> bool:
        return self._listening

    def get_audio(self) -> np.ndarray:
        frames = []
        while not self._queue.empty():
            frames.append(self._queue.get())
        if not frames:
            return np.zeros((0, self.channels), dtype=np.float32)
        return np.concatenate(frames, axis=0)

    def record_blocking(self, seconds: float) -> np.ndarray:
        self.start()
        time.sleep(seconds)
        self.stop()
        return self.get_audio()

    def _resolve_device(self):
        if self.device in (None, "", "Default"):
            return None
        if isinstance(self.device, str) and ":" in self.device:
            idx, _name = self.device.split(":", 1)
            if idx.strip().isdigit():
                return int(idx.strip())
        return self.device

    def _callback(self, indata, frames, time_info, status):
        if status:
            logger.debug("Audio stream status: %s", status)
        self._queue.put(indata.copy())
        rms = float(np.sqrt(np.mean(np.square(indata))))
        if rms > self.silence_threshold:
            self._last_voice_time = time.time()

    def _silence_watchdog(self):
        while not self._stop_event.is_set():
            if self.silence_timeout is None:
                return
            if time.time() - self._last_voice_time > self.silence_timeout:
                logger.debug("Silence timeout reached; stopping capture.")
                self.stop()
                if self.on_silence:
                    self.on_silence()
                return
            time.sleep(0.1)
