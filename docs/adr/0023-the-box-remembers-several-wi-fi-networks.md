---
status: accepted
---

# The box remembers several Wi-Fi networks

Until now the box kept one Wi-Fi network. On 2026-11-06 dragon took the box from the office home. The box had never joined the home network, so it had to go on a computer's cable there to be set, and setting it replaced the office network. Back at the office, the same again. A box that moves between a few places needs to know all of them.

So the box keeps a short list, and picks from it by itself:

- **Up to 5 networks, most recently joined first.** Setting a network adds it; setting a known name again gives it the new password. Either way it goes first and the box joins it now. With 5 known, adding one more forgets the one joined longest ago. Each network costs at most 97 bytes of the 4 KB link record (`link.rs`).
- **Which one to join.** At boot, and whenever the box loses its network, it scans and tries the known networks it hears, strongest first. If it hears none of them (a hidden network, or a failed scan), it tries every one, most recently joined first. When all fail it waits 10 s and goes again. The network it joins goes first in the list, so the menu, the apps and an older firmware all see the one it is on.
- **Set and forgotten over the cable only**, as before (ADR-0012): whoever is at the box decides where it goes. The apps list the networks by name and forget them one at a time. The passwords never leave the box.
- **The menu stays as it is.** The NETWORK view shows the network the box is on, or the first one, dimmed, while it is not on one. Forgetting is done from the apps.

## Compatibility

The link record keeps its old shape for the first network: name, password, then the Relay. The other networks come after the Relay, which firmware from before several networks never reads (ADR-0014). So rolling back the firmware keeps the box on its most recent network, and the first save of the old firmware drops the others.

`device.wifi.forget` with an `ssid` forgets that network; without one, as an App from before several networks sends it, every network goes. Firmware from before several networks knows one network and forgets it either way. The box reports `WIFI NETWORKS <count>` and a `WIFI KNOWN <ssid>` for each before the `WIFI NETWORK <ssid>` it always sent, so an older daemon still sees the first network.

## Considered options

- **Try the most recently joined network first, always.** Rejected: at home, the box would wait out the office network's 10 to 15 s failure before trying the network it can hear.
- **Join any open network it hears.** Rejected: the box would join networks nobody chose for it.
- **More than 5.** Not needed for a box that moves between a few places, and each one more is another network to try when none is heard.
