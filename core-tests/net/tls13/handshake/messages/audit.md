# core-tests/net/tls13/handshake/messages — audit

`core/net/tls13/handshake/messages.vr` — RFC 8446 §4 HandshakeType byte constants. 11 canonical value pins (ClientHello=1 … MessageHash=254) + ordering invariants (14 @test). ClientHello/ServerHello/etc. record encode/decode deferred (fixed-array + extension-list surface, L2 spec level).
