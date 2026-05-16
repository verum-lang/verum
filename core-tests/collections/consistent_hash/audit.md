# `core.collections.consistent_hash` — Audit

`core/collections/consistent_hash.vr` — `ConsistentHashRing`,
Ketama-compatible virtual-node hash ring (160 vnodes default).

## Status

**partial** — Empty-ring construction surface (new /
with_virtual_nodes / node_count / position_count / node_for_key
on empty / nodes_for_key on empty) is exhaustively tested.
Node-populated surface (add_node / remove_node with concrete Text
identifiers) is deferred pending `Text.from(...)` reachability
from user code (closed in some paths, pinned where it isn't).

## Action items

### Landed in this branch

1. 5 unit + 5 property + 2 integration + 4 PASS-GUARDs +
   1 @ignore'd populated-state pin.

### Deferred

* `add_node` / `remove_node` integration tests — gated on
  `Text.from` reachability.
* `node_for_key` distribution properties — gated on
  multi-node setup.
