//! Persistent plugin.json manifest cache (TPIN-26).
//!
//! Manifests are small, change rarely and cost Core API quota on every discovery, so
//! they persist in `appDataDir/plugins-cache.json` through tauri-plugin-store instead
//! of living only in the in-memory [`super::ResponseCache`]. Two TTLs apply:
//!
//! - fresh (soft) TTL: entries younger than this are served with zero network traffic
//!   and never pass through quota admission — a cache hit must not be blocked by a
//!   rate gate that guards requests we never send;
//! - hard TTL: stale entries may still be revalidated with If-None-Match / ETag until
//!   the hard expiry, at which point they are deleted *before* the miss is answered.
//!
//! Entries are namespaced by an identity fingerprint (`anon` or a truncated
//! lowercase SHA-256 of the PAT — never the token itself), so token rotation keeps
//! per-identity buckets intact across switches without ever persisting credentials.

use base64::Engine as _;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Store file under `appDataDir`, dedicated to manifest cache entries.
pub const MANIFEST_STORE_FILE: &str = "plugins-cache.json";
/// Key namespace; a version bump invalidates every previously persisted entry.
pub const MANIFEST_KEY_PREFIX: &str = "manifest:v1:";
/// Entries this fresh are served without any network request.
pub const FRESH_TTL_MS: i64 = 60 * 60 * 1000;
/// Entries older than this are deleted instead of revalidated.
pub const HARD_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// plugin.json is a manifest, not an archive: a generous but strict body bound.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
/// Bound on persisted entries; oldest-validated entries are evicted past it.
pub const MAX_MANIFEST_ENTRIES: usize = 256;

/// Current wall clock in milliseconds — the same clock source the client uses.
pub fn system_now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Stable per-identity namespace for manifest buckets. `pat:` carries only the
/// first 16 hex chars of the lowercase SHA-256 digest — enough to separate real
/// tokens, never enough to recover one. The raw token is never persisted or logged.
pub fn identity_fingerprint(token: Option<&str>) -> String {
    match token {
        Some(token) => {
            let digest = Sha256::digest(token.as_bytes());
            let mut hex = String::with_capacity(16);
            for byte in &digest[..8] {
                hex.push_str(&format!("{byte:02x}"));
            }
            format!("pat:{hex}")
        }
        None => "anon".to_owned(),
    }
}

/// Escape a key segment so `:`, `/`, `%` and other separators cannot merge two
/// distinct (owner, repo, branch) triples into one key. Only [A-Za-z0-9._-] pass
/// through; everything else becomes uppercase %XX.
fn escape_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `manifest:v1:<fingerprint>:<owner>:<repo>:<escaped-branch>`
pub fn manifest_key(fingerprint: &str, owner: &str, repo: &str, branch: &str) -> String {
    format!(
        "{MANIFEST_KEY_PREFIX}{fingerprint}:{}:{}:{}",
        escape_segment(owner),
        escape_segment(repo),
        escape_segment(branch)
    )
}

mod body_base64 {
    use super::*;
    use serde::{Deserializer, Serializer};
    pub fn serialize<S: Serializer>(body: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(body))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() > MAX_MANIFEST_BYTES.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom(
                "manifest cache body exceeds size limit",
            ));
        }
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

/// One persisted manifest bucket. Unknown fields are tolerated on read so a newer
/// build's entries remain loadable; missing required fields count as corruption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    #[serde(with = "body_base64")]
    pub body: Vec<u8>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// Before this instant the entry is served without any network request.
    pub fresh_until: i64,
    /// After this instant the entry is deleted instead of used or revalidated.
    pub expires_at: i64,
    /// Last successful validation time, used for oldest-first bounded eviction.
    pub last_used: i64,
}

impl ManifestEntry {
    /// Reject structurally parseable but corrupt state before it can bypass TTLs
    /// or put invalid conditional headers on an otherwise healthy API request.
    fn is_usable(&self) -> bool {
        self.body.len() <= MAX_MANIFEST_BYTES
            && self.last_used <= self.fresh_until
            && self.fresh_until <= self.expires_at
            && self.fresh_until.saturating_sub(self.last_used) <= FRESH_TTL_MS
            && self.expires_at.saturating_sub(self.last_used) <= HARD_TTL_MS
            && [self.etag.as_deref(), self.last_modified.as_deref()]
                .into_iter()
                .flatten()
                .all(|value| {
                    value.len() <= 8192 && reqwest::header::HeaderValue::from_str(value).is_ok()
                })
    }

