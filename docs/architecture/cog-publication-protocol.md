# Cog Publication Protocol v1

This is the source-cog publication contract shared by the CLI and registry
node. It specifies the request, admission rules and acknowledgement needed
for an authenticated publish-to-download flow. Passing a client HTTP fixture
does not establish that a registry node implements this contract. The release
gate remains the real-service flow in
[Registry delivery acceptance](registry-federated-design.md#6-delivery-acceptance).

## Request and exact-byte envelope

A client sends `POST <registry-base>/api/v1/cogs/publish` with:

- `Authorization: Bearer <credential>`
- `Content-Type: application/vnd.verum.cog-publish.v1`
- One binary body with the following fields in order.

| Field | Encoding | Meaning |
|---|---|---|
| Metadata length | 4-byte unsigned little-endian integer | Number of metadata bytes, excluding this length field |
| Metadata | Exactly the declared length of UTF-8 JSON | One source-metadata object |
| Archive length | 8-byte unsigned little-endian integer | Number of archive bytes, excluding this length field |
| Archive | Exactly the declared length of binary data | The original gzip-tar source cog |

The complete body length is `4 + metadata_length + 8 + archive_length`.
Every addition and integer conversion must be checked. Reject a zero-length
metadata object or archive, truncated length fields, truncated data, trailing
bytes, integer overflow and any configured size-limit violation. The media
type selects version 1; an unsupported media type or version is refused.
There is no base64 transformation, multipart boundary or HTTP content encoding
inside this envelope. A producer must not replace the archive with a decoded
or repacked equivalent.

The client uses the configured registry base, including its path prefix. It
does not follow publication redirects: unpublished source and credentials
must not be forwarded to a new endpoint by a redirect response.

## Source metadata

The object contains exactly these fields. Optional values are explicitly
`null`; empty collections use `[]` or `{}` as appropriate.

| Field | JSON type | Meaning |
|---|---|---|
| `name` | string | Cog name admitted by the project name validator |
| `version` | string | Exact semantic version, never a version requirement or `latest` |
| `description` | string or null | Publisher description |
| `authors` | array of strings | Publisher attribution |
| `license` | string or null | License declaration |
| `repository` | string or null | Source repository location |
| `homepage` | string or null | Project page |
| `keywords` | array of strings | Search terms |
| `categories` | array of strings | Categories |
| `readme` | string or null | README content |
| `dependencies` | object | Dependency names mapped to the CLI dependency specification |
| `features` | object | Feature names mapped to arrays of feature/dependency names |
| `checksum` | string | Lowercase, 64-digit SHA-256 hex of the exact archive bytes |

A dependency specification is either a version-requirement string or an
object whose only fields are `version` (string or null), `features` (array of
strings or null), `optional` (boolean or null) and `default_features` (boolean
or null). Omitted fields in this detailed dependency form have their normal
optional meaning. The top-level publication fields are all required.

Unknown fields are refused. In particular, `as_authority`, `authority`,
`published_at`, `artifacts`, `proofs`, `cbgr_profiles`, `signature`,
`ipfs_hash`, inclusion claims and verification verdicts are not publication
inputs. Node identity, publication time and evidence receipts belong to the
server. Source strings, including README and descriptions, are untrusted
publisher content and confer no authority or proof status.

Reject duplicate object keys at every depth, comparing decoded key strings
so escaped spellings cannot bypass the rule. This is a decoder requirement,
not a claim that ordinary JSON-to-map parsing already enforces it. The current
core JSON object parser inserts into a map and overwrites duplicate keys; a
server must check duplicates before that information is lost or use a strict
decoder. Object ordering and insignificant JSON whitespace have no semantic
meaning. These metadata bytes are not claimed to be canonical JSON or a
signature payload.

The client validates the name, exact version and checksum shape, bounds
serialization, reads bounded archive bytes and recomputes SHA-256 before
sending. The server independently validates the complete metadata schema,
archive format, declared digest and archive manifest coordinate before any
publication mutation. Malformed metadata is a `400` error; a well-formed
checksum that does not match the archive is a distinct `422` error.

Source publication v1 has no signature or proof-submission field. The current
CLI automatically signs when it finds a local signing key. Such a request is
explicitly refused by v1, including during `package publish --dry-run`; the
CLI must not discard the signature or announce that this request is valid for
publication. Verified signature and evidence support remains required launch
work tracked by T1648.

The command's current manifest-to-metadata conversion also flattens detailed
dependencies. T1651 tracks preserving feature/optional/default-feature options
and refusing source declarations that the registry cannot represent. Correct
wire framing alone does not establish faithful project dependency semantics.

## Limits and allocation

| Data | Default client limit | Protocol ceiling |
|---|---|---|
| Metadata | 262,144 bytes (256 KiB) | 1,048,576 bytes (1 MiB) |
| Compressed archive | 67,108,864 bytes (64 MiB) | 67,108,864 bytes (64 MiB) |
| Success receipt | 65,536 bytes (64 KiB) | 65,536 bytes (64 KiB) |

The Rust client exposes `PublicationLimits::new(metadata_bytes, archive_bytes)`
and `RegistryClient::with_publication_limits(...)`. Both configured values
must be positive and within their ceilings. These are consumed client API
settings; this contract introduces no manifest keys.

The server must apply its configured metadata/archive bounds before allocating
from an untrusted length. Its HTTP body limit must accommodate exactly the
configured envelope bound, including the 12 length bytes. At the defaults this
is 67,371,020 bytes; at the protocol ceilings it is 68,157,452 bytes. A parser
whose request-size limit also includes headers must account separately for
its bounded header budget. A one-MiB default HTTP request limit does not admit
a default-sized publication. Expanded archive contents require a separate
bounded extraction policy; the compressed-byte ceiling is not an expansion
limit.

## Authentication, authority and durable admission

The credential verifier resolves the bearer credential to a principal with an
explicit package/name scope. Missing or invalid credentials yield `401`; a
valid principal without the required publication scope yields `403`. Only the
server may derive a publication capability from that verified principal.
A publisher-supplied name or string never mints authority.

Archive storage, immutable metadata and the authoritative publication record
must be committed durably before a success response. A storage failure must
not leave an accepted in-memory coordinate behind. Admission and conflict
checks must remain correct for concurrent requests and after restart.

An exact retry with the same source metadata, archive bytes and authorized
principal may return the existing receipt. Changing metadata or archive bytes
at an existing immutable name/version coordinate yields `409`; changing the
coordinate's recorded authority cannot silently replace its provenance.
Transport JSON member order is irrelevant to an identical retry. An existing
catalog that compares only archive digests cannot establish this condition:
the admission transaction must store and compare the full validated source
metadata and authenticated principal, while retaining the original server
timestamp and provenance. This rule does not define a canonical serialization
or bless placeholder digest,
signature or log-proof implementations as cryptographic evidence.

## Success receipt and errors

Only `201 Created` for a durable new publication or `200 OK` for an identical
retry acknowledges publication. The response media type is `application/json`
with optional UTF-8 charset. Its bounded body is exactly one object:

```json
{"name":"example","version":"1.2.3","checksum":"<64 lowercase SHA-256 hex digits>"}
```

All three fields are required strings; unknown or duplicate fields are
refused. The client parses this receipt and requires exact name, version and
checksum equality with its validated request before reporting success. An
empty body, malformed JSON, extra JSON values, wrong coordinate, wrong digest,
oversized receipt or another success status does not acknowledge publication.
A receipt confirms admission to this node; it is not a signature or a proof of
inclusion in a transparency log.

| Status | Meaning |
|---|---|
| `400` | Malformed envelope or metadata, including unknown/duplicate fields |
| `401` | Missing or invalid credential |
| `403` | Principal lacks publication scope |
| `409` | Immutable coordinate, metadata or authority conflict |
| `413` | Metadata, archive or request size limit exceeded |
| `415` | Unsupported publication media type/version |
| `422` | Validly framed content fails digest, archive or manifest validation |
| `5xx` | Server failure; no successful publication receipt |

A missing or rejected receipt leaves the client uncertain whether the server
committed. A retry must use the same coordinate, metadata and exact bytes;
the client must not invent a new publication time or trust verdict to resolve
that uncertainty.

## Download parity and executable acceptance

After publication, exact-version metadata is available at
`<registry-base>/api/v1/cogs/<name>/<version>` and the same exact source archive
at `<registry-base>/api/v1/cogs/<name>/<version>/download`. Metadata reports the
matching checksum; the server supplies its publication time and provenance.
A consumer verifies the archive digest and records the selected registry in
its lockfile.

Client regressions in `crates/verum_cli/tests/registry/publication_transport.rs`
capture actual loopback HTTP requests and refusal responses. Command-handler
regressions in `crates/verum_cli/tests/cog/publication_validation.rs` exercise
archive creation, automatic signing and dry-run admission with a temporary
key. They do not establish authentication, archive validation, durability or
compiler installation on a registry node.

Final acceptance must publish a genuine source cog using a real authenticated
Verum service, download identical bytes, reject credential/scope violations,
malformed frames and conflicting retries, survive restart, then install and
mount the cog from a fresh project. Both the server and CLI source/tool
identities must be recorded. T1636 remains open until that service-level
interoperability gate passes.
