# Warp fuzz seed corpus

Seed inputs for the net fuzz harnesses under
`vcs/fuzz/harnesses/net/`. One subdirectory per parser under test:

```
seeds/net/
├── tls13/
│   ├── handshake/
│   │   ├── valid/       # RFC 8448 App A ClientHello + ServerHello
│   │   └── malformed/   # truncation + field-overflow seeds
│   └── record/
├── quic/
│   ├── packet/
│   │   └── valid/       # RFC 9001 App A Initial packet
│   └── frame/
├── h3/
│   ├── frame/
│   └── qpack/
└── x509/
    └── der/
        ├── valid/       # Let's Encrypt / DigiCert real-world certs
        └── malformed/
```

Files are raw bytes (no text encoding). Each `malformed/` corpus file
is expected to produce a typed error without panic when fed to its
harness.

## Contributing a seed

1. Capture raw wire bytes (wireshark export, tcpdump, etc.).
2. Strip outer layers (Ethernet → IP → UDP/TCP) down to the
   parser-input boundary.
3. Drop the raw byte file into the appropriate `valid/` or `malformed/`
   folder.
4. Run the harness once to confirm behaviour.
5. Commit with a short README entry describing the trace source.
