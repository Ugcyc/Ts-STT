import logging
import time
from typing import Callable, List

import keyboard
import pyperclip


logger = logging.getLogger(__name__)


class WindowsIntegration:
    def __init__(self, output_mode: str = "type", auto_paste_clipboard: bool = False):
        self.output_mode = output_mode
        self.auto_paste_clipboard = auto_paste_clipboard
        self._hotkeys: List[Callable[[], None]] = []

    def type_text(self, text: str, chunk_size: int = 120) -> None:
        """Send keystrokes into the currently focused window."""
        if not text:
            return
        for idx in range(0, len(text), chunk_size):
            chunk = text[idx : idx + chunk_size]
            keyboard.write(chunk, delay=0.01)
            time.sleep(0.02)

    def copy_to_clipboard(self, text: str, paste: bool | None = None) -> None:
        if not text:
            return
        pyperclip.copy(text)
        do_paste = self.auto_paste_clipboard if paste is None else paste
        if do_paste:
            time.sleep(0.05)  # Give the clipboard a moment to update before pasting.
            keyboard.send("ctrl+v")

    def output_text(self, text: str) -> None:
        if self.output_mode == "clipboard":
            self.copy_to_clipboard(text)
        elif self.output_mode == "paste":
            self.copy_to_clipboard(text, paste=True)
        else:
            self.type_text(text)

    def register_hotkey_push_to_talk(
        self, hotkey: str, on_press: Callable[[], None], on_release: Callable[[], None]
    ) -> None:
        logger.info("Registering push-to-talk hotkey: %s", hotkey)
        handle_press = keyboard.add_hotkey(hotkey, on_press, suppress=False, trigger_on_release=False)
        handle_release = keyboard.add_hotkey(hotkey, on_release, suppress=False, trigger_on_release=True)
        self._hotkeys.append(handle_press)
        self._hotkeys.append(handle_release)

    def register_hotkey_toggle(self, hotkey: str, on_toggle: Callable[[], None]) -> None:
        logger.info("Registering toggle hotkey: %s", hotkey)
        handle = keyboard.add_hotkey(hotkey, on_toggle, suppress=False)
        self._hotkeys.append(handle)

    def register_hotkey_action(self, hotkey: str, action: Callable[[], None]) -> None:
        logger.info("Registering action hotkey: %s", hotkey)
        handle = keyboard.add_hotkey(hotkey, action, suppress=False)
        self._hotkeys.append(handle)

    def clear_hotkeys(self) -> None:
        for handle in self._hotkeys:
            try:
                keyboard.remove_hotkey(handle)
            except KeyError:
                continue
        self._hotkeys = []