    /// Build an entry from a fresh 200 response.
    pub fn new(body: &[u8], headers: &HeaderMap, now: i64) -> Self {
        Self {
            body: body.to_vec(),
            etag: super::header(headers, "etag").map(str::to_owned),
            last_modified: super::header(headers, "last-modified").map(str::to_owned),
            fresh_until: now.saturating_add(FRESH_TTL_MS),
            expires_at: now.saturating_add(HARD_TTL_MS),
            last_used: now,
        }
    }

    /// A 304 renews BOTH TTLs. Validators omitted by the server are retained —
    /// GitHub only echoes validators that changed.
    pub fn renewed(&self, headers: &HeaderMap, now: i64) -> Self {
        Self {
            body: self.body.clone(),
            etag: super::header(headers, "etag")
                .map(str::to_owned)
                .or_else(|| self.etag.clone()),
            last_modified: super::header(headers, "last-modified")
                .map(str::to_owned)
                .or_else(|| self.last_modified.clone()),
            fresh_until: now.saturating_add(FRESH_TTL_MS),
            expires_at: now.saturating_add(HARD_TTL_MS),
            last_used: now,
        }
    }
}

pub enum ManifestHit {
    /// Serve without network and without quota admission.
    Fresh(Vec<u8>),
    /// Revalidate over the network using these validators (possibly none).
    Stale(ManifestEntry),
}

/// Storage seam over tauri-plugin-store (production) and in-memory maps (tests).
/// All methods are infallible: cache corruption and I/O failure degrade to misses
/// and no-ops, logging a generic warning — they must never fail app startup or a
/// GitHub call.
pub trait ManifestStore: Send + Sync {
    fn get(&self, key: &str) -> Option<ManifestEntry>;
    fn set(&self, key: &str, entry: &ManifestEntry);
    fn delete(&self, key: &str);
    fn entries(&self) -> Vec<(String, ManifestEntry)>;
}

/// Read one bucket, applying hard-expiry deletion *before* reporting a miss.
pub fn manifest_lookup(store: &dyn ManifestStore, key: &str, now: i64) -> Option<ManifestHit> {
    let entry = store.get(key)?;
    if !entry.is_usable() || entry.last_used > now {
        log::warn!("GitHub manifest cache: dropping invalid entry");
        store.delete(key);
        return None;
    }
    if entry.expires_at <= now {
        store.delete(key);
        return None;
    }
    if entry.fresh_until > now {
        Some(ManifestHit::Fresh(entry.body))
    } else {
        Some(ManifestHit::Stale(entry))
    }
}

/// Write an entry enforcing the entry/body bounds. Oversized bodies are dropped;
/// past the entry bound the oldest-validated buckets are evicted.
pub fn manifest_put(store: &dyn ManifestStore, key: &str, entry: &ManifestEntry) {
    if !entry.is_usable() {
        // A newer oversized/invalid response must not leave an old body reusable.
        store.delete(key);
        return;
    }
    let all = store.entries();
    if all.len() >= MAX_MANIFEST_ENTRIES && !all.iter().any(|(k, _)| k == key) {
        let mut victims: Vec<(String, i64)> =
            all.iter().map(|(k, e)| (k.clone(), e.last_used)).collect();
        victims.sort_by_key(|(_, used)| *used);
        for (victim, _) in victims
            .into_iter()
            .take(all.len() - MAX_MANIFEST_ENTRIES + 1)
        {
            store.delete(&victim);
        }
    }
    store.set(key, entry);
}

/// Startup prune covers every identity bucket, not just the current session.
/// Backends remove unreadable values during `entries()`; malformed TTLs and headers
/// are discarded here. A backwards clock jump invalidates future-dated entries.
pub fn prune_expired(store: &dyn ManifestStore, now: i64) {
    let mut live = Vec::new();
    for (key, entry) in store.entries() {
        if !entry.is_usable() || entry.last_used > now {
            log::warn!("GitHub manifest cache: dropping invalid entry");
            store.delete(&key);
        } else if entry.expires_at <= now {
            store.delete(&key);
        } else {
            live.push((key, entry.last_used));
        }
    }
    live.sort_by_key(|(_, used)| *used);
    let excess = live.len().saturating_sub(MAX_MANIFEST_ENTRIES);
    for (key, _) in live.into_iter().take(excess) {
        store.delete(&key);
    }
}

/// tauri-plugin-store backed persistence. Every operation degrades gracefully:
/// a corrupt value is deleted and reported as a miss, a failed save only warns.
pub struct TauriStoreBackend {
    store: std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>,
}

