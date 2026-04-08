# core/net Test Suite

Test coverage for Verum's network module.

## Test Organization

| File | Module | Coverage | Tests | Status |
|------|--------|----------|-------|--------|
| `addr_test.vr` | `net/addr` | Ipv4Addr, Ipv6Addr, IpAddr, SocketAddr | 87 | All Passing |
| `dns_test.vr` | `net/dns` | DnsError, DnsRecord, DnsRecordType, Resolver | 61 | All Passing |
| `tcp_test.vr` | `net/tcp` | TcpStream, TcpListener, options, shutdown, buffered I/O | 116 | All Passing |
| `udp_test.vr` | `net/udp` | UdpSocket, bind, send/recv, broadcast, multicast, options | 88 | All Passing |
| `addr_protocols_test.vr` | `net/addr` | Eq, Hash, Display for Ipv6Addr, IpAddr, SocketAddrV4/V6, SocketAddr, AddrParseError | 41 | All Passing |
| `udp_timeout_test.vr` | `net/udp` | UdpSocket set_read_timeout/set_write_timeout, timeout patterns, DNS timeout | 21 | All Passing |
| `tcp_protocols_test.vr` | `net/tcp`, `net/addr` | TcpStream/TcpListener Display, SocketAddr Display/Debug format | 16 | All Passing |
| `net_udp_dns_extended_test.vr` | `net/udp`, `net/dns`, `net/addr` | UDP error patterns, address patterns, DNS Resolver builder, DNS Record types/variants, DNS Error variants (all 13), DNS convenience functions, DnsRecordEntry, IpAddr v4/v6 | 83 | All Passing |

## Current Status

### addr_test.vr (55 tests passing)

**All tests passing:**
- Ipv4Addr: Construction (4), Properties (7), Conversion (4), Equality (2)
- Ipv6Addr: Construction (3), Properties (5), Equality (2)
- IpAddr: Variant tests (2 - `is V4`/`is V6` pattern), is_ipv4/is_ipv6 methods (2)
- IpAddr methods on extracted values: is_loopback (2), is_unspecified (2), is_multicast (2), match (2)
- SocketAddrV4: new, ip, set_port (3)
- SocketAddrV6: new, ip (2)
- SocketAddr: new_v4, new_v6, match_v4, match_v6 (4)
- AddrParseError: all variants (4)
- Edge cases (5)

### dns_test.vr (59 tests passing)

**All tests passing:**
- DnsError: all 13 variants (6 tests)
- DnsRecord: A, AAAA, CNAME, MX, TXT, NS, PTR, SRV, SOA construction (11 tests)
- DnsRecordType: all 10 variants with to_wire() (10 tests)
- Resolver: new(), timeout_ms(), retries(), nameserver_ip(), search_domain(), use_tcp() (8 tests)
- DnsRecordEntry: construction and field access (3 tests)

**Skipped tests (21 tests):**
- Domain validation (11 tests): requires Text.split() not available in VBC
- IP address checking (10 tests): requires Text.split() not available in VBC

**Known VBC Limitations:**
- `Text.split()` method not implemented in VBC interpreter
- Methods returning `&Text` have CBGR issues (record_type(), value_string())

## Fixed VBC Interpreter Bugs

Both bugs that previously blocked these tests have been fixed:

### Bug #1: `match *self` returns wrong variants (FIXED)

**Fix:** Changed `handle_deref` in `dispatch_table.rs` to NOT automatically unwrap
single-field variants. The previous code assumed single-field variants were always
`Heap<T>` wrappers, but sum types like `IpAddr.V6(Ipv6Addr)` also have single fields.

**Fixed in:** `crates/verum_vbc/src/interpreter/dispatch_table.rs` (lines 7134-7141)

### Bug #2: Methods on extracted sum type values fail (FIXED)

**Root cause:** Method dispatch was not type-aware. When multiple types define the
same method name (e.g., `IpAddr.V6` and `SocketAddr.V6` both use variant name `V6`),
the wrong method could be called because the interpreter just searched for any
function ending with `.method_name`.

**Fix:** Three-part solution in codegen:
1. Added `variant_payload_types: Option<Vec<String>>` to `FunctionInfo` to track
   the payload type of each variant
2. In `compile_match`, extract and store the scrutinee's type in `match_scrutinee_type`
3. In `compile_pattern_bind`, use the scrutinee type to look up the correct qualified
   variant name (e.g., `IpAddr.V6` instead of just `V6`)

**Fixed in:**
- `crates/verum_vbc/src/codegen/context.rs` - Added `variant_payload_types` and `match_scrutinee_type`
- `crates/verum_vbc/src/codegen/mod.rs` - Store payload types when registering variants
- `crates/verum_vbc/src/codegen/expressions.rs` - Use scrutinee type for qualified lookup

### Bug #3: Method calls on reference types fail (FIXED)

**Root cause:** When a variable has a reference type (e.g., `&Ipv6Addr`), the type annotation
extraction in `compile_let` only handled `TypeKind::Path` and `TypeKind::Generic`, ignoring
`TypeKind::Reference`. This caused `variable_type_names` to be empty for reference-typed
variables, breaking method resolution.

**Example:**
```verum
let returned_ip: &Ipv6Addr = addr.ip();
returned_ip.is_loopback();  // FAILED: method not found
```

**Fix:** Extended the type name extraction to handle reference types by extracting the
pointee type. For `&Ipv6Addr`, we now correctly extract `Ipv6Addr` as the type name.

**Fixed in:** `crates/verum_vbc/src/codegen/statements.rs` (lines 167-190)

## Key Types Tested

### Ipv4Addr (addr_test.vr)
- Construction: `new()`, `localhost()`, `unspecified()`, `broadcast()`
- Properties: `is_loopback()`, `is_unspecified()`, `is_private()`, `is_multicast()`, `is_broadcast()`
- Conversion: `octets()`, `to_u32()`, `from_u32()`
- Equality comparison

### Ipv6Addr (addr_test.vr)
- Construction: `new()`, `localhost()`, `unspecified()`
- Properties: `is_loopback()`, `is_unspecified()`, `is_multicast()`, `is_link_local()`, `is_unique_local()`
- Segment access: `segments()`
- Equality comparison

### IpAddr (V4 | V6) (addr_test.vr)
- Variant construction: `IpAddr.v4()`, `IpAddr.v6()`
- Pattern matching: `addr is V4`, `addr is V6`
- Methods: `is_ipv4()`, `is_ipv6()`, `is_loopback()`, etc. (all working)

### SocketAddrV4 / SocketAddrV6 (addr_test.vr)
- Construction: `new()`
- Port access: `port()`, `set_port()`
- IP access: `ip()` (working)

### AddrParseError (addr_test.vr)
- Variants: `InvalidFormat`, `InvalidOctet`, `InvalidPort`
- Pattern matching

## Test Count Summary

| File | Passing | Skipped | Failing | Total |
|------|---------|---------|---------|-------|
| addr_test.vr | 87 | 0 | 0 | 87 |
| dns_test.vr | 61 | 0 | 0 | 61 |
| tcp_test.vr | 116 | 0 | 0 | 116 |
| udp_test.vr | 88 | 0 | 0 | 88 |
| tcp_protocols_test.vr | 16 | 0 | 0 | 16 |
| net_udp_dns_extended_test.vr | 83 | 0 | 0 | 83 |
| **Total** | **451** | **0** | **0** | **451** |

## Next Steps

1. **Implement Text.split() in VBC** - Enable domain validation and IP address parsing tests
2. **Fix CBGR issues with &Text returns** - Methods returning string references have generation tracking issues
