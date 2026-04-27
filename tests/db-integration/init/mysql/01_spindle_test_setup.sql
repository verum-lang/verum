-- =============================================================================
-- 01_spindle_test_setup.sql — initial schema for Spindle MySQL tests
-- =============================================================================
-- Mounted into mysql:8.0 via docker-entrypoint-initdb.d. Runs exactly
-- once on first container boot per volume.
--
-- Spec: internal/specs/database.md §22.1 (integration baseline)
-- =============================================================================

CREATE DATABASE IF NOT EXISTS spindle_test
  DEFAULT CHARACTER SET utf8mb4
  DEFAULT COLLATE utf8mb4_0900_as_cs;

USE spindle_test;

-- Mirror of the Postgres baseline (modulo dialect).
CREATE TABLE IF NOT EXISTS users (
    id           BIGINT       NOT NULL AUTO_INCREMENT PRIMARY KEY,
    email        VARCHAR(254) NOT NULL UNIQUE,
    display_name VARCHAR(64)  NOT NULL,
    age          INT          NULL,
    created_at   TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT users_email_len    CHECK (CHAR_LENGTH(email) <= 254),
    CONSTRAINT users_display_len  CHECK (CHAR_LENGTH(display_name) BETWEEN 1 AND 64),
    CONSTRAINT users_age_range    CHECK (age IS NULL OR (age >= 0 AND age <= 150))
) ENGINE=InnoDB;

CREATE TABLE IF NOT EXISTS posts (
    id          BIGINT       NOT NULL AUTO_INCREMENT PRIMARY KEY,
    author_id   BIGINT       NOT NULL,
    title       VARCHAR(255) NOT NULL,
    body        MEDIUMTEXT   NOT NULL,
    score       INT          NOT NULL DEFAULT 0,
    created_at  TIMESTAMP    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP    NULL,
    CONSTRAINT posts_author_fk FOREIGN KEY (author_id) REFERENCES users(id) ON DELETE CASCADE,
    INDEX posts_author_idx (author_id),
    INDEX posts_created_idx (created_at DESC)
) ENGINE=InnoDB;

-- Bench fixture.
CREATE TABLE IF NOT EXISTS bench_kv (
    k BIGINT NOT NULL PRIMARY KEY,
    v TEXT   NOT NULL
) ENGINE=InnoDB;

GRANT ALL PRIVILEGES ON spindle_test.* TO 'spindle'@'%';
FLUSH PRIVILEGES;
