import logging
import platform
import time
from typing import Callable, Dict, Iterable, Set

import pyperclip
from pynput import keyboard


logger = logging.getLogger(__name__)


MOD_MAP = {
    "ctrl": "ctrl",
    "control": "ctrl",
    "cmd": "cmd",
    "command": "cmd",
    "alt": "alt",
    "option": "alt",
    "shift": "shift",
    "space": "space",
}


def _canonical_token(token: str) -> str:
    return MOD_MAP.get(token.lower(), token.lower())


class PynputIntegration:
    """Cross-platform hotkeys and typing using pynput (macOS/Linux/Windows)."""

    def __init__(self, output_mode: str = "type", auto_paste_clipboard: bool = False):
        self.output_mode = output_mode
        self.auto_paste_clipboard = auto_paste_clipboard
        self._listeners: list[keyboard.Listener] = []
        self._hotkeys: list[keyboard.GlobalHotKeys] = []
        self._pressed: Set[str] = set()
        self._active_ptt = False
        self._system = platform.system().lower()
        self._controller = keyboard.Controller()

    def type_text(self, text: str, chunk_size: int = 120) -> None:
        if not text:
            return
        for idx in range(0, len(text), chunk_size):
            chunk = text[idx : idx + chunk_size]
            self._controller.type(chunk)
            time.sleep(0.02)

    def copy_to_clipboard(self, text: str, paste: bool | None = None) -> None:
        if not text:
            return
        pyperclip.copy(text)
        do_paste = self.auto_paste_clipboard if paste is None else paste
        if do_paste:
            time.sleep(0.05)
            self._send_paste()

    def _send_paste(self):
        key_cmd = keyboard.Key.cmd if self._system == "darwin" else keyboard.Key.ctrl
        with self._controller.pressed(key_cmd):
            self._controller.press("v")
            self._controller.release("v")

    def output_text(self, text: str) -> None:
        if self.output_mode == "clipboard":
            self.copy_to_clipboard(text)
        elif self.output_mode == "paste":
            self.copy_to_clipboard(text, paste=True)
        else:
            self.type_text(text)

    # Hotkeys
    def register_hotkey_toggle(self, hotkey: str, on_toggle: Callable[[], None]) -> None:
        pattern = self._to_pynput_hotkey(hotkey)
        logger.info("Registering toggle hotkey (pynput): %s", pattern)
        ghk = keyboard.GlobalHotKeys({pattern: on_toggle})
        ghk.start()
        self._hotkeys.append(ghk)

    def register_hotkey_push_to_talk(
        self, hotkey: str, on_press: Callable[[], None], on_release: Callable[[], None]
    ) -> None:
        combo = set(self._parse_hotkey_tokens(hotkey))
        logger.info("Registering push-to-talk hotkey (pynput): %s", "+".join(combo))

        def _on_press(key):
            name = self._key_name(key)
            if name:
                self._pressed.add(name)
            if combo.issubset(self._pressed) and not self._active_ptt:
                self._active_ptt = True
                on_press()

        def _on_release(key):
            name = self._key_name(key)
            if name and name in self._pressed:
                self._pressed.discard(name)
            if self._active_ptt and (name in combo):
                self._active_ptt = False
                on_release()

        listener = keyboard.Listener(on_press=_on_press, on_release=_on_release)
        listener.start()
        self._listeners.append(listener)

    def clear_hotkeys(self) -> None:
        for ghk in self._hotkeys:
            try:
                ghk.stop()
            except Exception:  # noqa: BLE001
                pass
        self._hotkeys = []
        for listener in self._listeners:
            try:
                listener.stop()
            except Exception:  # noqa: BLE001
                pass
        self._listeners = []
        self._pressed.clear()
        self._active_ptt = False

    def register_hotkey_action(self, hotkey: str, action: Callable[[], None]) -> None:
        # Simple alias to toggle registration for single-action hotkeys.
        self.register_hotkey_toggle(hotkey, action)

    # Helpers
    def _parse_hotkey_tokens(self, hotkey: str) -> Iterable[str]:
        tokens = [t.strip() for t in hotkey.replace("+", " ").split()]
        return [_canonical_token(t) for t in tokens if t]

    def _to_pynput_hotkey(self, hotkey: str) -> str:
        parts = [f"<{_canonical_token(t)}>" for t in hotkey.split("+")]
        return "+".join(parts)

    def _key_name(self, key) -> str | None:
        try:
            if isinstance(key, keyboard.Key):
                return _canonical_token(key.name or "")
            if isinstance(key, keyboard.KeyCode):
                return _canonical_token(key.char or "")
        except Exception:  # noqa: BLE001
            return None
        return None
