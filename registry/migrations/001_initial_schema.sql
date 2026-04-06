-- Verum Registry Database Schema
-- PostgreSQL 17+
--
-- Migration: 001_initial_schema
-- Created: 2026-03-15
--
-- This migration creates all tables required by the Verum package registry.
-- Domain types: domain/user.vr, domain/package.vr, domain/version.vr,
--               domain/webhook.vr, domain/checksum.vr
-- Services:     services/sumdb_service.vr, services/artifact_service.vr,
--               services/security_service.vr

BEGIN;

-- ============================================================================
-- Extensions
-- ============================================================================

CREATE EXTENSION IF NOT EXISTS "pgcrypto";  -- gen_random_uuid()

-- ============================================================================
-- Users
-- ============================================================================
-- Domain: domain/user.vr :: User
-- Fields map to: id, username (Username), email (Email), role (UserRole),
--   created_at, is_active

CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username      TEXT UNIQUE NOT NULL CHECK (length(username) BETWEEN 2 AND 64),
    email         TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'regular'
                      CHECK (role IN ('regular', 'moderator', 'admin')),
    bio           TEXT NOT NULL DEFAULT '',
    avatar_url    TEXT NOT NULL DEFAULT '',
    is_active     BOOLEAN NOT NULL DEFAULT TRUE,
    suspension_reason TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE users IS 'Registry users. Maps to domain/user.vr :: User.';

-- ============================================================================
-- API Tokens
-- ============================================================================
-- Domain: domain/user.vr :: ApiToken
-- Scopes validated by CHECK to match TokenScope refinement type.

CREATE TABLE api_tokens (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT NOT NULL CHECK (length(name) >= 1),
    token_hash    TEXT UNIQUE NOT NULL,
    scopes        TEXT[] NOT NULL DEFAULT ARRAY['publish', 'yank']
                      CHECK (scopes <@ ARRAY['publish', 'yank', 'read', 'admin']::TEXT[]),
    is_active     BOOLEAN NOT NULL DEFAULT TRUE,
    expires_at    TIMESTAMPTZ,         -- NULL = never expires
    last_used_at  TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE api_tokens IS 'API authentication tokens. Maps to domain/user.vr :: ApiToken.';

CREATE INDEX idx_api_tokens_user_id ON api_tokens(user_id);
CREATE INDEX idx_api_tokens_token_hash ON api_tokens(token_hash);

-- ============================================================================
-- Packages
-- ============================================================================
-- Domain: domain/package.vr :: Package
-- Name validated to match ValidatedPackageName refinement type constraints.

CREATE TABLE packages (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT UNIQUE NOT NULL
                        CHECK (length(name) BETWEEN 2 AND 64
                               AND name !~ '^-'
                               AND name !~ '-$'
                               AND name !~ '--'),
    description     TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 1000),
    license         TEXT NOT NULL DEFAULT 'MIT' CHECK (length(license) BETWEEN 1 AND 64),
    repository_url  TEXT NOT NULL DEFAULT '' CHECK (length(repository_url) <= 2048),
    homepage_url    TEXT NOT NULL DEFAULT '' CHECK (length(homepage_url) <= 2048),
    documentation_url TEXT NOT NULL DEFAULT '' CHECK (length(documentation_url) <= 2048),
    keywords        TEXT[] NOT NULL DEFAULT '{}',
    categories      TEXT[] NOT NULL DEFAULT '{}',
    downloads       BIGINT NOT NULL DEFAULT 0 CHECK (downloads >= 0),
    latest_version  TEXT NOT NULL DEFAULT '0.0.0',
    is_blocked      BOOLEAN NOT NULL DEFAULT FALSE,
    block_reason    TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE packages IS 'Registered packages. Maps to domain/package.vr :: Package.';

CREATE INDEX idx_packages_name ON packages(name);
CREATE INDEX idx_packages_downloads ON packages(downloads DESC);
CREATE INDEX idx_packages_created_at ON packages(created_at DESC);
CREATE INDEX idx_packages_keywords ON packages USING GIN(keywords);
CREATE INDEX idx_packages_categories ON packages USING GIN(categories);

-- ============================================================================
-- Package Versions
-- ============================================================================
-- Domain: domain/package.vr :: PackageVersion
-- Version components validated per SemVer spec (domain/version.vr).

CREATE TABLE versions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    package_id      UUID NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    version         TEXT NOT NULL CHECK (length(version) >= 5),  -- "0.0.0" minimum
    major           INT NOT NULL CHECK (major >= 0),
    minor           INT NOT NULL CHECK (minor >= 0),
    patch           INT NOT NULL CHECK (patch >= 0),
    pre_release     TEXT NOT NULL DEFAULT '',
    build_metadata  TEXT NOT NULL DEFAULT '',
    description     TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 1000),
    readme          TEXT NOT NULL DEFAULT '',
    checksum        TEXT NOT NULL CHECK (length(checksum) = 64),  -- SHA-256 hex
    tarball_url     TEXT NOT NULL DEFAULT '',
    tarball_size    BIGINT NOT NULL DEFAULT 0 CHECK (tarball_size >= 0),
    published_by    UUID NOT NULL REFERENCES users(id),
    downloads       BIGINT NOT NULL DEFAULT 0 CHECK (downloads >= 0),
    yanked          BOOLEAN NOT NULL DEFAULT FALSE,
    yank_reason     TEXT,
    published_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (package_id, version)
);

