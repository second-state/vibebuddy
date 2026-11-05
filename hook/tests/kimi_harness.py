"""Shared by the Kimi smoke tests: a scripted model that stands in for Anthropic's API, the Kimi config that points
at it, and an ACP client. Nothing here reaches a real model or reads anyone's configuration: Kimi runs with
KIMI_CODE_HOME in a temporary directory and the config has a local API key.
"""
import http.server
import json
import queue
import subprocess
import threading

WAIT_SECONDS = 15
# Every prompt the tests send. None may appear in anything the hook posts.
PROMPTS = [
    "Reply with OK.", "Run pwd and then reply OK.", "Reply OK.", "Run pwd and reply OK.", "Wait for cancellation.",
]
CONFIG = '''default_model = "smoke"
auto_session_title = false
[providers.smoke]
type = "anthropic"
api_key = "local-test-only"
base_url = "http://127.0.0.1:{port}"
[models.smoke]
provider = "smoke"
model = "smoke"
max_context_size = 100000
max_output_size = 1000
'''
# Set when an `interrupt` request has arrived; the test sets `release` to let it finish.
held = threading.Event()
release = threading.Event()


class Model(http.server.BaseHTTPRequestHandler):
    """The scripted model. Its mode decides the answer: `text` says OK; `tool` asks for one `pwd`, then says OK;
    `failure` returns an API error; `interrupt` holds the request until `release` is set."""

    mode = "text"
    tool_sent = False
    tool_results = {}  # tool_use_id -> the tool_result block the model was sent
    lock = threading.Lock()

    @classmethod
    def begin(cls, mode):
        with cls.lock:
            cls.mode = mode
            cls.tool_sent = False
            cls.tool_results = {}

    @classmethod
    def pwd_result(cls):
        with cls.lock:
            return cls.tool_results.get("tool_smoke")

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with Model.lock:
            for message in request.get("messages", []):
                content = message.get("content")
                for block in content if isinstance(content, list) else []:
                    if isinstance(block, dict) and block.get("type") == "tool_result":
                        text = block.get("content", "")
                        if isinstance(text, list):
                            text = "".join(part.get("text", "") for part in text if isinstance(part, dict))
                        Model.tool_results[block.get("tool_use_id")] = {
                            "is_error": bool(block.get("is_error")), "text": str(text),
                        }
            mode = Model.mode
        if mode == "interrupt":
            held.set()
            release.wait(timeout=WAIT_SECONDS)
        if mode == "failure":
            body = json.dumps({"type": "error", "error": {"type": "invalid_request_error", "message": "scripted model failure"}}).encode()
            self.send_response(400)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        message = {"id": "msg_smoke", "type": "message", "role": "assistant",
                   "model": "smoke", "content": [{"type": "text", "text": "OK"}],
                   "stop_reason": "end_turn", "stop_sequence": None,
                   "usage": {"input_tokens": 10, "output_tokens": 1}}
        tool = mode == "tool" and not Model.tool_sent
        if tool:
            assert any(item["name"] == "Bash" for item in request.get("tools", [])), "Bash is not offered to the model"
            Model.tool_sent = True
            message["content"] = [{"type": "tool_use", "id": "tool_smoke", "name": "Bash", "input": {"command": "pwd"}}]
            message["stop_reason"] = "tool_use"
        if request.get("stream"):
            frames = [
                {"type": "message_start", "message": {**message, "content": [], "stop_reason": None}},
                {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}},
                {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "OK"}},
                {"type": "content_block_stop", "index": 0},
                {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": None}, "usage": {"output_tokens": 1}},
                {"type": "message_stop"},
            ]
            if tool:
                frames[1] = {"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "id": "tool_smoke", "name": "Bash", "input": {}}}
                frames[2] = {"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": json.dumps({"command": "pwd"})}}
                frames[4]["delta"]["stop_reason"] = "tool_use"
            body = "".join(f"event: {frame['type']}\ndata: {json.dumps(frame)}\n\n" for frame in frames).encode()
            content_type = "text/event-stream"
        else:
            body = json.dumps(message).encode()
            content_type = "application/json"
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # Expected when a cancelled prompt aborts an in-flight response.

    def log_message(self, *_):
        pass


def start_model():
    """The scripted model on a free loopback port; the caller shuts it down."""
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Model)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class Acp:
    """A `kimi acp` client over JSON-RPC on stdio. `approve(params)` picks the answer to each permission request;
    by default the first request is approved once."""

    def __init__(self, kimi, cwd, env):
        self.process = subprocess.Popen([kimi, "acp"], cwd=cwd, env=env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.messages = queue.Queue()
        self.approvals = []
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.process.stdout:
            self.messages.put(json.loads(line))

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()

    def receive(self, identifier, approve=lambda params: "approve_once"):
        while True:
            message = self.messages.get(timeout=2 * WAIT_SECONDS)
            if message.get("method") == "session/request_permission":
                self.approvals.append(message["params"])
                option = approve(message["params"])
                self.send({"jsonrpc": "2.0", "id": message["id"],
                           "result": {"outcome": {"outcome": "selected", "optionId": option}}})
            elif message.get("id") == identifier and "method" not in message:
                assert "error" not in message, message
                return message["result"]

    def rpc(self, identifier, method, params):
        self.send({"jsonrpc": "2.0", "id": identifier, "method": method, "params": params})
        return self.receive(identifier)

    def close(self):
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()
