CREATE TABLE service_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    service_name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    alive BOOLEAN NOT NULL DEFAULT true
);

CREATE INDEX idx_service_sessions_token ON service_sessions(token_hash);
CREATE INDEX idx_service_sessions_alive ON service_sessions(alive) WHERE alive = true;
