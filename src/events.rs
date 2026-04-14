//! Real-time event bus.
//!
//! Fire-and-forget: events are broadcast once, never persisted. The WS layer
//! reads from a `broadcast::Receiver` and maintains a private per-subscriber
//! queue (see `routes::ws`). Slow subscribers see their oldest pending events
//! dropped — the broadcast bus itself never blocks producers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::ids::PseudoId;
use crate::state::SessionStatus;

/// One event envelope, wire-matching the spec's `{type, at_ts, data}` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    SessionStateChanged {
        at_ts: DateTime<Utc>,
        data: SessionStateChanged,
    },
    ChunkUploaded {
        at_ts: DateTime<Utc>,
        data: ChunkUploaded,
    },
    SegmentCreated {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    SegmentUpdated {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    SegmentDeleted {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    BeatCreated {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    BeatUpdated {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    BeatDeleted {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    SceneCreated {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    SceneUpdated {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    SceneDeleted {
        at_ts: DateTime<Utc>,
        data: ResourceRef,
    },
    MuteRangeCreated {
        at_ts: DateTime<Utc>,
        data: MuteRangeRef,
    },
    MuteRangeDeleted {
        at_ts: DateTime<Utc>,
        data: MuteRangeRef,
    },
    AudioDeleted {
        at_ts: DateTime<Utc>,
        data: AudioDeleted,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStateChanged {
    pub session_id: Uuid,
    pub guild_id: i64,
    pub old: SessionStatus,
    pub new: SessionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkUploaded {
    pub session_id: Uuid,
    pub guild_id: i64,
    pub pseudo_id: PseudoId,
    pub seq: i32,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRef {
    pub session_id: Uuid,
    pub guild_id: i64,
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MuteRangeRef {
    pub session_id: Uuid,
    pub guild_id: i64,
    pub pseudo_id: PseudoId,
    pub range_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeleted {
    pub session_id: Uuid,
    pub guild_id: i64,
    pub pseudo_id: PseudoId,
}

impl Event {
    pub fn session_id(&self) -> Uuid {
        match self {
            Self::SessionStateChanged { data, .. } => data.session_id,
            Self::ChunkUploaded { data, .. } => data.session_id,
            Self::SegmentCreated { data, .. }
            | Self::SegmentUpdated { data, .. }
            | Self::SegmentDeleted { data, .. }
            | Self::BeatCreated { data, .. }
            | Self::BeatUpdated { data, .. }
            | Self::BeatDeleted { data, .. }
            | Self::SceneCreated { data, .. }
            | Self::SceneUpdated { data, .. }
            | Self::SceneDeleted { data, .. } => data.session_id,
            Self::MuteRangeCreated { data, .. } | Self::MuteRangeDeleted { data, .. } => {
                data.session_id
            }
            Self::AudioDeleted { data, .. } => data.session_id,
        }
    }

    pub fn guild_id(&self) -> i64 {
        match self {
            Self::SessionStateChanged { data, .. } => data.guild_id,
            Self::ChunkUploaded { data, .. } => data.guild_id,
            Self::SegmentCreated { data, .. }
            | Self::SegmentUpdated { data, .. }
            | Self::SegmentDeleted { data, .. }
            | Self::BeatCreated { data, .. }
            | Self::BeatUpdated { data, .. }
            | Self::BeatDeleted { data, .. }
            | Self::SceneCreated { data, .. }
            | Self::SceneUpdated { data, .. }
            | Self::SceneDeleted { data, .. } => data.guild_id,
            Self::MuteRangeCreated { data, .. } | Self::MuteRangeDeleted { data, .. } => {
                data.guild_id
            }
            Self::AudioDeleted { data, .. } => data.guild_id,
        }
    }

    /// Stable string name matching the `type` tag in the wire format.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SessionStateChanged { .. } => "session_state_changed",
            Self::ChunkUploaded { .. } => "chunk_uploaded",
            Self::SegmentCreated { .. } => "segment_created",
            Self::SegmentUpdated { .. } => "segment_updated",
            Self::SegmentDeleted { .. } => "segment_deleted",
            Self::BeatCreated { .. } => "beat_created",
            Self::BeatUpdated { .. } => "beat_updated",
            Self::BeatDeleted { .. } => "beat_deleted",
            Self::SceneCreated { .. } => "scene_created",
            Self::SceneUpdated { .. } => "scene_updated",
            Self::SceneDeleted { .. } => "scene_deleted",
            Self::MuteRangeCreated { .. } => "mute_range_created",
            Self::MuteRangeDeleted { .. } => "mute_range_deleted",
            Self::AudioDeleted { .. } => "audio_deleted",
        }
    }
}

pub type EventSender = broadcast::Sender<Event>;

/// Broadcast channel carrying every event. Capacity is intentionally larger
/// than the per-subscriber cap — it's the shared buffer, not a per-subscriber
/// queue.
pub fn create_bus() -> EventSender {
    let (tx, _) = broadcast::channel(4096);
    tx
}
