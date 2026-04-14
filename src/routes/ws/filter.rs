//! Subscription filter set. A subscriber's set of `Subscription`s is matched
//! against each incoming event.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::Event;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub struct Filter {
    #[serde(default)]
    pub guild_id: Option<i64>,
    #[serde(default)]
    pub session_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Subscription {
    pub event: String,
    #[serde(default)]
    pub filter: Filter,
}

#[derive(Debug, Default)]
pub struct SubscriptionSet {
    subs: HashSet<Subscription>,
}

impl SubscriptionSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&mut self, event_names: Vec<String>, filter: Filter) {
        for name in event_names {
            self.subs.insert(Subscription {
                event: name,
                filter: filter.clone(),
            });
        }
    }

    pub fn unsubscribe(&mut self, event_names: Vec<String>) {
        self.subs.retain(|s| !event_names.contains(&s.event));
    }

    pub fn active(&self) -> Vec<Subscription> {
        self.subs.iter().cloned().collect()
    }

    pub fn matches(&self, event: &Event) -> bool {
        let name = event.name();
        let guild = event.guild_id();
        let session = event.session_id();

        self.subs.iter().any(|s| {
            if s.event != name {
                return false;
            }
            match (s.filter.guild_id, s.filter.session_id) {
                (None, None) => true,
                (Some(g), None) => g == guild,
                (None, Some(sid)) => sid == session,
                (Some(g), Some(sid)) => g == guild && sid == session,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ChunkUploaded, Event, SessionStateChanged};
    use crate::ids::PseudoId;
    use crate::state::SessionStatus;
    use chrono::Utc;

    fn sample_event(kind: &str, guild_id: i64, session_id: Uuid) -> Event {
        match kind {
            "chunk_uploaded" => Event::ChunkUploaded {
                at_ts: Utc::now(),
                data: ChunkUploaded {
                    session_id,
                    guild_id,
                    pseudo_id: PseudoId::new("0123456789abcdef01234567").unwrap(),
                    seq: 1,
                    size_bytes: 10,
                },
            },
            "session_state_changed" => Event::SessionStateChanged {
                at_ts: Utc::now(),
                data: SessionStateChanged {
                    session_id,
                    guild_id,
                    old: SessionStatus::Recording,
                    new: SessionStatus::Uploaded,
                },
            },
            _ => panic!("unknown test event"),
        }
    }

    #[test]
    fn matches_by_event_name() {
        let mut set = SubscriptionSet::new();
        set.subscribe(vec!["chunk_uploaded".to_string()], Filter::default());
        let guild = 1;
        let sid = Uuid::new_v4();
        assert!(set.matches(&sample_event("chunk_uploaded", guild, sid)));
        assert!(!set.matches(&sample_event("session_state_changed", guild, sid)));
    }

    #[test]
    fn filters_by_guild_id() {
        let mut set = SubscriptionSet::new();
        set.subscribe(
            vec!["chunk_uploaded".to_string()],
            Filter {
                guild_id: Some(7),
                ..Default::default()
            },
        );
        let sid = Uuid::new_v4();
        assert!(set.matches(&sample_event("chunk_uploaded", 7, sid)));
        assert!(!set.matches(&sample_event("chunk_uploaded", 8, sid)));
    }

    #[test]
    fn unsubscribe_removes_all_with_event_name() {
        let mut set = SubscriptionSet::new();
        set.subscribe(
            vec!["chunk_uploaded".to_string()],
            Filter {
                guild_id: Some(1),
                ..Default::default()
            },
        );
        set.subscribe(
            vec!["chunk_uploaded".to_string()],
            Filter {
                guild_id: Some(2),
                ..Default::default()
            },
        );
        set.unsubscribe(vec!["chunk_uploaded".to_string()]);
        assert!(set.active().is_empty());
    }
}
