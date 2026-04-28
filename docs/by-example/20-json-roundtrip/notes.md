# 20 — JSON Round-Trip

`core.encoding.json` is the audit-hardened RFC 8259 parser that
enforces depth + per-string + per-array + per-object caps to defend
against hostile input.

## The two primitives

| Function | Signature | Purpose |
|---|---|---|
| `parse(text)` | `Text → Result<JsonValue, JsonError>` | UTF-8 in, ADT out |
| `stringify(value)` | `&JsonValue → Text` | ADT in, compact JSON out |

## The JsonValue ADT

```verum
type JsonValue is
    | JsonNull
    | JsonBool(Bool)
    | JsonInt(Int64)
    | JsonFloat(Float)
    | JsonString(Text)
    | JsonArray(List<JsonValue>)
    | JsonObject(Map<Text, JsonValue>);
```

Pattern-match to navigate; build by construction. There's no
"untyped" intermediate — `JsonValue` is the typed in-memory
representation.

## Caps (audit-enforced)

| Cap | Default | Why |
|---|---|---|
| `MAX_JSON_DEPTH` | 256 | Stack-overflow defence |
| `MAX_JSON_STRING_BYTES` | 32 MiB | Memory-DoS defence |
| `MAX_JSON_ARRAY_ITEMS` | 1,000,000 | Memory-DoS + O(N²) defence |
| `MAX_JSON_OBJECT_KEYS` | 1,000,000 | Memory-DoS + canonicalisation defence |

A peer sending a 256-deep nested object → `Err(JsonError.NestingTooDeep)`,
no stack overflow. A 32 MiB+ string in the wire → bail at the
boundary, no allocation. The defaults match the audit's class-2
(hostile-input fan-out) recipe documented in `#203`.

## Canonical form

For signing / hashing, use `core.encoding.jcs` (JSON Canonicalization
Scheme, RFC 8785) — it sorts object keys in UTF-16 code-unit order
and serialises numbers per ECMA-404, producing byte-identical output
across implementations. Both signer and verifier must use JCS or a
compatible canonicaliser; plain `stringify` does not sort keys.

## Round-trip property

For every JSON-spec-conforming input `x`:

```
parse(stringify(parse(x))) == parse(x)
```

The two parses produce equal `JsonValue` trees; `stringify` is
deterministic for any given tree. This makes JSON a safe IPC /
storage format for Verum data.
