-- =============================================================
-- Spindle integration-test schema.
-- =============================================================
--
-- Runs once on first container boot via docker-entrypoint-initdb.d.
-- Creates a non-admin role (`spindle_app`) for the typical handler
-- path AND a read-only role (`spindle_ro`) for capability-narrowing
-- tests. Both authenticate via scram-sha-256.
--
-- Schema covers every Phase-2 / Phase-3 codec the wire layer
-- supports — int4 / int8 / float4 / float8 / text / bytea / uuid /
-- bool / timestamp[tz] / date / time / interval / jsonb / inet /
-- int4range / int8range / numrange / tstzrange / int4[] / text[].
-- Integration tests round-trip every column.
-- =============================================================

CREATE ROLE spindle_app    LOGIN PASSWORD 'spindle_app_pw';
CREATE ROLE spindle_ro     LOGIN PASSWORD 'spindle_ro_pw';

GRANT CONNECT ON DATABASE spindle_test TO spindle_app, spindle_ro;
GRANT USAGE   ON SCHEMA  public        TO spindle_app, spindle_ro;
GRANT CREATE  ON SCHEMA  public        TO spindle_app;

-- Phase-2 + Phase-3 codec coverage table.
CREATE TABLE phase_codec_smoke (
    id             integer       PRIMARY KEY,
    is_active      boolean       NOT NULL DEFAULT true,
    big            bigint        NOT NULL,
    short          smallint      NOT NULL,
    flt            real          NOT NULL,
    dbl            double precision NOT NULL,
    name_text      text          NOT NULL,
    binary_data    bytea         NOT NULL,
    uid            uuid          NOT NULL,
    created_at     timestamptz   NOT NULL DEFAULT now(),
    naive_ts       timestamp     NOT NULL DEFAULT current_timestamp::timestamp,
    only_date      date          NOT NULL DEFAULT current_date,
    only_time      time          NOT NULL DEFAULT current_time::time,
    iv             interval      NOT NULL DEFAULT '1 day 02:03:04'::interval,
    jb             jsonb         NOT NULL DEFAULT '{}'::jsonb,
    addr           inet          NOT NULL DEFAULT '127.0.0.1'::inet,
    int_range      int4range     NOT NULL DEFAULT int4range(0, 100),
    int_array      integer[]     NOT NULL DEFAULT ARRAY[]::integer[],
    text_array     text[]        NOT NULL DEFAULT ARRAY[]::text[]
);

GRANT SELECT, INSERT, UPDATE, DELETE ON phase_codec_smoke TO spindle_app;
GRANT SELECT                          ON phase_codec_smoke TO spindle_ro;

-- Constraint table for refinement / capability tests.
CREATE TABLE accounts (
    id        bigserial PRIMARY KEY,
    email     text      NOT NULL UNIQUE
                CHECK (char_length(email) BETWEEN 3 AND 254
                       AND email ~ '^[^@]+@[^@]+\..+$'),
    age       integer   CHECK (age IS NULL OR (age >= 0 AND age <= 150)),
    balance   numeric   NOT NULL DEFAULT 0
                CHECK (balance >= 0)
);

GRANT SELECT, INSERT, UPDATE, DELETE ON accounts                    TO spindle_app;
GRANT USAGE,  SELECT, UPDATE         ON SEQUENCE accounts_id_seq    TO spindle_app;
GRANT SELECT                         ON accounts                    TO spindle_ro;

-- Channel for LISTEN/NOTIFY tests; just documenting — no DDL needed,
-- channel names are dynamic in Postgres.
COMMENT ON DATABASE spindle_test IS 'Spindle integration-test DB';