COMMENT ON TABLE versions IS 'Package versions. Maps to domain/package.vr :: PackageVersion and domain/version.vr :: SemVer.';

CREATE INDEX idx_versions_package_id ON versions(package_id);
CREATE INDEX idx_versions_published_at ON versions(published_at DESC);
CREATE INDEX idx_versions_semver ON versions(package_id, major DESC, minor DESC, patch DESC);

-- ============================================================================
-- Package Owners
-- ============================================================================
-- Many-to-many relationship: packages <-> users

CREATE TABLE package_owners (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    package_id  UUID NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL DEFAULT 'owner' CHECK (role IN ('owner', 'maintainer')),
    added_by    UUID REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (package_id, user_id)
);

COMMENT ON TABLE package_owners IS 'Package ownership. Multiple users can co-own a package.';

CREATE INDEX idx_package_owners_package_id ON package_owners(package_id);
CREATE INDEX idx_package_owners_user_id ON package_owners(user_id);

-- ============================================================================
-- Dependencies
-- ============================================================================
-- Domain: domain/package.vr :: Dependency

CREATE TABLE dependencies (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version_id          UUID NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    dependency_name     TEXT NOT NULL CHECK (length(dependency_name) BETWEEN 2 AND 64),
    version_constraint  TEXT NOT NULL CHECK (length(version_constraint) BETWEEN 1 AND 64),
    optional            BOOLEAN NOT NULL DEFAULT FALSE,
    is_dev              BOOLEAN NOT NULL DEFAULT FALSE,

    UNIQUE (version_id, dependency_name)
);

COMMENT ON TABLE dependencies IS 'Package dependencies per version. Maps to domain/package.vr :: Dependency.';

CREATE INDEX idx_dependencies_version_id ON dependencies(version_id);
CREATE INDEX idx_dependencies_dependency_name ON dependencies(dependency_name);

-- ============================================================================
-- Download Stats (daily aggregates)
-- ============================================================================

CREATE TABLE download_stats (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    package_id  UUID NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    version_id  UUID REFERENCES versions(id) ON DELETE SET NULL,
    date        DATE NOT NULL,
    count       BIGINT NOT NULL DEFAULT 0 CHECK (count >= 0),

    UNIQUE (package_id, version_id, date)
);

COMMENT ON TABLE download_stats IS 'Daily download aggregates for analytics.';

CREATE INDEX idx_download_stats_package_date ON download_stats(package_id, date DESC);
CREATE INDEX idx_download_stats_date ON download_stats(date DESC);

-- ============================================================================
-- Webhooks
-- ============================================================================
-- Domain: domain/webhook.vr :: Webhook

