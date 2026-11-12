#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["noiseprotocol>=0.3.1", "cryptography>=42"]
# ///
"""Play a VibePlay on this computer (docs/vibeplay.md), to test the daemon and the apps without the handheld.

    uv run tools/simulate-vibeplay.py                  # listens on port 7342 and shows a pairing code
    VIBEBUDDY_VIBEPLAY_ADDR=127.0.0.1:7342 vibebuddyd  # the daemon talks to it
    uv run tools/simulate-vibeplay.py --mdns           # also answers mDNS (dns-sd on macOS, avahi-publish on Linux)

It pairs with Noise XX when the right code comes in, links with Noise IK, and prints every line it gets.
Type `a` and Enter to press A (K2), `f <task id>` to pick a card, `p` to show a new pairing code, `q` to quit.
`--state FILE` keeps its key and its paired computers between runs; without it, each run is a new VibePlay.
"""

import argparse
import base64
import hashlib
import json
import os
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from noise.connection import Keypair, NoiseConnection

PAIR_PROLOGUE = b"vibebuddy-vibeplay-pair-v1"
LINK_PROLOGUE = b"vibebuddy-vibeplay-v1"
MAX_COMPUTERS = 4
CODE_TRIES = 3
CODE_LIFETIME = 600
SILENCE = 15
HANDSHAKE_TIMEOUT = 10