impl TauriStoreBackend {
    /// Open (creating if needed) `plugins-cache.json` under appDataDir and prune
    /// every hard-expired bucket. Callers pass `now_ms` so the prune uses the same
    /// clock source as the session; on `Err` the caller falls back to a client
    /// without a manifest store.
    pub fn open(app: &tauri::AppHandle, now_ms: i64) -> Result<Self, tauri_plugin_store::Error> {
        let store = tauri_plugin_store::StoreBuilder::new(app, MANIFEST_STORE_FILE)
            .disable_auto_save()
            .build()?;
        // StoreBuilder deliberately ignores load errors in tauri-plugin-store 2.
        // An explicit reload exposes corruption/I/O failure for a generic warning
        // without making a disposable cache capable of failing app startup.
        if let Err(error) = store.reload() {
            if !matches!(error, tauri_plugin_store::Error::Io(ref io)
                if io.kind() == std::io::ErrorKind::NotFound)
            {
                log::warn!("GitHub manifest cache: failed to load store");
            }
        }
        let backend = Self { store };
        prune_expired(&backend, now_ms);
        backend.flush();
        Ok(backend)
    }

    fn flush(&self) {
        if self.store.save().is_err() {
            // Never include raw backend errors, keys or values in cache warnings.
            log::warn!("GitHub manifest cache: failed to persist store");
        }
    }
}

impl ManifestStore for TauriStoreBackend {
    fn get(&self, key: &str) -> Option<ManifestEntry> {
        let value = self.store.get(key)?;
        match serde_json::from_value::<ManifestEntry>(value) {
            Ok(entry) => Some(entry),
            Err(_) => {
                log::warn!("GitHub manifest cache: dropping unreadable entry");
                self.store.delete(key);
                self.flush();
                None
            }
        }
    }

    fn set(&self, key: &str, entry: &ManifestEntry) {
        match serde_json::to_value(entry) {
            Ok(value) => {
                self.store.set(key.to_owned(), value);
                self.flush();
            }
            Err(_) => log::warn!("GitHub manifest cache: failed to encode entry"),
        }
    }

    fn delete(&self, key: &str) {
        if self.store.delete(key) {
            self.flush();
        }
    }

    fn entries(&self) -> Vec<(String, ManifestEntry)> {
        // Going through get() also removes malformed values at startup instead of
        // hiding them from pruning and leaving them on disk forever.
        self.store
            .keys()
            .into_iter()
            .filter(|key| key.starts_with(MANIFEST_KEY_PREFIX))
            .filter_map(|key| self.get(&key).map(|entry| (key, entry)))
            .collect()
    }
}

/// Test backend: stores raw JSON values so corrupt entries can be injected, and
/// can simulate backend unavailability. Shared `Arc`s across two backends model a
/// restart against the same persisted file.
#[cfg(test)]
pub mod memory {
    use super::*;
    use serde_json::Value as JsonValue;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryBackend {
        pub values: Mutex<BTreeMap<String, JsonValue>>,
        pub unavailable: AtomicBool,
    }

    impl MemoryBackend {
        fn alive(&self) -> bool {
            !self.unavailable.load(Ordering::SeqCst)
        }
    }

    impl ManifestStore for MemoryBackend {
        fn get(&self, key: &str) -> Option<ManifestEntry> {
            if !self.alive() {
                return None;
            }
            let value = self.values.lock().unwrap().get(key)?.clone();
            match serde_json::from_value::<ManifestEntry>(value) {
                Ok(entry) => Some(entry),
                Err(_) => {
                    // Mirrors TauriStoreBackend: unreadable entry is dropped.
                    self.values.lock().unwrap().remove(key);
                    None
                }
            }
        }
        fn set(&self, key: &str, entry: &ManifestEntry) {
            if !self.alive() {
                return;
            }
            if let Ok(value) = serde_json::to_value(entry) {
                self.values.lock().unwrap().insert(key.to_owned(), value);
            }
        }
        fn delete(&self, key: &str) {
            if !self.alive() {
                return;
            }
            self.values.lock().unwrap().remove(key);
        }
        fn entries(&self) -> Vec<(String, ManifestEntry)> {
            if !self.alive() {
                return Vec::new();
            }
            let keys: Vec<_> = self.values.lock().unwrap().keys().cloned().collect();
            keys.into_iter()
                .filter(|key| key.starts_with(MANIFEST_KEY_PREFIX))
                .filter_map(|key| self.get(&key).map(|entry| (key, entry)))
                .collect()
        }
    }
}
