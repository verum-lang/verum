# core-tests/net/quic/crypto/labels — audit
`core/net/quic/crypto/labels.vr` — RFC 9001 §5.2 Initial-keys length constants (AES-128-GCM). INITIAL_KEY_LEN=16 / INITIAL_IV_LEN=12 / INITIAL_HP_LEN=16 pins + relationships (4 @test). Byte-array label constants (QUIC_INITIAL_SALT_V1 etc.) deferred — element-access surface.