def b64(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def unb64(text):
    text = text.strip()
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def line(event, **fields):
    return json.dumps({"version": 1, "event": event, **fields}, separators=(",", ":"))


class VibePlay:
    def __init__(self, state_path, name):
        self.state_path = state_path
        self.name = name
        state = {}
        if state_path and os.path.exists(state_path):
            with open(state_path) as file:
                state = json.load(file)
        self.seed = unb64(state["seed"]) if "seed" in state else secrets.token_bytes(32)
        # computers: [{"key": X25519 static as Noise saw it, "name": ...}]
        self.computers = state.get("computers", [])
        self.save()
        public = Ed25519PrivateKey.from_private_bytes(self.seed).public_key().public_bytes_raw()
        self.key = b64(public)
        # The Ed25519 key as an X25519 secret, as the daemon and the box do: the first half of SHA-512 of the seed.
        self.noise_secret = hashlib.sha512(self.seed).digest()[:32]
        self.lock = threading.Lock()
        self.owner = None  # the computer on the link: (socket, session, name)
        self.code = None
        self.code_until = 0
        self.wrong = 0
        if not self.computers:
            self.new_code()

    def save(self):
        if not self.state_path:
            return
        with open(self.state_path, "w") as file:
            json.dump({"seed": b64(self.seed), "computers": self.computers}, file)
        os.chmod(self.state_path, 0o600)

    def new_code(self):
        self.code = f"{secrets.randbelow(1_000_000):06d}"
        self.code_until = time.monotonic() + CODE_LIFETIME
        self.wrong = 0
        print(f"\n  Pairing code: {self.code}  (open VibeBuddy on your computer and pick this VibePlay)\n", flush=True)

    def showing_code(self):
        if self.code and time.monotonic() > self.code_until:
            print("the pairing code ran out", flush=True)
            self.new_code()
        return self.code is not None

    def noise(self, pattern, prologue):
        noise = NoiseConnection.from_name(pattern)
        noise.set_as_responder()
        noise.set_keypair_from_private_bytes(Keypair.STATIC, self.noise_secret)
        noise.set_prologue(prologue)
        noise.start_handshake()
        return noise

    def serve(self, connection, peer):
        connection.settimeout(HANDSHAKE_TIMEOUT)
        reader = connection.makefile("rb")
        send = lambda text: connection.sendall(text.encode() + b"\n")
        try:
            send(line("link.noise", vibeplay=self.key))
            first = reader.readline()
            if not first:
                return  # the daemon only asked who is here
            first = first.decode().strip()
            if first.startswith("{"):
                if json.loads(first).get("event") == "vibeplay.pair_start":
                    self.pair(reader, send, peer)
                else:
                    send(line("link.error", code="bad_message"))
                return
            self.link(connection, reader, send, first, peer)
        except (OSError, ValueError, Exception) as error:  # noqa: BLE001 - a test tool reports and goes on
            print(f"[{peer}] connection ended: {error}", flush=True)
        finally:
            connection.close()

    def pair(self, reader, send, peer):
        noise = self.noise(b"Noise_XX_25519_ChaChaPoly_SHA256", PAIR_PROLOGUE)
        noise.read_message(unb64(reader.readline().decode()))
        send(b64(noise.write_message()))
        handshake = noise.noise_protocol.handshake_state  # kept: finishing the handshake drops it
        noise.read_message(unb64(reader.readline().decode()))
        computer = b64(handshake.rs.public_bytes)
        request = json.loads(noise.decrypt(unb64(reader.readline().decode())))
        with self.lock:
            if request.get("event") != "vibeplay.pair":
                answer = line("link.error", code="bad_message")
            elif not self.showing_code():
                answer = line("vibeplay.pair_error", reason="closed")
            elif request.get("code") != self.code:
                self.wrong += 1
                print(f"[{peer}] wrong code ({self.wrong} of {CODE_TRIES})", flush=True)
                answer = line("vibeplay.pair_error", reason="code")
                if self.wrong >= CODE_TRIES:
                    self.new_code()
            elif len(self.computers) >= MAX_COMPUTERS and not any(c["key"] == computer for c in self.computers):
                answer = line("vibeplay.pair_error", reason="full")
            else:
                self.computers = [c for c in self.computers if c["key"] != computer]
                self.computers.append({"key": computer, "name": request.get("name", "")})
                self.save()
                self.code = None
                print(f"[{peer}] paired with {request.get('name')!r}", flush=True)
                answer = line("vibeplay.paired")
        send(b64(noise.encrypt(answer.encode())))

    def link(self, connection, reader, send, first, peer):
        noise = self.noise(b"Noise_IK_25519_ChaChaPoly_SHA256", LINK_PROLOGUE)
        noise.read_message(unb64(first))
        computer = b64(noise.noise_protocol.handshake_state.rs.public_bytes)
        known = next((c for c in self.computers if c["key"] == computer), None)
        if known is None:
            print(f"[{peer}] a computer that isn't paired", flush=True)
            send(line("link.error", code="auth"))
            return
        send(b64(noise.write_message()))
        seal = lambda text: send(b64(noise.encrypt(text.encode())))
        with self.lock:
            if self.owner is not None:
                seal(line("link.busy", reason="linked", owner=self.owner[2]))
                return
            self.owner = (connection, seal, known["name"])
        print(f"[{peer}] linked to {known['name']!r}", flush=True)
        try:
            seal(line("vibeplay.hello", build="simulator", name=self.name, screen=[720, 480]))
            connection.settimeout(SILENCE)
            while True:
                sealed = reader.readline()
                if not sealed:
                    break
                text = noise.decrypt(unb64(sealed.decode())).decode()
                print(f"< {text}", flush=True)
                if json.loads(text).get("event") == "vibeplay.unpair":
                    with self.lock:
                        self.computers = [c for c in self.computers if c["key"] != computer]
                        self.save()
                        if not self.computers:
                            self.new_code()
                    print("the computer forgot this VibePlay", flush=True)
                    break
        except socket.timeout:
            print(f"[{peer}] the computer was silent for {SILENCE} s: gone", flush=True)
        finally:
            with self.lock:
                self.owner = None
            print(f"[{peer}] link ended", flush=True)

    def press(self, text):
        with self.lock:
            owner = self.owner
        if owner is None:
            print("no computer is linked", flush=True)
            return
        owner[1](text)
        print(f"> {text}", flush=True)


def advertise(vibeplay, port):
    """Answers mDNS with the system's own tool, as QingOS will for the app."""
    instance = f"VibePlay {vibeplay.key[:8]}"
    txt = [f"key={vibeplay.key}", f"name={vibeplay.name}", f"paired={1 if vibeplay.computers else 0}"]
    if shutil.which("dns-sd"):
        command = ["dns-sd", "-R", instance, "_vibebuddy-vibeplay._tcp", "local", str(port), *txt]
    elif shutil.which("avahi-publish"):
        command = ["avahi-publish", "-s", instance, "_vibebuddy-vibeplay._tcp", str(port), *txt]
    else:
        sys.exit("--mdns needs dns-sd (macOS) or avahi-publish (Linux)")
    return subprocess.Popen(command, stdout=subprocess.DEVNULL)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--port", type=int, default=7342)
    parser.add_argument("--bind", default="0.0.0.0")
    parser.add_argument("--name", default="Simulated VibePlay")
    parser.add_argument("--state", help="keep the key and paired computers in this file")
    parser.add_argument("--mdns", action="store_true", help="answer mDNS for _vibebuddy-vibeplay._tcp")
    args = parser.parse_args()

    vibeplay = VibePlay(args.state, args.name)
    server = socket.create_server((args.bind, args.port), reuse_port=False)
    print(f"VibePlay {vibeplay.key[:8]} listening on {args.bind}:{args.port}", flush=True)
    advertiser = advertise(vibeplay, args.port) if args.mdns else None

    def accept():
        while True:
            connection, (host, port) = server.accept()
            threading.Thread(target=vibeplay.serve, args=(connection, f"{host}:{port}"), daemon=True).start()

    threading.Thread(target=accept, daemon=True).start()
    # A plain kill still withdraws the mDNS answer.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    try:
        for command in sys.stdin:
            command = command.strip()
            if command == "a":
                vibeplay.press(line("button", button="K2", action="press"))
            elif command.startswith("f "):
                vibeplay.press(line("focus", task=command[2:].strip()))
            elif command == "p":
                with vibeplay.lock:
                    vibeplay.new_code()
            elif command == "q":
                break
            elif command:
                print("a = press A, f <task id> = pick a card, p = new pairing code, q = quit", flush=True)
    except KeyboardInterrupt:
        pass
    finally:
        if advertiser:
            advertiser.terminate()


if __name__ == "__main__":
    main()
