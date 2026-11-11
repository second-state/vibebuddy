# VibeBuddy on the VibeGame

The app the VibeGame handheld runs: a full-screen QingOS app (wasm32) that the computer connects to on the local network. The protocol, the screens and the keys are in [`docs/vibegame.md`](../docs/vibegame.md).

- `core/` (`vibegame-core`): pairing (Noise XX), the link (Noise IK), the events and Pomodoro, in plain Rust. It reads no clock and touches no socket.
- `src/`: QingOS's connection events and keys in; lines, mDNS, data, sounds and the 240×160 screen out.
- `assets/`: the robot's three lines as WAV (`tools/make-sounds.py` makes them from `voices/jessica/`) and the cover. CC BY-SA 4.0, see `LICENSE-ASSETS`.

It is its own Cargo workspace, not in the root one. `src/art.rs` includes the box's Agent icons from `firmware-rs/core`, so it builds from a checkout of the whole repository.

```bash
cargo test --workspace                                  # the tests, on the computer
VIBEGAME_SHOTS=shots cargo test --test app shots        # every screen as a PNG, in English and Chinese
cargo build --release --target wasm32-unknown-unknown   # what QingOS runs
vibegame run                                            # play it in a window (vibegame CLI with qingos-sdk 0ff2e32 or later)
```
