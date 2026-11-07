---
status: proposed
---

# A company edition is data on the box, not a build

On 2026-11-06 we discussed a company edition: a company buys a batch of boxes as a gift for its customers or its staff. Each box comes with a company Character, made from the company's logo or mascot. The App shows it first in the Character list. Ordinary boxes and ordinary users never see it.

We will not make a firmware build, an App build or an App component for each company. With more companies, that would give us more builds to release, test and keep. So there is one firmware and one App for everyone, and a company edition is only data that the factory writes to the box:

- **Its own partition.** A new 2 MB `edition` partition after `link`, at 0xa14000. The factory writes it, and nothing else writes it. It holds a Character pack in the usual `VBCP` format and a small edition manifest: the edition id, the company name, and the one-sentence card summary. The `voices` partition cannot hold the company Character: when the user picks another Character, the pack in `voices` is written over, and the company Character would be lost.
- **The box says it has one.** In answer to `device.hello`, a box with an edition sends `EDITION <id>`. A box with an empty or missing `edition` partition sends no `EDITION` line.
- **The App reads the card from the box.** When it sees `EDITION`, the App reads the pack header, the look and the manifest from the box (about 7 KB, not the full 2 MB). It shows the company Character first in the Character list, and the robot second. Without `EDITION`, the list does not change. The Mac app and the Linux app both do this.
- **The user can change Character, and can go back.** The company Character is not locked. The user can pick any other Character. To go back, the App sends one command, and the box copies the pack from `edition` to `voices` itself. This needs no transfer, no network, and no other computer.
- **An optional boot logo.** The edition can also hold one full-screen logo. Today the box has no boot screen, and it restarts each time the daemon opens the serial port, so the logo shows only after a power-on if the firmware can tell one apart, else for at most 1 s on each boot. It never delays the link. Whether the firmware can tell a power-on apart is checked on a real box first.
- **The factory writes it.** A tool (`tools/make-edition`) builds the edition image from a company Character. The factory flashes it together with the firmware and the new partition table.
- **The company's assets stay out of this repo.** This repo is open source. The company's logo, look, voice and lines belong to the company, so they go in a private repo.

## Considered options

- **Ship the company Character in the App.** Rejected: every ordinary user gets every company's assets, and each new company needs an App release.
- **Keep only an edition id on the box, and get the Character from our service.** Rejected: it needs the network and a service to keep, and anyone who changes the id can get another company's Character.
- **Write the company Character only to `voices` at the factory.** Rejected: the first change of Character deletes it for ever.
- **Lock the box to the company Character.** Rejected: it gives more rules in the firmware and the App, and a worse gift. The user can always go back to it in one step.

## Consequences

- New boxes get a new partition table. `link` does not move (ADR-0014), and boxes already sold keep their layout and have no edition. The firmware must work with both tables: it finds `edition` at run time and treats a missing partition as an empty one.
- The firmware gets the `EDITION` line, a read of the edition's header, look and manifest, and a command to copy `edition` to `voices`. Each is in docs/protocol.md when it lands.
- A new company is a new edition image. It needs no new code.
- An edition cannot change after the factory writes it. If companies want to change it later (for example, holiday lines), a `kind: edition` transfer (ADR-0016) can come later.

## Open questions

- A gift code for the Paid plan, bought by the company with the boxes, so that voice works without the user's own keys.
- Recipients who do not use coding Agents: what the box does for them.
