---
status: accepted
---

# A box links to one computer at a time, and moves between computers by takeover

Once the box can be reached over the local network and the Relay, more than one computer can reach it. Today's stats, special occasions, the clock in the heartbeat and the three task cards are all decided by the daemon (`daemon/src/occasions.rs`), so two daemons feeding one box would each say "first done of the day", count their own stats and overwrite each other's cards. We decided a box has at most one link, to one computer, and a computer links to at most one box. Another paired computer gets the box only by takeover: plugging in USB always takes it, the user can take it from the App, and once the linked computer has been gone long enough (about 10 minutes) another paired computer may link on its own. Agent activity never moves the box.

## Considered options

- **The box merges per-computer snapshots** and takes over stats and occasions. Works the same over every way of carrying the link, but moves a large part of the domain into the firmware.
- **The Relay merges.** Fails when both computers reach the box over USB or the local network, unless every link is forced through the Relay.
- **One computer aggregates for the others.** Adds daemon-to-daemon forwarding and a leader to elect.
- **Whichever computer has Agent activity gets the box.** Flaps when both are busy.

## Consequences

The aggregator, today's stats and occasions stay in the daemon unchanged. If merging is ever wanted, messages will need the sending computer's id; nothing else here blocks it.
