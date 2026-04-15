-- Features-pass schema: the first and only migration against the locked
-- chronicle-data-api Features + Interfaces + Behavior spec.
--
-- Highlights:
--   * pseudo_id is 24 hex characters everywhere (spec §10).
--   * No blocklist / global opt-out anywhere (spec scope fence).
--   * Session state machine: recording | uploaded | transcribing |
--     transcribing_failed | transcribed | abandoned | deleted.
--   * `user_display_names` alias table (idempotent PK on (pseudo_id,
--     display_name)).
--   * Chunk storage with server-assigned `seq`, idempotency keys, and
--     the capture metadata columns.
--   * `session_metadata` JSONB blob with ETag versioning.
--   * Uniform CRUD for segments / beats / scenes: `client_id`, `original`,
--     `author_service`, `author_user_pseudo_id`, row ETag.
--   * Mute ranges, participant wipe flags, resource-agnostic audit log,
--     and `users.is_admin` for the admin bootstrap path.

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

-------------------------------------------------------------------------------
-- Users + display names
-------------------------------------------------------------------------------
CREATE TABLE users (
    pseudo_id    TEXT PRIMARY KEY CHECK (pseudo_id ~ '^[0-9a-f]{24}$'),
    is_admin     BOOLEAN     NOT NULL DEFAULT FALSE,
    data_wiped_at TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE user_display_names (
    pseudo_id     TEXT        NOT NULL REFERENCES users(pseudo_id) ON DELETE CASCADE,
    display_name  TEXT        NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    seen_count    INTEGER     NOT NULL DEFAULT 1,
    source        TEXT        NOT NULL CHECK (source IN ('bot','portal_override')),
    PRIMARY KEY (pseudo_id, display_name)
);

-------------------------------------------------------------------------------
-- Sessions + participants
-------------------------------------------------------------------------------
CREATE TABLE sessions (
    id                UUID        PRIMARY KEY,
    guild_id          BIGINT      NOT NULL,
    started_at        TIMESTAMPTZ NOT NULL,
    ended_at          TIMESTAMPTZ,
    abandoned_at      TIMESTAMPTZ,
    deleted_at        TIMESTAMPTZ,
    game_system       TEXT,
    campaign_name     TEXT,
    participant_count INTEGER,
    s3_prefix         TEXT        NOT NULL,
    status            TEXT        NOT NULL DEFAULT 'recording' CHECK (status IN (
        'recording',
        'uploaded',
        'transcribing',
        'transcribing_failed',
        'transcribed',
        'abandoned',
        'deleted'
    )),
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_sessions_guild_status ON sessions(guild_id, status);
CREATE INDEX idx_sessions_status       ON sessions(status);

CREATE TABLE session_participants (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id         UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    pseudo_id          TEXT NOT NULL REFERENCES users(pseudo_id) ON DELETE CASCADE
                              CHECK (pseudo_id ~ '^[0-9a-f]{24}$'),
    consent_scope      TEXT CHECK (consent_scope IN ('full','decline','timed_out')),
    consented_at       TIMESTAMPTZ,
    mid_session_join   BOOLEAN NOT NULL DEFAULT FALSE,
    no_llm_training    BOOLEAN NOT NULL DEFAULT FALSE,
    no_public_release  BOOLEAN NOT NULL DEFAULT FALSE,
    data_wiped_at      TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, pseudo_id)
);
CREATE INDEX idx_participants_session ON session_participants(session_id);
CREATE INDEX idx_participants_pseudo  ON session_participants(pseudo_id);

-------------------------------------------------------------------------------
-- Session metadata blob (ETag-versioned)
-------------------------------------------------------------------------------
CREATE TABLE session_metadata (
    session_id UUID        PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    blob       JSONB       NOT NULL,
    etag       TEXT        NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-------------------------------------------------------------------------------
-- Chunk metadata (S3 is still the bytes store)
-------------------------------------------------------------------------------
CREATE TABLE session_chunks (
    session_id         UUID        NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    pseudo_id          TEXT        NOT NULL CHECK (pseudo_id = 'mixed' OR pseudo_id ~ '^[0-9a-f]{24}$'),
    seq                INTEGER     NOT NULL,
    s3_key             TEXT        NOT NULL,
    size_bytes         INTEGER     NOT NULL,
    capture_started_at TIMESTAMPTZ NOT NULL,
    duration_ms        INTEGER     NOT NULL,
    client_chunk_id    TEXT        NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, pseudo_id, seq),
    UNIQUE (session_id, pseudo_id, client_chunk_id)
);
CREATE INDEX idx_chunks_session ON session_chunks(session_id);

-------------------------------------------------------------------------------
-- Uniform CRUD rows: segments, beats, scenes
-------------------------------------------------------------------------------
CREATE TABLE session_segments (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id              UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    client_id               TEXT NOT NULL,
    pseudo_id               TEXT,
    start_ms                BIGINT NOT NULL,
    end_ms                  BIGINT NOT NULL,
    text                    TEXT NOT NULL,
    confidence              DOUBLE PRECISION,
    flags                   JSONB NOT NULL DEFAULT '{}'::jsonb,
    original                JSONB,
    etag                    TEXT NOT NULL,
    author_service          TEXT NOT NULL,
    author_user_pseudo_id   TEXT CHECK (author_user_pseudo_id IS NULL OR author_user_pseudo_id ~ '^[0-9a-f]{24}$'),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, client_id)
);
CREATE INDEX idx_segments_session_start ON session_segments(session_id, start_ms);

