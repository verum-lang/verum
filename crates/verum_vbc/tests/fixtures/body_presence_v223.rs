//! Independent v2.23 wire fixture, following the writer at fec9b1af6^.
//!
//! This is a small hand-encoded module, not current bytes with an older label.
//! Its four function descriptors stop after the semantic-formal presence byte;
//! no later result, source-signature or callable-identity tails are present.
use verum_common::List;

pub fn module_bytes() -> List<u8> {
    const HEADER: usize = 96;
    const USIZE: u32 = 14;
    const STRINGS: [&str; 7] = [
        "body_archive",
        "empty",
        "forward",
        "USize",
        "USize.empty",
        "USize.forward",
        "self",
    ];
    let mut bytes = List::new();
    bytes.resize(HEADER, 0u8);
    let mut strings = List::new();
    for string in STRINGS {
        strings.push((bytes.len() - HEADER) as u32);
        bytes.extend_from_slice(&(string.len() as u32).to_le_bytes());
        bytes.extend_from_slice(string.as_bytes());
    }
    let string_size = (bytes.len() - HEADER) as u32;
    let type_start = bytes.len() as u32;
    bytes.extend_from_slice(&USIZE.to_le_bytes());
    bytes.extend_from_slice(&strings[3].to_le_bytes());
    bytes.extend_from_slice(&[0, 1]); // primitive, public
    bytes.extend_from_slice(&8u32.to_le_bytes()); // size
    bytes.extend_from_slice(&8u32.to_le_bytes()); // alignment
    // No type parameters, fields, variants, Drop, Clone or protocol entries;
    // no alias, transparent wrapper, alias spelling, origin or declared layout.
    bytes.extend_from_slice(&[0; 11]);
    bytes.push(1); // unrestricted resource discipline (v2.18)
    let function_start = bytes.len() as u32;
    for (id, name) in [strings[1], strings[2], strings[4], strings[5]]
        .into_iter()
        .enumerate()
    {
        let method = id >= 2;
        bytes.extend_from_slice(&(id as u32).to_le_bytes());
        bytes.extend_from_slice(&name.to_le_bytes());
        bytes.push(u8::from(method));
        if method {
            bytes.extend_from_slice(&USIZE.to_le_bytes());
        }
        bytes.push(1); // public
        bytes.push(if id % 2 == 0 { 8 } else { 0 }); // reserved bit, NOT body evidence
        bytes.extend_from_slice(&0u16.to_le_bytes()); // properties
        bytes.extend_from_slice(&(id as u32).to_le_bytes()); // bytecode offset
        bytes.extend_from_slice(&1u32.to_le_bytes()); // one RetV, even for forward
        bytes.extend_from_slice(&u16::from(method).to_le_bytes()); // locals
        bytes.extend_from_slice(&u16::from(method).to_le_bytes()); // registers
        bytes.extend_from_slice(&0u16.to_le_bytes()); // stack
        bytes.push(0); // type parameters
        bytes.push(u8::from(method)); // ordinary parameters
        if method {
            bytes.extend_from_slice(&strings[6].to_le_bytes());
            bytes.push(1); // Concrete(USize)
            bytes.extend_from_slice(&USIZE.to_le_bytes());
            bytes.extend_from_slice(&[0, 0]); // immutable; no default
        }
        bytes.extend_from_slice(&[1, 0, 0, 0, 0]); // Concrete(Unit) result
        // No contexts, intrinsic, const flag, register hints or return spelling.
        bytes.extend_from_slice(&[0; 5]);
        bytes.push(u8::from(method)); // parameter spelling count
        if method {
            bytes.push(0);
        } // no declared spelling
        // No origin, explicit generics, value-use receipts or semantic formals.
        bytes.extend_from_slice(&[0; 4]);
    }
    let bytecode_start = bytes.len() as u32;
    bytes.extend_from_slice(&[0, 0x5a, 0x5a, 0x5a, 0x5a]); // no compression; four RetV
    let end = bytes.len() as u32;
    let wire = bytes.as_mut_slice();
    wire[..4].copy_from_slice(b"VBC1");
    wire[4..6].copy_from_slice(&2u16.to_le_bytes());
    wire[6..8].copy_from_slice(&23u16.to_le_bytes());
    for (at, value) in [
        (12, strings[0]),
        (16, type_start),
        (20, 1),
        (24, function_start),
        (28, 4),
        (32, bytecode_start),
        (40, HEADER as u32),
        (44, string_size),
        (48, bytecode_start),
        (52, 5),
        (56, end),
    ] {
        wire[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let hash = blake3::hash(&wire[HEADER..]);
    wire[72..80].copy_from_slice(&hash.as_bytes()[..8]);
    let dependencies = blake3::hash(&[]);
    wire[80..88].copy_from_slice(&dependencies.as_bytes()[..8]);
    bytes
}
