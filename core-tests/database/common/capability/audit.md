# Capability-typed database handles

The handle owns a shared pool. Its adapter borrows the underlying pool once
the operation's capability bound has been selected. The PostgreSQL and
MySQL asynchronous transaction scopes keep the checked-out guard and its
transaction token. They borrow the actor from that guard for BEGIN, COMMIT
and ROLLBACK. Each scope consumes its token before releasing the guard.

The [public fixture](../../../../vcs/specs/L2-standard/database/handle_capabilities.vr)
uses PostgreSQL pools configured with zero initial connections. It checks
the public read method and the flags on admin, COPY, read-only and NOTIFY
handles. Ordinary source checking admits the fixture. The development CLI
requires a matching embedded runtime archive to execute the program.
Successful checkout, SQL execution and return to the pool require their own
driver and server tests.

Three external source controls exercise restrictions independently. Access
to the [adapter's pool](../../../../vcs/specs/L2-standard/database/handle_private_pool.vr)
and [conversion from COPY to transactional access](../../../../vcs/specs/L2-standard/database/handle_copy_transaction.vr)
both produce E400. Calling [a write operation on a read-only handle](../../../../vcs/specs/L2-standard/database/handle_readonly_write.vr)
produces E405 for its unsatisfied `CanWrite` bound.

The [capability unit tests](unit_test.vr) check each grant through the
public `has` method. A separate interpreter check parsed the capability
owner, compiled its marker implementations and mask constructors to VBC,
and read the resulting masks. It observed COPY `0x43`, NOTIFY `0x21`,
admin `0x0f`, read `0x01`, read/write/transaction `0x07` and replication
`0x11`. Changing each expected value independently made the corresponding
comparison fail. This execution exercised marker dispatch and mask
construction. The pool fixture also requires the shared pool implementation
and the PostgreSQL adapter.

## Synchronous checkout lifetime

The [checkout fixture](../../../../vcs/specs/L2-standard/database/pooled_connection_borrow.vr)
accepts a real `PooledConnection<PgConnection>`. It queries the checked-out
connection and consumes the handle on either the healthy or discard path.
Source checking reports E312 twice on the query path.

```text
let result = handle.conn_mut().simple_query(sql);
             ^^^^^^^^^^^^^^^^^
E312: cannot establish the lifetime of references stored in `handle` after this mutable call
```

The companion function borrows the connection, ends that borrow and
releases the handle. It reports one E312 on `handle.conn_mut()` with no
query present. The checkout retains a separate reference to its parent pool.

The checker traces references returned by a borrowed sum pattern to the
record field and payload position selected by each arm. A direct language
trap with literal arguments can close a terminal arm. The pool's defining
module imports the source `panic` through the prelude. That function calls
`panic_impl`, which writes to stderr and exits the process. Its writes
require a separate proof. [Panic implementation](../../../../core/base/panic.vr)

Whole-source checking reports 19 lifetime diagnostics in the PostgreSQL
adapter, 15 in MySQL and 10 in SQLite. The common pool implementation has
two further diagnostics on `idle` in `launch`. The common capability
module, handle owner and both asynchronous adapters pass whole-source
checking.
