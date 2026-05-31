# core-tests/net/h3/push — audit
`core/net/h3/push.vr` — RFC 9114 §4.6 server push. PushError 3-variant (BudgetExhausted{requested,limit} record + AlreadyCancelled(UInt64) tuple + ClientGoneAway unit): ctors + Eq + payload preservation/distinctness + disjointness (7 @test, exercises CLASS-9/D2b cross-module fields). Qualified Type.Variant. Push promise/stream lifecycle deferred to L2.
