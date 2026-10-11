# Map mutable lookup: ordinary consumer replay

The unchanged nested Map regression now prints both exact oracles:

```text
map-read: ok
map-mutate: ok
```

It reads the original nested variant/record value, mutates its fields through
`Map.get_mut`, and observes the changes through the original map. The source,
stdout oracle and 120-second deadline are unchanged from the retained failing
consumer replay. The repaired run exits zero in 84.175 seconds.

The exact complete collections cookbook example also passes, printing
`map-access: ok` in 103.136 seconds. It covers owned scalar lookup, absent keys,
stored-slot mutation and fresh lookup. Its saved scalar remains unchanged.
The retained page is the pre-publication snapshot used to extract that exact
example; subsequent explanatory prose is maintained in the website repository.

The [manifest](manifest.json) retains source fixtures, expected output, raw
logs, runners and both receipts. The consumer CLI was rebuilt at
`6a0bf542b9a2a3158d78b4015cf1b533eecfd13e` in 229.928 seconds. All 5,392
recorded source inputs stayed unchanged, as did the five stdlib files from
the independently recorded Stage23 producer. Their original automatic bake
is not a new bake of this consumer revision.

The runtime fixes also pass the separate
[50-test main integration gate](../../map-reference-main-landing/README.md)
and are committed on local main. The consumer branch additionally includes
previously recorded registry/checker work; this is not a rebuilt main
distribution or evidence that every pending branch has landed.

These replays establish interpreter Map access for the recorded cases.
Entry APIs, iterator helpers, whole-map copying, representation unification,
stale-borrow detection, native AOT and registry publication remain separate
acceptance requirements. Shape changes are tested only after the borrow ends.
