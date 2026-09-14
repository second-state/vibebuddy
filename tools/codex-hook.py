#!/usr/bin/env python3
"""把 Codex 生命周期事件最小化后转发给本机 AgentBeacon。"""

import json
import sys
import urllib.request


ENDPOINT = "http://127.0.0.1:7331/v1/codex-hooks"
ALLOWED_FIELDS = ("session_id", "turn_id", "hook_event_name", "cwd")


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
