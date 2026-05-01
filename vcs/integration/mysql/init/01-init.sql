-- =============================================================
-- Spindle MySQL integration-test schema.
-- =============================================================
--
-- Runs once on first container boot via docker-entrypoint-initdb.d.
-- Creates the app + read-only roles (admin already created via
-- MYSQL_USER / MYSQL_PASSWORD env). Schema covers the codec types
-- the wire layer supports + a constraint table for capability
-- tests.
-- =============================================================

USE spindle_test;

-- App-tier role — DML on the test schema.
CREATE USER 'spindle_app'@'%' IDENTIFIED WITH caching_sha2_password BY 'spindle_app_pw';
GRANT SELECT, INSERT, UPDATE, DELETE, REFERENCES ON spindle_test.* TO 'spindle_app'@'%';

-- Read-only role — capability narrowing tests.
CREATE USER 'spindle_ro'@'%' IDENTIFIED WITH caching_sha2_password BY 'spindle_ro_pw';
GRANT SELECT ON spindle_test.* TO 'spindle_ro'@'%';

-- Admin already has every privilege via MYSQL_USER + MYSQL_PASSWORD.
GRANT ALL PRIVILEGES ON spindle_test.* TO 'spindle_admin'@'%';

-- REPLICATION SLAVE allows COM_BINLOG_DUMP[_GTID] and the
-- companion replication helpers used by the binlog module
-- (P-PROD-MYSQL-BINLOG-V0). Without this, the dump command
-- returns ER_SPECIFIC_ACCESS_DENIED.
GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO 'spindle_admin'@'%';

FLUSH PRIVILEGES;

-- Phase-2 type-coverage smoke table.
CREATE TABLE phase_codec_smoke (
    id          INT          PRIMARY KEY,
    is_active   BOOLEAN      NOT NULL DEFAULT TRUE,
    big         BIGINT       NOT NULL,
    short       SMALLINT     NOT NULL,
    flt         FLOAT        NOT NULL,
    dbl         DOUBLE       NOT NULL,
    name_text   VARCHAR(255) NOT NULL,
    binary_data VARBINARY(255) NOT NULL,
    created_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    naive_dt    DATETIME     NOT NULL DEFAULT CURRENT_TIMESTAMP,
    only_date   DATE         NOT NULL DEFAULT (CURRENT_DATE),
    only_time   TIME         NOT NULL DEFAULT '00:00:00',
    jb          JSON         NOT NULL DEFAULT (JSON_OBJECT())
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

GRANT SELECT, INSERT, UPDATE, DELETE ON phase_codec_smoke TO 'spindle_app'@'%';
GRANT SELECT ON phase_codec_smoke TO 'spindle_ro'@'%';

-- Constraints table for the capability-violation test (UNIQUE +
-- CHECK constraints exercise commit #13's ConstraintViolation
-- accessor).
CREATE TABLE accounts (
    id      BIGINT       AUTO_INCREMENT PRIMARY KEY,
    email   VARCHAR(254) NOT NULL UNIQUE,
    age     INT          NULL CHECK (age IS NULL OR (age >= 0 AND age <= 150)),
    balance DECIMAL(20,4) NOT NULL DEFAULT 0 CHECK (balance >= 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

GRANT SELECT, INSERT, UPDATE, DELETE ON accounts TO 'spindle_app'@'%';
GRANT SELECT ON accounts TO 'spindle_ro'@'%';
