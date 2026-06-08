import unittest
from unittest.mock import Mock

import numpy as np

from flow_stt.stt_engine import SpeechToTextEngine
from flow_stt.stt_engine import NemotronStreamingSession


class FakeProcessor:
    def __init__(self, model):
        self.chunks = []
        self.options = {}

    def set_option(self, key, value):
        self.options[key] = value

    def process(self, chunk):
        self.chunks.append(chunk.copy())
        return f"chunk-{len(self.chunks)}"

    def flush(self):
        return "flush"


class FakeGenerator:
    def __init__(self, model, params):
        self.inputs = []
        self.runtime_options = {}
        self.pending = False

    def set_runtime_option(self, key, value):
        self.runtime_options[key] = value

    def set_inputs(self, inputs):
        self.inputs.append(inputs)
        self.pending = True

    def is_done(self):
        return not self.pending

    def generate_next_token(self):
        self.pending = False

    def get_next_tokens(self):
        return [1]


class FakeTokenizerStream:
    def decode(self, token):
        return "text "


class FakeTokenizer:
    def __init__(self, model):
        pass

    def create_stream(self):
        return FakeTokenizerStream()


class FakeOg:
    StreamingProcessor = FakeProcessor
    Tokenizer = FakeTokenizer
    Generator = FakeGenerator

    class GeneratorParams:
        def __init__(self, model):
            pass


class NemotronEngineTests(unittest.TestCase):
    def make_engine(self):
        engine = SpeechToTextEngine.__new__(SpeechToTextEngine)
        engine.backend = "nemotron"
        engine.nemotron_target_lang = "auto"
        engine.nemotron_strip_language_tags = True
        engine.model = Mock()
        return engine

    def test_decode_nemotron_tokens_collects_generated_text(self):
        generator = Mock()
        generator.is_done.side_effect = [False, False, True]
        generator.get_next_tokens.side_effect = [[1], [2]]
        stream = Mock()
        stream.decode.side_effect = ["Hello", " world"]

        text = SpeechToTextEngine._decode_nemotron_tokens(generator, stream)

        self.assertEqual(text, "Hello world")

    def test_streaming_session_processes_full_chunks_before_finish(self):
        session = NemotronStreamingSession(Mock(), "auto", True, og_module=FakeOg)

        partial = session.push(np.zeros(session.chunk_samples + 10, dtype=np.float32))

        self.assertEqual(partial, "text")
        self.assertEqual(len(session.processor.chunks), 1)
        result = session.finish()
        self.assertEqual(len(session.processor.chunks), 2)
        self.assertEqual(result.final_text, "text text text")
        self.assertEqual(session.generator.runtime_options["lang_id"], "101")

    def test_streaming_session_strips_language_tags_between_sentences(self):
        session = NemotronStreamingSession(Mock(), "en-US", True, og_module=FakeOg)

        cleaned = session._clean_text(
            "This works. <en-US> Oh wow! <en-US> It is fast. <en-US>"
        )

        self.assertEqual(cleaned, "This works. Oh wow! It is fast.")


if __name__ == "__main__":
    unittest.main()