CREATE TABLE webhooks (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id             UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    package_id          UUID REFERENCES packages(id) ON DELETE CASCADE,
    name                TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 128),
    url                 TEXT NOT NULL CHECK (length(url) BETWEEN 10 AND 2048),
    events              TEXT[] NOT NULL DEFAULT ARRAY['package.published']
                            CHECK (events <@ ARRAY[
                                'webhook.test', 'package.published', 'package.yanked',
                                'package.updated', 'package.deleted',
                                'owner.added', 'owner.removed'
                            ]::TEXT[]),
    secret              TEXT NOT NULL,
    is_active           BOOLEAN NOT NULL DEFAULT TRUE,
    last_triggered_at   TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE webhooks IS 'Webhook subscriptions. Maps to domain/webhook.vr :: Webhook.';

CREATE INDEX idx_webhooks_user_id ON webhooks(user_id);
CREATE INDEX idx_webhooks_package_id ON webhooks(package_id);

-- ============================================================================
-- Webhook Deliveries
-- ============================================================================
-- Domain: domain/webhook.vr :: WebhookDelivery

CREATE TABLE webhook_deliveries (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    webhook_id        UUID NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event             TEXT NOT NULL,
    payload           JSONB NOT NULL,
    success           BOOLEAN NOT NULL,
    status_code       INT,
    response_time_ms  INT,
    error_message     TEXT,
    delivered_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE webhook_deliveries IS 'Webhook delivery attempts and results. Maps to domain/webhook.vr :: WebhookDelivery.';

CREATE INDEX idx_webhook_deliveries_webhook_id ON webhook_deliveries(webhook_id);
CREATE INDEX idx_webhook_deliveries_delivered_at ON webhook_deliveries(delivered_at DESC);

-- ============================================================================
-- Package Flags (moderation)
-- ============================================================================

CREATE TABLE package_flags (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    package_id      UUID NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    reporter_id     UUID NOT NULL REFERENCES users(id),
    reason          TEXT NOT NULL DEFAULT 'other'
                        CHECK (reason IN ('malware', 'typosquatting', 'spam',
                                          'license_violation', 'name_squatting', 'other')),
    description     TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 2000),
    status          TEXT NOT NULL DEFAULT 'open'
                        CHECK (status IN ('open', 'investigating', 'resolved', 'dismissed')),
    resolved_by     UUID REFERENCES users(id),
    resolution_note TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at     TIMESTAMPTZ
);

COMMENT ON TABLE package_flags IS 'Moderation flags for reported packages.';

CREATE INDEX idx_package_flags_package_id ON package_flags(package_id);
CREATE INDEX idx_package_flags_status ON package_flags(status) WHERE status != 'resolved';

-- ============================================================================
-- Sumdb: Transparent Log Entries
-- ============================================================================
-- Service: services/sumdb_service.vr :: SumdbEntry

CREATE TABLE sumdb_entries (
    entry_id    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    package     TEXT NOT NULL,
    version     TEXT NOT NULL,
    checksum    TEXT NOT NULL CHECK (length(checksum) = 64),
    hash        TEXT NOT NULL CHECK (length(hash) = 64),
    timestamp   TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (package, version)
);

COMMENT ON TABLE sumdb_entries IS 'Append-only transparent log of package checksums. Maps to services/sumdb_service.vr :: SumdbEntry.';

CREATE INDEX idx_sumdb_entries_package_version ON sumdb_entries(package, version);

-- ============================================================================
-- Sumdb: Merkle Tree Nodes
-- ============================================================================
-- Service: services/sumdb_service.vr (internal Merkle tree structure)

CREATE TABLE sumdb_merkle (
    level       INT NOT NULL,
    node_index  BIGINT NOT NULL,
    hash        TEXT NOT NULL CHECK (length(hash) = 64),

    PRIMARY KEY (level, node_index)
);

COMMENT ON TABLE sumdb_merkle IS 'Merkle tree nodes for sumdb inclusion proofs.';

-- ============================================================================
-- Sumdb: Signed Tree Heads
-- ============================================================================
-- Service: services/sumdb_service.vr :: TreeHead

