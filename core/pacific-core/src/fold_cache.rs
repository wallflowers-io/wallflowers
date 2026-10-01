//! THE FOLD CACHE (O-69; core/docs/launch/mdr/icd-pin.md § O-69).
//!
//! A cached fold equals a fresh fold of the same object, under the same model, in its
//! state and in its failure. Everything here serves that sentence: the key moves whenever
//! anything the fold reads could have moved, and a key that cannot be trusted is no key.
//!
//! THE KEY is `(object, lens, open nonce, fold_gen, model id)`:
//! - `fold_gen`, one integer per object that SQLite triggers raise on every change to the
//!   rows the fold reads (the object's log, roster, owner history and owner), inside the
//!   writing transaction, so no write path can forget to invalidate;
//! - the open nonce, random at every open of the directory, so a restore that brings back an
//!   older `fold_gen` can never meet an entry made before it;
//! - the model id, sha256 of the pinned ICD and the core commit the binary was built from,
//!   named once by the embedding binary. A dirty or unstamped build names none and folds
//!   uncached (FC-8).
//!
//! IT HOLDS FOLDS, NOT ANSWERS: nothing is scoped here, and every caller filters after it
//! exactly as after a fresh fold. It lives in memory, per directory, bounded in bytes, and
//! evicts the least recently used; an eviction is only a miss. No folded state is written
//! anywhere.
//!
//! ITS BYTES ARE COUNTED, NOT ESTIMATED (FC-10): a fold retains about three times its rows,
//! so an estimate bounds nothing. The embedding names its allocator's count (`set_measure`);
//! with none, nothing is kept.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// The pinned ICD's hash, as committed beside the ICD (O-68): half of the model id.
const PIN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.sha256"));

/// What a directory holds when nothing sets a bound: 32 MiB.
pub const DEFAULT_BYTES: usize = 32 << 20;

static MODEL: OnceLock<Option<[u8; 32]>> = OnceLock::new();
static BYTES: OnceLock<usize> = OnceLock::new();
static MEASURE: OnceLock<fn() -> isize> = OnceLock::new();

/// Name this thread's bytes allocated net of those freed, from a counting global allocator,
/// once, at start. Kept only if it counts: an allocation made here must move it.
pub fn set_measure(allocated: fn() -> isize) {
    let before = allocated();
    let probe = std::hint::black_box(vec![0u8; 256]);
    let counts = allocated() - before >= 256;
    drop(probe);
    if counts {
        let _ = MEASURE.set(allocated);
    }
}

/// Name the core commit this binary was built from, once, at start. `None` (a dirty or an
/// unstamped build) leaves the cache off for the life of the process.
pub fn set_model(core_commit: Option<&str>) {
    let _ = MODEL.set(core_commit.filter(|c| !c.is_empty()).map(|c| model_of(PIN, c)));
}

/// The model id for a pin file's contents and a commit: the pin's hash, then the commit.
pub fn model_of(pin_file: &str, core_commit: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"pacific-fold-model:v1");
    h.update(pin_file.split_whitespace().next().unwrap_or_default().as_bytes());
    h.update([0]);
    h.update(core_commit.as_bytes());
    h.finalize().into()
}

/// This process's model id, if it named one. A test build has no stamped commit, so a
/// test names a model in `PACIFIC_FOLD_CACHE_MODEL` instead, and only when nothing was set
/// at start; a deploy never sets it, and a deployed binary names its own commit.
pub fn model() -> Option<[u8; 32]> {
    MEASURE.get()?;
    *MODEL.get_or_init(|| {
        std::env::var("PACIFIC_FOLD_CACHE_MODEL").ok().filter(|m| !m.is_empty()).map(|m| model_of(PIN, &format!("test:{m}")))
    })
}

/// Bound every directory's cache, once, at start (the session's memory budget).
pub fn set_bytes(bytes: usize) {
    let _ = BYTES.set(bytes);
}

fn bytes_limit() -> usize {
    *BYTES.get_or_init(|| DEFAULT_BYTES)
}

