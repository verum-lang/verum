# core-tests/net/h3/qpack/integer — audit

`core/net/h3/qpack/integer.vr` — RFC 9204 §4.1.1 QPACK prefix-integer codec. IntError 2-variant (Truncated/Overflow): ctors + Eq + disjointness (3 @test). Qualified Type.Variant. encode/decode varint round-trip deferred (byte-buffer surface, same class as hpack integer codec).
