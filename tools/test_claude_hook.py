import importlib.util
from pathlib import Path
import sys
import unittest


sys.dont_write_bytecode = True
MODULE_PATH = Path(__file__).with_name("claude-hook.py")
SPEC = importlib.util.spec_from_file_location("claude_hook", MODULE_PATH)
CLAUDE_HOOK = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(CLAUDE_HOOK)


class SanitizedPayloadTests(unittest.TestCase):
    def test_prompt_and_transcript_never_leave_the_machine(self) -> None:
        payload = CLAUDE_HOOK.sanitized_payload(
            {
                "session_id": "s1",
                "prompt_id": "p1",
                "hook_event_name": "UserPromptSubmit",
                "cwd": "/work/agent-beacon",
                "prompt": "请把我的密钥改成 sk-secret",
                "transcript_path": "/Users/someone/.claude/projects/x.jsonl",
                "session_title": "内部项目代号",
            }
        )
        self.assertEqual(
            payload,
            {
                "session_id": "s1",
                "prompt_id": "p1",
                "hook_event_name": "UserPromptSubmit",
                "cwd": "/work/agent-beacon",
            },
        )

    def test_subagent_identity_is_preserved(self) -> None:
        payload = CLAUDE_HOOK.sanitized_payload(
            {
                "session_id": "s1",
                "prompt_id": "p1",
                "hook_event_name": "SubagentStart",
                "agent_id": "a1",
                "agent_type": "Explore",
            }
        )
        self.assertEqual(payload["agent_id"], "a1")
        self.assertEqual(payload["agent_type"], "Explore")

    def test_question_becomes_a_derived_flag_without_the_message(self) -> None:
        payload = CLAUDE_HOOK.sanitized_payload(
            {
                "session_id": "s1",
                "prompt_id": "p1",
                "hook_event_name": "Stop",
                "last_assistant_message": "我该用方案 A 还是方案 B？",
            }
        )
        self.assertEqual(payload["response_kind"], "input_required")
        self.assertNotIn("last_assistant_message", payload)

    def test_subagent_report_is_never_classified_as_waiting(self) -> None:
        payload = CLAUDE_HOOK.sanitized_payload(
            {
                "session_id": "s1",
                "prompt_id": "p1",
                "hook_event_name": "SubagentStop",
                "agent_id": "a1",
                "last_assistant_message": "要继续深入排查吗？",
            }
        )
        self.assertNotIn("response_kind", payload)


if __name__ == "__main__":
    unittest.main()
