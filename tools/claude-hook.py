#!/usr/bin/env python3
"""把 Claude Code 生命周期事件最小化后转发给本机 Vibe Buddy。"""

import json
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hook_filter import requires_user_input  # noqa: E402


ENDPOINT = "http://127.0.0.1:7331/v1/claude-hooks"
ALLOWED_FIELDS = (
    "session_id",
    "prompt_id",
    "hook_event_name",
    "cwd",
    "agent_id",
    "agent_type",
)


def sanitized_payload(source: object) -> dict[str, str]:
    if not isinstance(source, dict):
        return {}
    payload = {
        key: value
        for key in ALLOWED_FIELDS
        if isinstance((value := source.get(key)), str)
    }
    if "session_id" not in payload or "hook_event_name" not in payload:
        return {}
    # 子 agent 的最后一段是写给父会话的报告，不是向用户提问，因此只判定主会话。
    if payload["hook_event_name"] == "Stop" and requires_user_input(
        source.get("last_assistant_message")
    ):
        payload["response_kind"] = "input_required"
    return payload


def main() -> int:
    try:
        source = json.load(sys.stdin)
        payload = sanitized_payload(source)
        if not payload:
            return 0
        request = urllib.request.Request(
            ENDPOINT,
            data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(request, timeout=0.5):
            pass
    except (OSError, ValueError):
        # Vibe Buddy 未运行或输入无效时，不能干扰 Claude Code 主流程。
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
