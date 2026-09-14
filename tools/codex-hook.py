#!/usr/bin/env python3
"""把 Codex 生命周期事件最小化后转发给本机 AgentBeacon。"""

import json
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from hook_filter import requires_user_input  # noqa: E402


ENDPOINT = "http://127.0.0.1:7331/v1/codex-hooks"
ALLOWED_FIELDS = ("session_id", "turn_id", "hook_event_name", "cwd")


def navigable_thread_id(source: dict[str, object]) -> str | None:
    """把子 Agent 的生命周期会话映射到桌面端可打开的父会话。"""
    session_id = source.get("session_id")
    if not isinstance(session_id, str):
        return None

    transcript_path = source.get("transcript_path")
    if not isinstance(transcript_path, str):
        return session_id
    try:
        with Path(transcript_path).open(encoding="utf-8") as transcript:
            metadata = json.loads(transcript.readline())
    except (OSError, ValueError):
        return session_id

    payload = metadata.get("payload") if isinstance(metadata, dict) else None
    if not isinstance(payload, dict):
        return session_id
    metadata_id = payload.get("id")
    own_thread_id = metadata_id if isinstance(metadata_id, str) else session_id
    if payload.get("thread_source") != "subagent":
        return own_thread_id

    origin = payload.get("source")
    if not isinstance(origin, dict):
        return own_thread_id
    subagent = origin.get("subagent")
    if not isinstance(subagent, dict):
        return own_thread_id
    spawn = subagent.get("thread_spawn")
    if not isinstance(spawn, dict):
        return own_thread_id
    parent_thread_id = spawn.get("parent_thread_id")
    return parent_thread_id if isinstance(parent_thread_id, str) else own_thread_id


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
    thread_id = navigable_thread_id(source)
    if thread_id is not None:
        payload["thread_id"] = thread_id
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
