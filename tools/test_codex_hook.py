import importlib.util
from pathlib import Path
import sys
import unittest


sys.dont_write_bytecode = True
MODULE_PATH = Path(__file__).with_name("codex-hook.py")
SPEC = importlib.util.spec_from_file_location("codex_hook", MODULE_PATH)
CODEX_HOOK = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(CODEX_HOOK)


class SanitizedPayloadTests(unittest.TestCase):
    def test_question_is_classified_without_forwarding_message_text(self) -> None:
        source = {
            "session_id": "session-a",
            "turn_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/agent-beacon",
            "last_assistant_message": (
                "第一项推荐使用 GitHub Issues。\n\n"
                "是否采用 GitHub Issues？直接回复“是”即可。"
            ),
        }

        payload = CODEX_HOOK.sanitized_payload(source)

        self.assertEqual(payload.get("response_kind"), "input_required")
        self.assertNotIn("last_assistant_message", payload)

    def test_completed_response_is_not_classified_as_input_required(self) -> None:
        source = {
            "session_id": "session-a",
            "turn_id": "turn-a",
            "hook_event_name": "Stop",
            "cwd": "/work/agent-beacon",
            "last_assistant_message": "修复已完成，全量测试通过。",
        }

        payload = CODEX_HOOK.sanitized_payload(source)

        self.assertNotIn("response_kind", payload)
        self.assertNotIn("last_assistant_message", payload)

    def test_optional_follow_up_offer_is_not_treated_as_required_input(self) -> None:
        for message in (
            "如果你还需要调整配色，可以告诉我。",
            "修复完成。If you need anything else, let me know.",
        ):
            with self.subTest(message=message):
                source = {
                    "session_id": "session-a",
                    "turn_id": "turn-a",
                    "hook_event_name": "Stop",
                    "cwd": "/work/agent-beacon",
                    "last_assistant_message": message,
                }

                payload = CODEX_HOOK.sanitized_payload(source)

                self.assertNotIn("response_kind", payload)


if __name__ == "__main__":
    unittest.main()
