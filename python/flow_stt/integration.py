import platform

from .windows_integration import WindowsIntegration
from .pynput_integration import PynputIntegration


def get_integration(output_mode: str, auto_paste_clipboard: bool):
    system = platform.system().lower()
    if system == "windows":
        return WindowsIntegration(output_mode, auto_paste_clipboard)
    return PynputIntegration(output_mode, auto_paste_clipboard)