CREATE TABLE sumdb_tree_head (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    tree_size   BIGINT NOT NULL CHECK (tree_size >= 0),
    root_hash   TEXT NOT NULL,
    timestamp   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    signature   TEXT NOT NULL
);

COMMENT ON TABLE sumdb_tree_head IS 'Signed Merkle tree heads for transparent log consistency. Maps to services/sumdb_service.vr :: TreeHead.';

CREATE INDEX idx_sumdb_tree_head_size ON sumdb_tree_head(tree_size DESC);

-- ============================================================================
-- CBGR Profiles
-- ============================================================================
-- Service: services/artifact_service.vr :: CbgrProfile

CREATE TABLE cbgr_profiles (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version_id          UUID NOT NULL REFERENCES versions(id) ON DELETE CASCADE UNIQUE,
    avg_overhead_ns     INT NOT NULL CHECK (avg_overhead_ns >= 0),
    p50_ns              INT NOT NULL CHECK (p50_ns >= 0),
    p95_ns              INT NOT NULL CHECK (p95_ns >= 0),
    p99_ns              INT NOT NULL CHECK (p99_ns >= 0),
    total_refs          INT NOT NULL DEFAULT 0 CHECK (total_refs >= 0),
    promotable_refs     INT NOT NULL DEFAULT 0 CHECK (promotable_refs >= 0),
    memory_overhead_pct REAL NOT NULL DEFAULT 0.0 CHECK (memory_overhead_pct >= 0.0),
    hot_paths           JSONB NOT NULL DEFAULT '[]',
    profiled_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE cbgr_profiles IS 'CBGR reference overhead profiles per version. Maps to services/artifact_service.vr :: CbgrProfile.';

CREATE INDEX idx_cbgr_profiles_version_id ON cbgr_profiles(version_id);

-- ============================================================================
-- Verification Proofs
-- ============================================================================
-- Service: services/artifact_service.vr :: ProofArtifact

CREATE TABLE verification_proofs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version_id          UUID NOT NULL REFERENCES versions(id) ON DELETE CASCADE,
    subject             TEXT NOT NULL,
    format              TEXT NOT NULL CHECK (format IN ('smt2', 'coq', 'lean4', 'dedukti', 'metamath')),
    solver              TEXT NOT NULL,
    solver_version      TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('verified', 'timeout', 'unknown', 'failed')),
    failure_reason      TEXT,
    verification_time_ms INT NOT NULL DEFAULT 0 CHECK (verification_time_ms >= 0),
    properties          TEXT[] NOT NULL DEFAULT '{}',
    proof_storage_key   TEXT NOT NULL,  -- Storage path for proof data
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE verification_proofs IS 'Formal verification proof artifacts per version. Maps to services/artifact_service.vr :: ProofArtifact.';

CREATE INDEX idx_verification_proofs_version_id ON verification_proofs(version_id);
CREATE INDEX idx_verification_proofs_status ON verification_proofs(status);

-- ============================================================================
-- OIDC Trusted Publishers
-- ============================================================================
-- Handler: handlers/oidc.vr

CREATE TABLE oidc_trusted_publishers (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    package_id      UUID NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    issuer          TEXT NOT NULL CHECK (issuer IN (
                        'https://token.actions.githubusercontent.com',
                        'https://gitlab.com'
                    )),
    repository_owner TEXT NOT NULL,
    repository_name  TEXT NOT NULL,
    workflow_name    TEXT,  -- NULL = any workflow allowed
    environment      TEXT,  -- NULL = any environment allowed
    created_by       UUID NOT NULL REFERENCES users(id),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (package_id, issuer, repository_owner, repository_name)
);

COMMENT ON TABLE oidc_trusted_publishers IS 'OIDC trusted publisher configurations for CI/CD keyless publishing.';

CREATE INDEX idx_oidc_trusted_publishers_package_id ON oidc_trusted_publishers(package_id);

-- ============================================================================
-- Updated-at trigger
-- ============================================================================

CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TRIGGER trg_packages_updated_at
    BEFORE UPDATE ON packages
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

COMMIT;
