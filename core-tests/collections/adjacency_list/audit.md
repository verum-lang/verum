# `core.collections.adjacency_list` — Audit

Conformance review for `core/collections/adjacency_list.vr` —
`AdjacencyList<V, L>`, generic labelled-edge graph adjacency-list
representation.

## Status

Pending unit-test run completion; the conformance suite targets
**partial** — covers empty / add_vertex / add_edge / vertex_count /
edge_count.  Iteration surface (vertices_ref / out_edges_ref) and
graph algorithms (complete_graph / path_graph / cycle_graph)
deferred pending the wrapper-iter dispatch class close-out shared
with multiset §B / slice §D.

## 1-4. (See INVENTORY.md for cross-stdlib usage / hardcodes /
gaps / defects table.)

## 5. Action items

### Landed in this branch

1. Unit-test surface — 4 tests covering empty graph + add_vertex +
   add_edge bookkeeping.
2. Property-test surface — 3 monotonicity laws (vertex_count /
   edge_count monotone; empty zero counts).
3. Integration tests — 2 scenarios (triangle graph; duplicate edges).
4. Regression suite — 4 PASS-GUARDs for the working surface.

### Deferred

1. Iteration surface (out_edges_ref / vertices_ref) — gated on
   wrapper-iter dispatch fix.
2. Graph generators (complete_graph / path_graph / cycle_graph)
   require `List.from(...)` constructor reachability.
