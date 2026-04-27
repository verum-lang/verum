-- =============================================================================
-- 01_spindle_test_setup.sql — initial schema for Spindle Postgres tests
-- =============================================================================
-- Mounted into postgres:16-alpine + postgres:14-alpine via the
-- docker-entrypoint-initdb.d hook. Executed exactly once per fresh
-- volume; later runs of the container reuse the prepared state. The
-- adapter test corpus assumes this baseline exists.
--
-- Spec: internal/specs/database.md §22.1 (integration baseline)
-- =============================================================================

CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- A schema scoped to the integration tests so we never collide with
-- the user-default `public` schema if a developer is poking at the
-- container manually.
CREATE SCHEMA IF NOT EXISTS spindle_it AUTHORIZATION spindle;
SET search_path TO spindle_it, public;

-- Canonical "user" table — exercises common types (int, text, ts).
CREATE TABLE IF NOT EXISTS spindle_it.users (
    id           bigserial    PRIMARY KEY,
    email        text         UNIQUE NOT NULL CHECK (char_length(email) <= 254),
    display_name text         NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 64),
    age          int          CHECK (age IS NULL OR (age >= 0 AND age <= 150)),
    created_at   timestamptz  NOT NULL DEFAULT now()
);

-- "post" table — exercises foreign key + array columns.
CREATE TABLE IF NOT EXISTS spindle_it.posts (
    id          bigserial    PRIMARY KEY,
    author_id   bigint       NOT NULL REFERENCES spindle_it.users(id) ON DELETE CASCADE,
    title       text         NOT NULL,
    body        text         NOT NULL,
    tags        text[]       NOT NULL DEFAULT '{}',
    score       int          NOT NULL DEFAULT 0,
    created_at  timestamptz  NOT NULL DEFAULT now(),
    updated_at  timestamptz
);

CREATE INDEX IF NOT EXISTS posts_author_idx     ON spindle_it.posts(author_id);
CREATE INDEX IF NOT EXISTS posts_created_at_idx ON spindle_it.posts(created_at DESC);

-- Bench fixture — populated lazily by the integration runner.
CREATE TABLE IF NOT EXISTS spindle_it.bench_kv (
    k bigint PRIMARY KEY,
    v text   NOT NULL
);

GRANT USAGE ON SCHEMA spindle_it TO spindle;
GRANT ALL ON ALL TABLES IN SCHEMA spindle_it TO spindle;
GRANT ALL ON ALL SEQUENCES IN SCHEMA spindle_it TO spindle;
