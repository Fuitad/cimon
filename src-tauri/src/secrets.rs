//! Secure token storage.
//!
//! Access tokens live ONLY in the OS keychain (macOS Keychain, Windows Credential Manager),
//! never on disk, in logs, or in the frontend. [`SecretToken`] redacts itself in
//! `Debug`/`Display` and zeroizes its buffer on drop, so a token cannot leak through a log
//! line or error surface.

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::{Zeroize, Zeroizing};

const SERVICE: &str = "cimon";

/// A token wrapper that never reveals itself through `Debug`/`Display` and zeroizes on drop.
/// Call [`SecretToken::expose`] only at the point of use, and keep the borrow short-lived.
pub struct SecretToken(String);

impl SecretToken {
    pub fn new(token: impl Into<String>) -> Self {
        SecretToken(token.into())
    }

    /// Borrow the raw token. Keep the borrow as short-lived as possible.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretToken(***)")
    }
}

impl std::fmt::Display for SecretToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("***")
    }
}

impl Drop for SecretToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Error from the token store. The message is redacted and never contains token material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenStoreError(pub String);

impl std::fmt::Display for TokenStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "token store error: {}", self.0)
    }
}
impl std::error::Error for TokenStoreError {}

/// Abstraction over secure storage so tests can substitute an in-memory fake (no real
/// keychain access in CI).
pub trait TokenStore: Send + Sync {
    fn store(&self, account_id: &str, token: &str) -> Result<(), TokenStoreError>;
    fn get(&self, account_id: &str) -> Result<Option<SecretToken>, TokenStoreError>;
    fn delete(&self, account_id: &str) -> Result<(), TokenStoreError>;
}

/// OS keychain-backed store (macOS Keychain, Windows Credential Manager).
pub struct KeyringStore;

impl KeyringStore {
    pub fn new() -> Self {
        KeyringStore
    }

