#!/usr/bin/env python3
"""把 Codex 生命周期事件最小化后转发给本机 AgentBeacon。"""

import json
import re
import sys
import urllib.request


ENDPOINT = "http://127.0.0.1:7331/v1/codex-hooks"
ALLOWED_FIELDS = ("session_id", "turn_id", "hook_event_name", "cwd")
QUESTION_AT_END = re.compile(r"[?？][\s*_`'\"”’。.!！]*$")
QUESTION_THEN_REPLY = re.compile(
    r"[?？].{0,100}(?:回复|回答|确认|选择|告诉|reply|respond|confirm|choose)",
    re.IGNORECASE | re.DOTALL,
)
DIRECT_REPLY_REQUEST = re.compile(
    r"(?:^|[。.!！]\s*)(?:(?:请(?:你)?(?:直接)?)|直接)"
    r"(?:回复|回答|确认|选择|告诉)|"
    r"(?:^|[.!]\s*)(?:please\s+)?(?:reply|respond|confirm|choose)\b|"
    r"\blet me know\b",
    re.IGNORECASE,
)
CHOICE_LIST = re.compile(r"^\s*(?:[-*]|\d+[.)])\s+", re.MULTILINE)
OPTIONAL_OFFER = re.compile(
    r"(?:^|[。.!！]\s*)(?:如果|若|如需|if\b).{0,120}"
    r"(?:回复|回答|告诉|reply|respond|let me know)",
    re.IGNORECASE | re.DOTALL,
)


def requires_user_input(message: object) -> bool:
    if not isinstance(message, str) or not message.strip():
        return False

    paragraphs = [
        paragraph.strip()
        for paragraph in re.split(r"\n\s*\n", message.strip())
        if paragraph.strip()
    ]
    final = paragraphs[-1]
    if QUESTION_AT_END.search(final) or QUESTION_THEN_REPLY.search(final):
        return True
    if DIRECT_REPLY_REQUEST.search(final):
        return OPTIONAL_OFFER.search(final) is None
    if len(paragraphs) >= 2 and CHOICE_LIST.search(final):
        return QUESTION_AT_END.search(paragraphs[-2]) is not None
    return False


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
        # AgentBeacon 未运行或输入无效时，不能干扰 Codex 主流程。
        pass
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
