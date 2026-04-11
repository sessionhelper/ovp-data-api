# chronicle-data-api

Internal storage abstraction for the Chronicle toolchain. Rust/Axum service
that owns the durable state for OVP session recordings: Postgres for
structured metadata, Hetzner Object Storage for raw per-speaker PCM audio.

## Role in the stack

```
┌─────────────────┐
│  chronicle-bot  │  (Discord capture bot)
└────────┬────────┘
         │ POST chunks, record events
         ▼
┌─────────────────────┐       ┌────────────────┐
│  chronicle-data-api │──────►│   Postgres 16  │
│  (this service)     │       └────────────────┘
│                     │       ┌────────────────┐
│  shared-secret auth │──────►│  Hetzner S3    │
└────────┬────────────┘       └────────────────┘
         │ WebSocket events (chunk_uploaded, session_finalized, …)
         ▼
┌───────────────────┐   ┌─────────────────┐
│ chronicle-worker  │   │ chronicle-portal│
│ (pipeline)        │   │ (BFF → reads)   │
└───────────────────┘   └─────────────────┘
```

Every other Chronicle service writes to or reads from this service — it is
the single point of contact with durable storage. No service touches
Postgres or S3 directly.

## Key responsibilities

- **Ingest per-speaker audio chunks** (~2MB, ~10.92s of s16le stereo 48kHz)
  uploaded by `chronicle-bot` during recording
- **Stream chunks to S3** under `sessions/{session_id}/audio/{pseudo_id}/chunk_{seq:04}.pcm`
- **Maintain session state** in Postgres: sessions, participants, consent events,
  transcript segments, beats, scenes
- **Broadcast events** (WebSocket + SSE) to downstream consumers:
  `chunk_uploaded`, `session_finalized`, `consent_updated`, `transcript_ready`
- **Enforce authentication**: shared-secret bearer tokens for services,
  token-hashed service sessions in `service_sessions` table
- **Enforce pseudonymization at ingest** per the `CLAUDE.md` convention
  (SHA-256 first-8 of the Discord user ID)
- **Audio mix endpoint** for time-range per-speaker playback

## Auth model

**One level of trust. Shared secret in, session token out. That's it.**

chronicle-data-api is internal-only — it binds to loopback
(`127.0.0.1:8001`) on the VPS and is not reachable from outside the
machine. The only trust boundary is the `SHARED_SECRET` env var. A
client that proves it has the shared secret via `POST /internal/auth`
gets back a session token; subsequent requests send that token as a
`Bearer` header. Every authenticated client is treated identically —
there is no per-service allowlist, no role-based authorization, no
tiered delivery QoS at this layer.

**The internal/external boundary is enforced in `chronicle-portal`, not
here.** The portal authenticates users via Discord OAuth, decides what
each user is allowed to see, and fan-outs filtered events to browsers
via SSE. chronicle-data-api does not know what a user is.

See `sessionhelper-hub/CLAUDE.md` § shared-secret auth for the wire
protocol details.

## Quick start

```bash
cp .env.example .env
# Edit .env with DATABASE_URL, S3 credentials, SHARED_SECRET
cargo run --release
```

The service binds to `0.0.0.0:8001` by default and runs migrations on
startup against the configured Postgres. A healthy start looks like:

```
starting chronicle-data-api bind=0.0.0.0:8001
connected to postgres
database migrations complete
s3 client initialized endpoint=… bucket=ovp-dataset-…
listening addr=0.0.0.0:8001
```

## Env vars

See `.env.example` for the full list. Required:

| Var | Purpose |
|---|---|
| `DATABASE_URL` | Postgres connection URL |
| `BIND_ADDR` | host:port for the Axum listener (default `0.0.0.0:8001`) |
| `S3_ENDPOINT` | S3-compatible endpoint URL (Hetzner: `https://nbg1.your-objectstorage.com`) |
| `S3_ACCESS_KEY` | S3 access key ID |
| `S3_SECRET_KEY` | S3 secret access key |
| `S3_BUCKET` | Target bucket (defaults to `ovp-dataset-raw`) |
| `SHARED_SECRET` | Cross-service auth token (see `sessionhelper-hub/CLAUDE.md`) |
| `RUST_LOG` | tracing filter (e.g. `chronicle_data_api=info,tower_http=info`) |

## Deploy

On tagged `v*` pushes, GitHub Actions builds and pushes to
`ghcr.io/sessionhelper/chronicle-data-api:{latest,dev,vX.Y.Z}`, then
fetches the canonical prod compose file from
`sessionhelper-hub/infra/prod-compose.yml` and restarts the
`chronicle-data-api.service` systemd unit on the prod VPS. See
`.github/workflows/deploy.yml`.

The systemd unit source is in `deploy/chronicle-data-api.service` — note
that despite the name, on the dev VPS the same unit brings up the whole
compose stack (postgres + data-api + bot + worker + feeders).

## Related docs

- [`sessionhelper-hub/ARCHITECTURE.md`](https://github.com/sessionhelper/sessionhelper-hub/blob/main/ARCHITECTURE.md) — cross-service data flow, the canonical system picture
- [`sessionhelper-hub/SPEC.md`](https://github.com/sessionhelper/sessionhelper-hub/blob/main/SPEC.md) — OVP program spec (mission, goals, phases, traceability)
- [`sessionhelper-hub/CLAUDE.md`](https://github.com/sessionhelper/sessionhelper-hub/blob/main/CLAUDE.md) — org-wide Rust + git + secrets conventions
- `CLAUDE.md` (this repo) — component-specific notes
