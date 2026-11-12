# VibeBuddy on the VibePlay

The app the VibePlay handheld runs: a full-screen QingOS app (wasm32) that the computer connects to on the local network. The protocol, the screens and the keys are in [`docs/vibeplay.md`](../docs/vibeplay.md).

- `core/` (`vibeplay-core`): pairing (Noise XX), the link (Noise IK), and the buddy: the box's firmware (`firmware-rs/core`) given the events, drawing the box's screens, with its Pomodoro and leisure. Also the screens only the VibePlay has, drawn with the firmware's canvas and fonts. Plain Rust: it reads no clock and touches no socket.
- `src/`: QingOS's connection events and keys in; lines, mDNS, data, sounds and the 640 × 480 frame out (the firmware's 320 × 240 screen, doubled).
- `assets/`: the robot's three lines as WAV (`tools/make-sounds.py` makes them from `voices/jessica/`) and the cover. CC BY-SA 4.0, see `LICENSE-ASSETS`.

It is its own Cargo workspace, not in the root one. It links `firmware-rs/core` and `protocol/`, so it builds from a checkout of the whole repository.

```bash
cargo test --workspace                                  # the tests, on the computer
VIBEPLAY_SHOTS=shots cargo test --test app shots        # every screen as a PNG, in both looks
tools/compare-box-screens.sh                            # the screens against the firmware's previews, pixel for pixel
VIBEPLAY=<vibeplay CLI> tools/compare-wasm-shots.sh     # the release wasm's screens against the tests'
cargo build --release --target wasm32-unknown-unknown   # what QingOS runs
vibeplay run                                            # play it in a window (vibeplay CLI from vibeplay-sdk 9e8e424 or later)
```
