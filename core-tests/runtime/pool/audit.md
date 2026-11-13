# Blocking worker pool

`ThreadPool.submit` and `ThreadPool.global_submit` accept a callable with signature
`fn(Int) -> Int` and return a `PoolTaskHandle`. The five private VBC declarations
carry their intrinsic identities independently of their source identifiers. Both
tiers refuse captured environments before admission. Indirection through a
function parameter preserves that restriction.

The interpreter executes jobs eagerly and keeps results in the owning VM. Darwin
native workers execute callbacks on operating-system threads. A completed handle
can be collected after its local pool shuts down. Repeating `join()` on the same
handle returns zero without reading an underlying completion twice.

The unit, regression and property suites exercise the public pool, including
independent outstanding jobs, result ordering and collection after shutdown.
`blocking_pool_contract` exercises the complete pool source through serialized
VBC. `native_pool_refusal` covers emitted native workers. Other native targets
require their own threading implementation.
