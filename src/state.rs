//! Session state machine.
//!
//! Valid states per the locked spec §Features 2:
//!
//! ```text
//! recording ──finalize──> uploaded ──claim──> transcribing ──> transcribed
//!    │                                            │
//!    │                                            └── error ──> transcribing_failed ──retry──> transcribing
//!    │
//!    └── catastrophic / manual discard ──> abandoned
//!                                             │
//!                                             └── resume (within RESUME_TTL) ──> recording
//! ```
//!
//! `deleted` is a terminal tombstone reachable from any state (via the
//! delete cascade) and accepts no further transitions.
//!
//! The table of valid transitions is authoritative — clients that propose an
//! illegal transition receive 409 Conflict.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Recording,
    Uploaded,
    Transcribing,
    TranscribingFailed,
    Transcribed,
    Abandoned,
    Deleted,
}

impl SessionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recording => "recording",
            Self::Uploaded => "uploaded",
            Self::Transcribing => "transcribing",
            Self::TranscribingFailed => "transcribing_failed",
            Self::Transcribed => "transcribed",
            Self::Abandoned => "abandoned",
            Self::Deleted => "deleted",
        }
    }
}

impl fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown session status: {0}")]
pub struct UnknownStatus(pub String);

impl FromStr for SessionStatus {
    type Err = UnknownStatus;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "recording" => Self::Recording,
            "uploaded" => Self::Uploaded,
            "transcribing" => Self::Transcribing,
            "transcribing_failed" => Self::TranscribingFailed,
            "transcribed" => Self::Transcribed,
            "abandoned" => Self::Abandoned,
            "deleted" => Self::Deleted,
            other => return Err(UnknownStatus(other.to_string())),
        })
    }
}

impl sqlx::Type<sqlx::Postgres> for SessionStatus {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for SessionStatus {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(SessionStatus::from_str(&s)?)
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for SessionStatus {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        <&str as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.as_str(), buf)
    }
}

/// Is `to` a legal transition from `from`?
///
/// Implemented as an exhaustive match on `from` so adding a new status forces
/// the compiler to demand we think about its outgoing edges.
pub fn can_transition(from: SessionStatus, to: SessionStatus) -> bool {
    use SessionStatus::*;
    match from {
        Recording => matches!(to, Uploaded | Abandoned | Deleted),
        Uploaded => matches!(to, Transcribing | Abandoned | Deleted),
        Transcribing => matches!(
            to,
            // self-transition covers the worker-orphan re-claim case:
            // a second worker claims and re-enters `transcribing` without
            // a status change being an error.
            Transcribing | Transcribed | TranscribingFailed | Deleted
        ),
        TranscribingFailed => matches!(to, Transcribing | Deleted),
        Transcribed => matches!(to, Deleted),
        // `abandoned → recording` is expressed only via POST /resume (with
        // TTL enforcement); raw PATCH never sees it.
        Abandoned => matches!(to, Deleted),
        // Deleted is terminal.
        Deleted => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<SessionStatus> {
        use SessionStatus::*;
        vec![
            Recording,
            Uploaded,
            Transcribing,
            TranscribingFailed,
            Transcribed,
            Abandoned,
            Deleted,
        ]
    }

    #[test]
    fn happy_path_transitions() {
        use SessionStatus::*;
        assert!(can_transition(Recording, Uploaded));
        assert!(can_transition(Uploaded, Transcribing));
        assert!(can_transition(Transcribing, Transcribed));
    }

    #[test]
    fn transcribing_self_transition_allowed() {
        use SessionStatus::*;
        assert!(can_transition(Transcribing, Transcribing));
    }

    #[test]
    fn error_retry_loop() {
        use SessionStatus::*;
        assert!(can_transition(Transcribing, TranscribingFailed));
        assert!(can_transition(TranscribingFailed, Transcribing));
    }

    #[test]
    fn abandoned_is_pathological_via_patch() {
        use SessionStatus::*;
        // Resume uses its own POST endpoint, not PATCH.
        assert!(!can_transition(Abandoned, Recording));
    }

    #[test]
    fn deleted_is_terminal() {
        use SessionStatus::*;
        for target in all() {
            assert!(!can_transition(Deleted, target), "{target} from Deleted");
        }
    }

    #[test]
    fn no_self_transition_except_transcribing() {
        use SessionStatus::*;
        for s in all() {
            if s == Transcribing {
                continue;
            }
            assert!(!can_transition(s, s), "{s} → {s} must be illegal");
        }
    }

    #[test]
    fn every_state_reaches_deleted() {
        use SessionStatus::*;
        for s in all() {
            if s == Deleted {
                continue;
            }
            assert!(can_transition(s, Deleted), "{s} → Deleted");
        }
    }
}