CREATE TABLE session_beats (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id              UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    client_id               TEXT NOT NULL,
    start_ms                BIGINT NOT NULL,
    end_ms                  BIGINT NOT NULL,
    title                   TEXT NOT NULL,
    summary                 TEXT NOT NULL DEFAULT '',
    flags                   JSONB NOT NULL DEFAULT '{}'::jsonb,
    original                JSONB,
    etag                    TEXT NOT NULL,
    author_service          TEXT NOT NULL,
    author_user_pseudo_id   TEXT CHECK (author_user_pseudo_id IS NULL OR author_user_pseudo_id ~ '^[0-9a-f]{24}$'),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, client_id)
);
CREATE INDEX idx_beats_session_start ON session_beats(session_id, start_ms);

CREATE TABLE session_scenes (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id              UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    client_id               TEXT NOT NULL,
    start_ms                BIGINT NOT NULL,
    end_ms                  BIGINT NOT NULL,
    title                   TEXT NOT NULL,
    summary                 TEXT NOT NULL DEFAULT '',
    flags                   JSONB NOT NULL DEFAULT '{}'::jsonb,
    original                JSONB,
    etag                    TEXT NOT NULL,
    author_service          TEXT NOT NULL,
    author_user_pseudo_id   TEXT CHECK (author_user_pseudo_id IS NULL OR author_user_pseudo_id ~ '^[0-9a-f]{24}$'),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, client_id)
);
CREATE INDEX idx_scenes_session_start ON session_scenes(session_id, start_ms);

-------------------------------------------------------------------------------
-- Mute ranges (additive; never rewrites audio)
-------------------------------------------------------------------------------
CREATE TABLE mute_ranges (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id      UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    pseudo_id       TEXT NOT NULL CHECK (pseudo_id ~ '^[0-9a-f]{24}$'),
    start_offset_ms BIGINT NOT NULL,
    end_offset_ms   BIGINT NOT NULL,
    reason          TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_mute_session_participant ON mute_ranges(session_id, pseudo_id);

-------------------------------------------------------------------------------
-- Append-only audit log (resource-agnostic)
-------------------------------------------------------------------------------
CREATE TABLE audit_log (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    at_ts          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    actor_service  TEXT NOT NULL,
    actor_pseudo   TEXT,
    session_id     UUID,
    resource_type  TEXT NOT NULL,
    resource_id    TEXT NOT NULL,
    action         TEXT NOT NULL,
    detail         JSONB
);
CREATE INDEX idx_audit_session   ON audit_log(session_id);
CREATE INDEX idx_audit_resource  ON audit_log(resource_type, resource_id);
CREATE INDEX idx_audit_at_ts     ON audit_log(at_ts DESC);

-------------------------------------------------------------------------------
-- Service sessions: drop 'alive' toggle, use heartbeat freshness only
-------------------------------------------------------------------------------
DROP TABLE IF EXISTS service_sessions CASCADE;
CREATE TABLE service_sessions (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    service_name  TEXT NOT NULL,
    token_hash    TEXT NOT NULL UNIQUE,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_service_sessions_last_seen ON service_sessions(last_seen_at);
