import json
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Optional

# Configuration defaults keep the out-of-box experience simple.
DEFAULT_CONFIG = {
    "hotkey": "ctrl+shift+space",
    "mode": "push_to_talk",  # or "toggle"
    "output_mode": "type",  # or "clipboard" or "paste"
    "mic_device": None,
    "model_size": "small",
    "language": "en",
    "spoken_punctuation": True,
    "auto_paste_clipboard": False,
    "silence_timeout_secs": 60.0,  # Stop after long silence; hotkey release still stops immediately.
    "enable_ui": True,
    "log_transcripts": False,
    "prefer_gpu": True,
    "replay_hotkey": "ctrl+alt+r",
}


def _default_config_path() -> Path:
    base = Path.home() / "AppData" / "Local" / "flow_stt"
    base.mkdir(parents=True, exist_ok=True)
    return base / "config.json"


@dataclass
class Config:
    hotkey: str
    mode: str
    output_mode: str
    mic_device: Optional[str]
    model_size: str
    language: str
    spoken_punctuation: bool
    auto_paste_clipboard: bool
    silence_timeout_secs: Optional[float]
    enable_ui: bool
    log_transcripts: bool
    prefer_gpu: bool
    replay_hotkey: str
    path: Path

    @classmethod
    def from_dict(cls, data: dict, path: Path) -> "Config":
        merged = DEFAULT_CONFIG.copy()
        merged.update({k: v for k, v in data.items() if v is not None})
        return cls(path=path, **merged)

    def to_dict(self) -> dict:
        data = asdict(self).copy()
        data.pop("path", None)
        return data


class ConfigManager:
    def __init__(self, path: Optional[Path] = None):
        self.path = path or _default_config_path()
        self.config = self.load()

    def load(self) -> Config:
        if self.path.exists():
            try:
                with self.path.open("r", encoding="utf-8") as f:
                    data = json.load(f)
            except (json.JSONDecodeError, OSError):
                data = DEFAULT_CONFIG.copy()
        else:
            data = DEFAULT_CONFIG.copy()
            self.save(Config.from_dict(data, self.path))
        # Migrate old default (1.8s) to the new longer timeout.
        if data.get("silence_timeout_secs") == 1.8:
            data["silence_timeout_secs"] = DEFAULT_CONFIG["silence_timeout_secs"]
        return Config.from_dict(data, self.path)

    def save(self, config: Optional[Config] = None) -> None:
        cfg = config or self.config
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self.path.open("w", encoding="utf-8") as f:
            json.dump(cfg.to_dict(), f, indent=2)
        self.config = cfg

    def update(self, **kwargs) -> Config:
        data = self.config.to_dict()
        data.update(kwargs)
        self.config = Config.from_dict(data, self.path)
        self.save(self.config)
        return self.config
