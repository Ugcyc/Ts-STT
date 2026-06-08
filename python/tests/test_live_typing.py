import unittest
from unittest.mock import Mock, call

from flow_stt.live_typing import LiveTypingSession


class LiveTypingSessionTests(unittest.TestCase):
    def test_appends_new_text(self):
        integration = Mock()
        session = LiveTypingSession(integration)

        session.update("Hello")
        session.update("Hello world")

        self.assertEqual(
            integration.replace_typed_suffix.call_args_list,
            [call(0, "Hello"), call(0, " world")],
        )

    def test_replaces_revised_suffix(self):
        integration = Mock()
        session = LiveTypingSession(integration)
        session.update("I like cats")

        session.update("I like coding")

        integration.replace_typed_suffix.assert_called_with(3, "oding")


if __name__ == "__main__":
    unittest.main()