    fn entry(account_id: &str) -> Result<keyring::Entry, TokenStoreError> {
        keyring::Entry::new(SERVICE, account_id).map_err(|e| TokenStoreError(e.to_string()))
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for KeyringStore {
    fn store(&self, account_id: &str, token: &str) -> Result<(), TokenStoreError> {
        Self::entry(account_id)?
            .set_password(token)
            .map_err(|e| TokenStoreError(e.to_string()))
    }

    fn get(&self, account_id: &str) -> Result<Option<SecretToken>, TokenStoreError> {
        match Self::entry(account_id)?.get_password() {
            Ok(pw) => Ok(Some(SecretToken::new(pw))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(TokenStoreError(e.to_string())),
        }
    }

    fn delete(&self, account_id: &str) -> Result<(), TokenStoreError> {
        match Self::entry(account_id)?.delete_credential() {
            Ok(()) => Ok(()),
            // Idempotent: deleting a missing entry is success.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(TokenStoreError(e.to_string())),
        }
    }
}

/// A write-through, in-memory cache over another [`TokenStore`].
///
/// The wrapped store (the OS keychain) is consulted at most once per account per process run;
/// later reads are served from memory. Without this, the poller reads the keychain on every tick
/// and the settings window reads it again when listing projects, and on an unsigned dev build
/// (where "Always Allow" cannot be persisted to the item's ACL) every read re-prompts the user.
///
/// Caching does not widen the trust boundary: the token is already held in memory transiently to
/// make each API call. Cached values are zeroized when evicted (and on drop) via [`Zeroizing`].
pub struct CachingTokenStore {
    inner: Box<dyn TokenStore>,
    cache: Mutex<HashMap<String, Zeroizing<String>>>,
}

impl CachingTokenStore {
    pub fn new(inner: Box<dyn TokenStore>) -> Self {
        CachingTokenStore {
            inner,
            cache: Mutex::new(HashMap::new()),
        }
    }
}

impl TokenStore for CachingTokenStore {
    fn store(&self, account_id: &str, token: &str) -> Result<(), TokenStoreError> {
        self.inner.store(account_id, token)?;
        // Write through so a later read is served from memory (no keychain access, no prompt).
        self.cache
            .lock()
            .unwrap()
            .insert(account_id.to_string(), Zeroizing::new(token.to_string()));
        Ok(())
    }

    fn get(&self, account_id: &str) -> Result<Option<SecretToken>, TokenStoreError> {
        if let Some(cached) = self.cache.lock().unwrap().get(account_id) {
            return Ok(Some(SecretToken::new(cached.as_str())));
        }
        match self.inner.get(account_id)? {
            Some(tok) => {
                let raw = tok.expose().to_string();
                self.cache
                    .lock()
                    .unwrap()
                    .insert(account_id.to_string(), Zeroizing::new(raw.clone()));
                Ok(Some(SecretToken::new(raw)))
            }
            None => Ok(None),
        }
    }

    fn delete(&self, account_id: &str) -> Result<(), TokenStoreError> {
        // Invalidate first so a failed inner delete still drops the cached copy.
        self.cache.lock().unwrap().remove(account_id);
        self.inner.delete(account_id)
    }
}

/// In-memory token store for tests across modules (commands tests reuse it). Test-only.
#[cfg(test)]
pub(crate) struct MemoryTokenStore {
    inner: std::sync::Mutex<std::collections::HashMap<String, String>>,
    /// Number of `get` calls, so tests can assert when the keychain is (not) touched.
    reads: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl MemoryTokenStore {
    pub(crate) fn new() -> Self {
        MemoryTokenStore {
            inner: std::sync::Mutex::new(std::collections::HashMap::new()),
            reads: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(crate) fn count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub(crate) fn reads(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
impl TokenStore for MemoryTokenStore {
    fn store(&self, account_id: &str, token: &str) -> Result<(), TokenStoreError> {
        self.inner
            .lock()
            .unwrap()
            .insert(account_id.to_string(), token.to_string());
        Ok(())
    }
    fn get(&self, account_id: &str) -> Result<Option<SecretToken>, TokenStoreError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self
            .inner
            .lock()
            .unwrap()
            .get(account_id)
            .map(SecretToken::new))
    }
    fn delete(&self, account_id: &str) -> Result<(), TokenStoreError> {
        self.inner.lock().unwrap().remove(account_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_token_redacts_in_debug_and_display() {
        let t = SecretToken::new("supersecretvalue");
        assert_eq!(format!("{t}"), "***");
        let dbg = format!("{t:?}");
        assert!(
            !dbg.contains("supersecretvalue"),
            "Debug leaked the token: {dbg}"
        );
        assert!(dbg.contains("***"));
        // The raw value is still reachable at the point of use.
        assert_eq!(t.expose(), "supersecretvalue");
    }

    #[test]
    fn memory_store_roundtrip_and_idempotent_delete() {
        let store = MemoryTokenStore::new();
        assert!(store.get("acct-1").unwrap().is_none());
        store.store("acct-1", "tok").unwrap();
        assert_eq!(store.get("acct-1").unwrap().unwrap().expose(), "tok");
        store.delete("acct-1").unwrap();
        assert!(store.get("acct-1").unwrap().is_none());
        // Deleting an absent entry is success.
        store.delete("acct-1").unwrap();
    }

    /// A token store that counts how many times the (slow, prompt-causing) `get` is called.
    struct CountingInner {
        reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        map: Mutex<std::collections::HashMap<String, String>>,
    }

    impl TokenStore for CountingInner {
        fn store(&self, id: &str, token: &str) -> Result<(), TokenStoreError> {
            self.map.lock().unwrap().insert(id.into(), token.into());
            Ok(())
        }
        fn get(&self, id: &str) -> Result<Option<SecretToken>, TokenStoreError> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.map.lock().unwrap().get(id).map(SecretToken::new))
        }
        fn delete(&self, id: &str) -> Result<(), TokenStoreError> {
            self.map.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn counting() -> (
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        CountingInner,
    ) {
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let inner = CountingInner {
            reads: reads.clone(),
            map: Mutex::new(std::collections::HashMap::new()),
        };
        (reads, inner)
    }

    #[test]
    fn caching_store_reads_inner_once_then_serves_from_memory() {
        use std::sync::atomic::Ordering;
        let (reads, inner) = counting();
        inner.store("a", "tok").unwrap();
        let cache = CachingTokenStore::new(Box::new(inner));
        for _ in 0..3 {
            assert_eq!(cache.get("a").unwrap().unwrap().expose(), "tok");
        }
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "keychain (inner) read once, then served from memory"
        );
    }

    #[test]
    fn caching_store_store_writes_through_and_delete_invalidates() {
        use std::sync::atomic::Ordering;
        let (reads, inner) = counting();
        let cache = CachingTokenStore::new(Box::new(inner));
        // store writes through to the cache, so the first get needs no inner read (no prompt).
        cache.store("a", "tok").unwrap();
        assert_eq!(cache.get("a").unwrap().unwrap().expose(), "tok");
        assert_eq!(reads.load(Ordering::SeqCst), 0, "store populated the cache");
        // delete drops the cached copy, so a later get falls through to the inner store again.
        cache.delete("a").unwrap();
        assert!(cache.get("a").unwrap().is_none());
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "read inner after invalidation"
        );
    }
}
