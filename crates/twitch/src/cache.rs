use dashmap::DashMap;
use std::{sync::Arc, time::Instant};
use tracing::trace;
use twitch_api::helix::chat::BadgeSet;

const EXPIRY_INTERVAL: u64 = 7200;
const BADGES_EXPIRY_INTERVAL: u64 = 3600;

// Banned users are stored as None
#[derive(Clone, Default)]
pub struct UsersCache {
    ids: Arc<DashMap<String, (Instant, Option<String>)>>,
    logins: Arc<DashMap<String, (Instant, Option<String>)>>,
}

impl UsersCache {
    pub fn insert(&self, id: String, name: String) {
        self.insert_optional(Some(id), Some(name));
    }

    pub fn insert_optional(&self, id: Option<String>, name: Option<String>) {
        let inserted_at = Instant::now();

        if let Some(id) = id.clone() {
            self.ids.insert(id, (inserted_at, name.clone()));
        }

        if let Some(name) = name {
            self.logins.insert(name, (inserted_at, id));
        }
    }

    pub fn get_login(&self, id: &str) -> Option<Option<String>> {
        let entry = self.ids.get(id)?;
        if entry.value().0.elapsed().as_secs() > EXPIRY_INTERVAL {
            drop(entry);
            trace!(id, "evicting expired user");
            self.ids.remove(id);
            None
        } else {
            trace!(id, "user found in cache");
            Some(entry.value().1.clone())
        }
    }

    pub fn get_id(&self, name: &str) -> Option<Option<String>> {
        let entry = self.logins.get(name)?;
        if entry.value().0.elapsed().as_secs() > EXPIRY_INTERVAL {
            let key = entry.key().clone();
            drop(entry);
            trace!(login = name, "evicting expired user");
            self.logins.remove(&key);
            None
        } else {
            trace!(login = name, "user found in cache");
            Some(entry.value().1.clone())
        }
    }
}

/// Twitch chat badge sets keyed by channel id, `None` for the global ones.
///
/// Badges are served publicly, so they are cached to keep requests from
/// reaching Helix and eating into the shared app token rate limit.
#[derive(Clone, Default)]
pub struct BadgesCache {
    sets: Arc<DashMap<Option<String>, CachedBadges>>,
}

struct CachedBadges {
    inserted_at: Instant,
    sets: Vec<BadgeSet>,
}

impl BadgesCache {
    pub fn insert(&self, channel_id: Option<&str>, sets: Vec<BadgeSet>) {
        let cached = CachedBadges {
            inserted_at: Instant::now(),
            sets,
        };
        self.sets.insert(channel_id.map(str::to_owned), cached);
    }

    pub fn get(&self, channel_id: Option<&str>) -> Option<Vec<BadgeSet>> {
        let key = channel_id.map(str::to_owned);
        let entry = self.sets.get(&key)?;

        if entry.inserted_at.elapsed().as_secs() > BADGES_EXPIRY_INTERVAL {
            drop(entry);
            trace!(channel_id = ?key, "evicting expired chat badges");
            self.sets.remove(&key);
            None
        } else {
            Some(entry.sets.clone())
        }
    }
}
