class LiveTypingSession:
    def __init__(self, integration):
        self.integration = integration
        self.typed_text = ""

    def update(self, text: str) -> None:
        if text == self.typed_text:
            return
        prefix_length = _common_prefix_length(self.typed_text, text)
        remove_count = len(self.typed_text) - prefix_length
        replacement = text[prefix_length:]
        self.integration.replace_typed_suffix(remove_count, replacement)
        self.typed_text = text


def _common_prefix_length(left: str, right: str) -> int:
    limit = min(len(left), len(right))
    index = 0
    while index < limit and left[index] == right[index]:
        index += 1
    return index
