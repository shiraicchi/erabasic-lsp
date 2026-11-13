//! Fast hash map implementation using fxhash.

pub use fxhash::{FxHashMap as HashMap, FxHashSet as HashSet, FxHasher as Hasher};

/// Type alias for HashMap with FxHasher
pub type FxHashMap<K, V> = fxhash::FxHashMap<K, V>;

/// Type alias for HashSet with FxHasher
pub type FxHashSet<T> = fxhash::FxHashSet<T>;