/// FC-2: with `PACIFIC_FOLD_CACHE_VERIFY=1`, every hit is refolded and compared, and a
/// difference panics naming the object and the key.
pub fn verify() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| std::env::var("PACIFIC_FOLD_CACHE_VERIFY").is_ok_and(|v| v == "1"))
}

/// One fold's key. `lens` is the kind the fold runs under; `what` separates the products
/// of one fold (the coordinator, a view) so they never meet.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub object: Vec<u8>,
    pub lens: String,
    pub what: &'static str,
    pub nonce: u64,
    pub gen: u64,
    pub model: [u8; 32],
}

struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    bytes: usize,
    used: u64,
}

/// The entries of one directory.
#[derive(Default)]
pub struct FoldCache {
    entries: HashMap<Key, Entry>,
    bytes: usize,
    clock: u64,
    /// Hits and misses since this directory opened: what the cache is doing, for tests.
    pub hits: u64,
    pub misses: u64,
    /// A bound of its own, for a test; otherwise the process's (`set_bytes`).
    limit: Option<usize>,
}

impl FoldCache {
    /// A cache with a bound of its own (a test's).
    pub fn with_limit(bytes: usize) -> Self {
        Self { limit: Some(bytes), ..Default::default() }
    }

    /// The value under `key`, if it is there and of the type asked for.
    pub fn get<V: Clone + 'static>(&mut self, key: &Key) -> Option<V> {
        self.clock += 1;
        let clock = self.clock;
        let got = self.entries.get_mut(key).and_then(|e| {
            e.used = clock;
            e.value.downcast_ref::<V>().cloned()
        });
        if got.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        got
    }

    /// Keep a copy of `value` under `key`, at what the copy retains as the allocator counts
    /// it. Nothing is kept without a measure.
    pub fn put<V: Clone + Send + Sync + 'static>(&mut self, key: Key, value: &V) {
        let Some(allocated) = MEASURE.get() else { return };
        self.drop_stale(&key);
        let before = allocated();
        let copy: Arc<dyn Any + Send + Sync> = Arc::new(value.clone());
        let held = (allocated() - before).max(0) as usize;
        let bytes = held + key.object.len() + key.lens.len();
        self.insert(key, copy, bytes);
    }

    /// Any older entry of the same object, lens and product goes: only the newest fold of
    /// an object is worth keeping.
    fn drop_stale(&mut self, key: &Key) {
        let stale: Vec<Key> = self
            .entries
            .keys()
            .filter(|k| k.object == key.object && k.lens == key.lens && k.what == key.what && *k != key)
            .cloned()
            .collect();
        for k in stale {
            self.remove(&k);
        }
    }

    /// Keep `value` at `bytes`, evicting the least recently used to fit. A value larger than
    /// the whole bound is not kept at all.
    fn insert(&mut self, key: Key, value: Arc<dyn Any + Send + Sync>, bytes: usize) {
        let limit = self.limit.unwrap_or_else(bytes_limit);
        // The table grows now, if it will, so what it costs is known before anything goes.
        self.entries.reserve(1);
        let table = Self::table_for(self.entries.capacity());
        if bytes + table > limit {
            return;
        }
        while self.bytes + bytes + table > limit {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone()) else { return };
            self.remove(&oldest);
        }
        self.clock += 1;
        if let Some(old) = self.entries.insert(key, Entry { value, bytes, used: self.clock }) {
            self.bytes -= old.bytes;
        }
        self.bytes += bytes;
    }

    fn remove(&mut self, key: &Key) {
        if let Some(e) = self.entries.remove(key) {
            self.bytes -= e.bytes;
        }
    }

    /// What the cache holds, in its own accounting: its entries as counted, and its table.
    pub fn bytes(&self) -> usize {
        self.bytes + Self::table_for(self.entries.capacity())
    }

    /// The most a table of `n` slots allocates: at most 2n + 1 buckets of a slot and a
    /// control byte each, and a group's width.
    fn table_for(n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (2 * n + 1) * (std::mem::size_of::<(Key, Entry)>() + 1) + 16
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(object: u8, gen: u64) -> Key {
        Key { object: vec![object], lens: "forum".into(), what: "fold", nonce: 7, gen, model: [1; 32] }
    }

    /// The LRU's mechanics at given sizes. What an entry measures is FC-10's, through a
    /// Node and a counting allocator: `tests/o69_fold_cache_bound.rs`.
    impl FoldCache {
        fn put_sized<V: Send + Sync + 'static>(&mut self, key: Key, value: V, bytes: usize) {
            self.drop_stale(&key);
            self.insert(key, Arc::new(value), bytes);
        }
    }

    /// A newer fold of the same object replaces the older; another object's is kept.
    #[test]
    fn only_the_newest_fold_of_an_object_is_kept() {
        let mut c = FoldCache::default();
        c.put_sized(key(1, 1), 10u32, 100);
        c.put_sized(key(2, 1), 20u32, 100);
        c.put_sized(key(1, 2), 11u32, 100);
        assert_eq!(c.get::<u32>(&key(1, 1)), None, "the older generation is gone");
        assert_eq!(c.get::<u32>(&key(1, 2)), Some(11));
        assert_eq!(c.get::<u32>(&key(2, 1)), Some(20));
        assert_eq!((c.len(), c.bytes()), (2, 200 + FoldCache::table_for(c.entries.capacity())), "the entries, and the table");
    }

    /// At its bound the cache evicts the least recently used and stays within the bound; a
    /// value larger than the whole bound is not kept.
    #[test]
    fn at_its_bound_the_least_recently_used_goes() {
        // Room for three entries and the table four need.
        const E: usize = 10_000;
        let limit = 3 * E + FoldCache::table_for(7);
        let mut c = FoldCache::with_limit(limit);
        c.put_sized(key(1, 1), 1u32, E);
        c.put_sized(key(2, 1), 2u32, E);
        c.put_sized(key(3, 1), 3u32, E);
        assert_eq!(c.len(), 3);
        assert_eq!(c.get::<u32>(&key(1, 1)), Some(1), "1 is used, so 2 is now the oldest");
        c.put_sized(key(4, 1), 4u32, E);
        assert_eq!(c.get::<u32>(&key(2, 1)), None, "the least recently used went");
        assert_eq!((c.get::<u32>(&key(1, 1)), c.get::<u32>(&key(3, 1)), c.get::<u32>(&key(4, 1))), (Some(1), Some(3), Some(4)));
        assert!(c.bytes() <= limit, "within its bound: {} > {limit}", c.bytes());
        c.put_sized(key(5, 1), 5u32, limit + 1);
        assert_eq!(c.get::<u32>(&key(5, 1)), None, "larger than the bound is not kept");
        assert_eq!(c.len(), 3, "and evicts nothing");
        assert!(c.bytes() <= limit);
    }

    /// With no measure named, nothing is kept (FC-10: an entry's size is never guessed).
    #[test]
    fn without_a_measure_nothing_is_kept() {
        let mut c = FoldCache::default();
        c.put(key(1, 1), &10u32);
        assert_eq!((c.len(), c.bytes()), (0, 0));
    }

    /// A value of another type under the same key is no hit.
    #[test]
    fn a_hit_is_of_the_type_asked_for() {
        let mut c = FoldCache::default();
        c.put_sized(key(1, 1), 10u32, 1);
        assert_eq!(c.get::<u64>(&key(1, 1)), None);
    }

    /// The model id moves with the pin and with the commit.
    #[test]
    fn the_model_moves_with_the_pin_and_the_commit() {
        let a = model_of("aa  delta-graph.icd.json\n", "c1");
        assert_eq!(a, model_of("aa  other-name\n", "c1"), "only the hash in the pin file counts");
        assert_ne!(a, model_of("bb  delta-graph.icd.json\n", "c1"), "a new pin");
        assert_ne!(a, model_of("aa  delta-graph.icd.json\n", "c2"), "a new commit");
    }
}
