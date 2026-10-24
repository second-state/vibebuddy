# Window-listing fixtures

Synthetic, written from each tool's documented output shape. Nothing here was captured from a running compositor or X
server, so the field layout is not yet verified against real output. Recapture on a real session before relying on it.

- `sway-get-tree.json`: the shape of `swaymsg -t get_tree` in the sway IPC reference (`nodes`, `floating_nodes`, the
  container `id`, and `pid` on windows). Layout and pids are invented for the test.
- `wmctrl-list.txt`: the shape of `wmctrl -l -p` (window id, desktop, pid, host, title; desktop `-1` for sticky
  windows). Ids and pids are invented.
