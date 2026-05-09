#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    deprecated,
    unexpected_cfgs,
    forgetting_copy_types
)]
//! Snowflake sequence overflow drift guard (#72).
//!
//! `core/base/snowflake.vr` defines the Snowflake ID generator.  When the
//! 12-bit sequence counter saturates at 4095, the generator must advance to
//! the next millisecond before emitting the next ID.
//!
//! This drift guard pins:
//!   1. snowflake.vr defines SEQUENCE_BITS = 12.
//!   2. snowflake.vr defines WORKER_BITS = 10.
//!   3. snowflake.vr defines MAX_SEQUENCE = 4095.
//!   4. snowflake.vr defines TIMESTAMP_SHIFT = 22.
//!   5. snowflake.vr has a `wait_for_next_ms` function.
//!   6. next_id increments sequence and wraps via `& MAX_SEQUENCE`.
//!   7. next_id calls `wait_for_next_ms` when sequence wraps to 0.
//!   8. SnowflakeError has ClockRegressed / WorkerIdOutOfRange / ClockBeforeEpoch.
//!   9. SnowflakeParts type exists with timestamp_ms, worker_id, sequence fields.
//!  10. VCS spec uses SEQUENCE_BITS, MAX_SEQUENCE, TIMESTAMP_SHIFT, WORKER_BITS.

const SNOWFLAKE_VR: &str = include_str!("../../../core/base/snowflake.vr");
const SNOWFLAKE_SPEC: &str = include_str!(
    "../../../vcs/specs/L2-standard/time/snowflake_sequence_overflow.vr"
);

// ── 1. SEQUENCE_BITS = 12 ────────────────────────────────────────────────────

#[test]
fn snowflake_vr_defines_sequence_bits_12() {
    assert!(
        SNOWFLAKE_VR.contains("SEQUENCE_BITS"),
        "snowflake.vr must define SEQUENCE_BITS"
    );
    assert!(
        SNOWFLAKE_VR.contains("= 12;"),
        "snowflake.vr SEQUENCE_BITS must be assigned 12"
    );
}

// ── 2. WORKER_BITS = 10 ──────────────────────────────────────────────────────

#[test]
fn snowflake_vr_defines_worker_bits_10() {
    assert!(
        SNOWFLAKE_VR.contains("WORKER_BITS"),
        "snowflake.vr must define WORKER_BITS"
    );
    assert!(
        SNOWFLAKE_VR.contains("= 10;"),
        "snowflake.vr WORKER_BITS must be assigned 10"
    );
}

// ── 3. MAX_SEQUENCE = 4095 ───────────────────────────────────────────────────

#[test]
fn snowflake_vr_defines_max_sequence_4095() {
    assert!(
        SNOWFLAKE_VR.contains("MAX_SEQUENCE"),
        "snowflake.vr must define MAX_SEQUENCE"
    );
    assert!(
        SNOWFLAKE_VR.contains("4095"),
        "snowflake.vr MAX_SEQUENCE comment or value must reference 4095"
    );
}

// ── 4. TIMESTAMP_SHIFT = 22 ──────────────────────────────────────────────────

#[test]
fn snowflake_vr_defines_timestamp_shift_22() {
    assert!(
        SNOWFLAKE_VR.contains("TIMESTAMP_SHIFT") && SNOWFLAKE_VR.contains("22_u64"),
        "snowflake.vr must define TIMESTAMP_SHIFT = 22_u64"
    );
}

// ── 5. wait_for_next_ms function ─────────────────────────────────────────────

#[test]
fn snowflake_vr_has_wait_for_next_ms() {
    assert!(
        SNOWFLAKE_VR.contains("wait_for_next_ms"),
        "snowflake.vr must define a 'wait_for_next_ms' function"
    );
}

// ── 6. sequence increment via & MAX_SEQUENCE ─────────────────────────────────

#[test]
fn snowflake_vr_wraps_sequence_with_bitand() {
    assert!(
        SNOWFLAKE_VR.contains("& MAX_SEQUENCE"),
        "snowflake.vr next_id must wrap sequence via '& MAX_SEQUENCE'"
    );
}

// ── 7. next_id calls wait_for_next_ms on sequence == 0 ───────────────────────

#[test]
fn snowflake_vr_next_id_calls_wait_on_wrap() {
    assert!(
        SNOWFLAKE_VR.contains("self.sequence == 0_u64"),
        "snowflake.vr must check 'self.sequence == 0_u64' after wrap"
    );
    assert!(
        SNOWFLAKE_VR.contains("wait_for_next_ms(self.last_ts_ms)"),
        "snowflake.vr must call 'wait_for_next_ms(self.last_ts_ms)' when sequence wraps"
    );
}

// ── 8. SnowflakeError variants ────────────────────────────────────────────────

#[test]
fn snowflake_error_has_clock_regressed() {
    assert!(
        SNOWFLAKE_VR.contains("ClockRegressed"),
        "SnowflakeError must have ClockRegressed variant"
    );
}

#[test]
fn snowflake_error_has_worker_id_out_of_range() {
    assert!(
        SNOWFLAKE_VR.contains("WorkerIdOutOfRange"),
        "SnowflakeError must have WorkerIdOutOfRange variant"
    );
}

#[test]
fn snowflake_error_has_clock_before_epoch() {
    assert!(
        SNOWFLAKE_VR.contains("ClockBeforeEpoch"),
        "SnowflakeError must have ClockBeforeEpoch variant"
    );
}

// ── 9. SnowflakeParts fields ──────────────────────────────────────────────────

#[test]
fn snowflake_parts_type_exists() {
    assert!(
        SNOWFLAKE_VR.contains("SnowflakeParts"),
        "snowflake.vr must define SnowflakeParts type"
    );
}

#[test]
fn snowflake_parts_has_sequence_field() {
    assert!(
        SNOWFLAKE_VR.contains("sequence:"),
        "SnowflakeParts must have a 'sequence' field"
    );
}

#[test]
fn snowflake_parts_has_timestamp_ms_field() {
    assert!(
        SNOWFLAKE_VR.contains("timestamp_ms:"),
        "SnowflakeParts must have a 'timestamp_ms' field"
    );
}

// ── 10. VCS spec pins the constants ──────────────────────────────────────────

#[test]
fn spec_uses_sequence_bits() {
    assert!(
        SNOWFLAKE_SPEC.contains("SEQUENCE_BITS"),
        "snowflake_sequence_overflow.vr must reference SEQUENCE_BITS"
    );
}

#[test]
fn spec_uses_max_sequence() {
    assert!(
        SNOWFLAKE_SPEC.contains("MAX_SEQUENCE"),
        "snowflake_sequence_overflow.vr must reference MAX_SEQUENCE"
    );
}

#[test]
fn spec_uses_timestamp_shift() {
    assert!(
        SNOWFLAKE_SPEC.contains("TIMESTAMP_SHIFT"),
        "snowflake_sequence_overflow.vr must reference TIMESTAMP_SHIFT"
    );
}

#[test]
fn spec_is_typecheck_pass() {
    assert!(
        SNOWFLAKE_SPEC.contains("@test: typecheck-pass"),
        "snowflake_sequence_overflow.vr must be '@test: typecheck-pass'"
    );
}
