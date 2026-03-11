//! In-memory outbound OAuth2 access-token cache with per-key refresh locks.

use std::{collections::HashMap, sync::Arc};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use secrecy::SecretString;
use tokio::sync::{Mutex, RwLock};

use super::config::OutboundOauth2CacheKey;

type RefreshLockMap = HashMap<OutboundOauth2CacheKey, Arc<Mutex<()>>>;

#[derive(Clone, Debug)]
pub(crate) struct CachedOutboundToken {
    pub(crate) access_token: SecretString,
    pub(crate) refresh_token: Option<SecretString>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
}

impl CachedOutboundToken {
    pub(crate) fn is_fresh(&self, now: DateTime<Utc>, skew_sec: u64) -> bool {
        let Some(expires_at) = self.expires_at else {
            return true;
        };
        let Ok(skew) = ChronoDuration::from_std(std::time::Duration::from_secs(skew_sec)) else {
            return false;
        };
        now + skew < expires_at
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OutboundTokenCache {
    entries: Arc<RwLock<HashMap<OutboundOauth2CacheKey, CachedOutboundToken>>>,
    refresh_locks: Arc<Mutex<RefreshLockMap>>,
}

impl OutboundTokenCache {
    pub(crate) async fn get(&self, key: &OutboundOauth2CacheKey) -> Option<CachedOutboundToken> {
        self.entries.read().await.get(key).cloned()
    }

    pub(crate) async fn get_fresh(
        &self,
        key: &OutboundOauth2CacheKey,
        now: DateTime<Utc>,
        skew_sec: u64,
    ) -> Option<CachedOutboundToken> {
        self.get(key)
            .await
            .filter(|entry| entry.is_fresh(now, skew_sec))
    }

    pub(crate) async fn upsert(
        &self,
        key: OutboundOauth2CacheKey,
        entry: CachedOutboundToken,
    ) -> Option<CachedOutboundToken> {
        self.entries.write().await.insert(key, entry)
    }

    pub(crate) async fn refresh_lock(&self, key: &OutboundOauth2CacheKey) -> Arc<Mutex<()>> {
        let mut guard = self.refresh_locks.lock().await;
        Arc::clone(
            guard
                .entry(key.clone())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }
}

#[cfg(test)]
// Inline tests verify private cache freshness and per-key lock behavior.
mod tests {
    use super::{CachedOutboundToken, OutboundTokenCache};
    use crate::auth::outbound::config::OutboundOauth2CacheKey;
    use crate::config::UpstreamOauth2GrantType;
    use chrono::{Duration as ChronoDuration, Utc};
    use secrecy::{ExposeSecret, SecretString};
    use std::sync::Arc;

    fn cache_key() -> OutboundOauth2CacheKey {
        OutboundOauth2CacheKey {
            upstream_name: "reports".to_owned(),
            grant: UpstreamOauth2GrantType::ClientCredentials,
            scopes: vec!["read".to_owned()],
            audience: None,
            resource: None,
            extra_token_params: Vec::new(),
        }
    }

    #[test]
    fn cached_token_is_fresh_with_future_expiry() {
        let now = Utc::now();
        let entry = CachedOutboundToken {
            access_token: SecretString::new("access-token".to_owned().into_boxed_str()),
            refresh_token: None,
            expires_at: Some(now + ChronoDuration::seconds(120)),
        };
        assert!(entry.is_fresh(now, 30));
        assert!(!entry.is_fresh(now, 120));

        let no_expiry = CachedOutboundToken {
            access_token: SecretString::new("no-expiry".to_owned().into_boxed_str()),
            refresh_token: None,
            expires_at: None,
        };
        assert!(no_expiry.is_fresh(now, 0));
        assert!(no_expiry.is_fresh(now, 10_000));

        let boundary_entry = CachedOutboundToken {
            access_token: SecretString::new("boundary".to_owned().into_boxed_str()),
            refresh_token: None,
            expires_at: Some(now + ChronoDuration::seconds(30)),
        };
        assert!(boundary_entry.is_fresh(now, 29));
        assert!(!boundary_entry.is_fresh(now, 30));
        assert!(!boundary_entry.is_fresh(now, 31));
    }

    #[tokio::test]
    async fn token_cache_get_fresh_filters_stale_entries() {
        let cache = OutboundTokenCache::default();
        let key = cache_key();
        let now = Utc::now();
        cache
            .upsert(
                key.clone(),
                CachedOutboundToken {
                    access_token: SecretString::new("token".to_owned().into_boxed_str()),
                    refresh_token: None,
                    expires_at: Some(now + ChronoDuration::seconds(5)),
                },
            )
            .await;

        let fresh = cache
            .get_fresh(&key, now, 3)
            .await
            .expect("entry should be fresh with skew=3");
        assert_eq!(fresh.access_token.expose_secret(), "token");
        assert_eq!(fresh.expires_at, Some(now + ChronoDuration::seconds(5)));
        assert!(cache.get_fresh(&key, now, 5).await.is_none());
    }

    #[tokio::test]
    async fn refresh_lock_reuses_same_key_lock() {
        let cache = OutboundTokenCache::default();
        let key = cache_key();
        let lock_a = cache.refresh_lock(&key).await;
        let lock_b = cache.refresh_lock(&key).await;
        assert!(Arc::ptr_eq(&lock_a, &lock_b));

        let mut other_key = cache_key();
        other_key.upstream_name = "other-upstream".to_owned();
        let lock_c = cache.refresh_lock(&other_key).await;
        assert!(!Arc::ptr_eq(&lock_a, &lock_c));
    }
}
