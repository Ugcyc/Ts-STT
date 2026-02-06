import re
from dataclasses import dataclass
from typing import List, Tuple


PUNCTUATION_RULES: List[Tuple[Tuple[str, ...], str]] = [
    (("new", "paragraph"), "\n\n"),
    (("new", "line"), "\n"),
    (("newline",), "\n"),
    (("comma",), ","),
    (("period",), "."),
    (("full", "stop"), "."),
    (("question", "mark"), "?"),
    (("exclamation", "mark"), "!"),
    (("exclamation", "point"), "!"),
]


@dataclass
class TextResult:
    final_text: str
    partial_text: str | None = None


class TextPostProcessor:
    def __init__(self, enable_spoken_punctuation: bool = True):
        self.enable_spoken_punctuation = enable_spoken_punctuation

    def process(self, text: str) -> TextResult:
        if not text:
            return TextResult(final_text="")
        cleaned = text.strip()
        if self.enable_spoken_punctuation:
            cleaned = self._apply_spoken_punctuation(cleaned)
        cleaned = self._clean_spaces(cleaned)
        cleaned = self._capitalize(cleaned)
        return TextResult(final_text=cleaned)

    def _apply_spoken_punctuation(self, text: str) -> str:
        words = text.split()
        output: List[str] = []
        i = 0
        while i < len(words):
            matched = False
            for keys, symbol in PUNCTUATION_RULES:
                if tuple(w.lower() for w in words[i : i + len(keys)]) == keys:
                    output.append(symbol)
                    i += len(keys)
                    matched = True
                    break
            if not matched:
                output.append(words[i])
                i += 1
        return " ".join(output)

    def _clean_spaces(self, text: str) -> str:
        text = re.sub(r"\s+([,\.!?])", r"\1", text)
        text = re.sub(r"\s+\n", "\n", text)
        text = re.sub(r"\n\s+", "\n", text)
        text = re.sub(r"\s{2,}", " ", text)
        return text.strip()

    def _capitalize(self, text: str) -> str:
        parts = re.split(r"([\.!?]\s+|\n+)", text)
        capitalized_parts = []
        for part in parts:
            if not part:
                continue
            if re.match(r"[\.!?]\s+|\n+", part):
                capitalized_parts.append(part)
                continue
            capitalized_parts.append(part[:1].upper() + part[1:] if part else part)
        return "".join(capitalized_parts).strip()
