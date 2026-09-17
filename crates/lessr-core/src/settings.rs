//! Per-stage tunables: an ordered map of scalars, read with a default.
//!
//! A stage names what it needs where it needs it — `settings.u64("max_lines",
//! 200)` — so there is no table of compiled-in defaults to drift out of step
//! with the code that reads them. Two properties matter more than the map:
//!
//! - A value that is missing, malformed or of the wrong type returns the
//!   default and is recorded. A bad line in `config.toml` must never panic or
//!   fail the pipeline: that would break the agent Lessr is there to make
//!   cheaper.
//! - A key nothing asked for is recorded too, so `lessr config` can show a
//!   typo instead of swallowing it.
//!
//! Both records live in atomics rather than behind a lock. A `Settings` is
//! shared by whatever threads the proxy runs on, and a diagnostic must never
//! become a synchronisation point on the path it is diagnosing. The ordering
//! is `Relaxed` throughout because nothing is published through these flags;
//! they are counters a human reads later.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// The kinds of scalar a setting can hold.
///
/// Scalars only: a tunable that needs structure is a mechanism asking for its
/// own file, not a config key.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ValueKind {
    /// `true` or `false`.
    Bool,
    /// A whole number.
    Int,
    /// Text.
    Str,
}

impl ValueKind {
    /// The name used when reporting a value a stage could not use.
    pub const fn as_str(self) -> &'static str {
        match self {
            ValueKind::Bool => "boolean",
            ValueKind::Int => "integer",
            ValueKind::Str => "string",
        }
    }

    /// The byte that stands for this kind in the snapshot and in the
    /// rejection flag. Stable across releases: it is on disk.
    pub(crate) const fn tag(self) -> u8 {
        match self {
            ValueKind::Bool => 1,
            ValueKind::Int => 2,
            ValueKind::Str => 3,
        }
    }

    /// The kind a byte stands for, or `None` for a byte written by a version
    /// we do not know.
    pub(crate) const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(ValueKind::Bool),
            2 => Some(ValueKind::Int),
            3 => Some(ValueKind::Str),
            _ => None,
        }
    }
}

/// One scalar a stage can be tuned with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// `true` or `false`.
    Bool(bool),
    /// A whole number. Signed because a config file can hold one; the `u64`
    /// accessor treats a negative as malformed rather than wrapping it.
    Int(i64),
    /// Text. `Box<str>` because a setting is written once and read many times.
    Str(Box<str>),
}

impl Value {
    /// Which kind this is.
    pub const fn kind(&self) -> ValueKind {
        match self {
            Value::Bool(_) => ValueKind::Bool,
            Value::Int(_) => ValueKind::Int,
            Value::Str(_) => ValueKind::Str,
        }
    }

    /// This value as a `u64`, if it can be one.
    pub fn as_u64(&self) -> Option<u64> {
        self.scalar().as_u64()
    }

    /// This value as a `bool`, if it can be one.
    pub fn as_bool(&self) -> Option<bool> {
        self.scalar().as_bool()
    }

    /// This value as text, if it is text.
    pub fn as_str(&self) -> Option<&str> {
        self.scalar().as_str()
    }

    /// This value without the box, so the coercion rules have one home.
    pub(crate) fn scalar(&self) -> Scalar<'_> {
        match self {
            Value::Bool(b) => Scalar::Bool(*b),
            Value::Int(n) => Scalar::Int(*n),
            Value::Str(s) => Scalar::Str(s),
        }
    }
}

/// A scalar that borrows instead of owning.
///
/// The snapshot holds its values as bytes in a mapped file and the map holds
/// them as a [`Value`]; both convert through this, so what counts as a usable
/// number is decided in exactly one place and the hook path builds no
/// [`Value`] to ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scalar<'a> {
    Bool(bool),
    Int(i64),
    Str(&'a str),
}

impl<'a> Scalar<'a> {
    /// As a `u64`, if it can be one.
    ///
    /// Text that is plainly a number is accepted: a user who wrote
    /// `max_lines = "200"` meant 200, and refusing it would only make Lessr
    /// look broken. A negative number cannot be one, and neither can a bool.
    pub(crate) fn as_u64(self) -> Option<u64> {
        match self {
            Scalar::Int(n) => u64::try_from(n).ok(),
            Scalar::Str(s) => s.trim().parse::<u64>().ok(),
            Scalar::Bool(_) => None,
        }
    }

    /// As a `bool`, if it can be one: the spellings a config file realistically
    /// holds, and `0`/`1`, which is what an environment variable carries.
    pub(crate) fn as_bool(self) -> Option<bool> {
        match self {
            Scalar::Bool(b) => Some(b),
            Scalar::Int(0) => Some(false),
            Scalar::Int(1) => Some(true),
            Scalar::Int(_) => None,
            Scalar::Str(s) => match s.trim() {
                s if s.eq_ignore_ascii_case("true") => Some(true),
                s if s.eq_ignore_ascii_case("yes") => Some(true),
                s if s.eq_ignore_ascii_case("on") => Some(true),
                "1" => Some(true),
                s if s.eq_ignore_ascii_case("false") => Some(false),
                s if s.eq_ignore_ascii_case("no") => Some(false),
                s if s.eq_ignore_ascii_case("off") => Some(false),
                "0" => Some(false),
                _ => None,
            },
        }
    }

    /// As text, if it is text.
    ///
    /// A number is not rendered on the fly: the accessor hands back a borrow,
    /// and there is nothing to borrow from for a value never stored as text.
    pub(crate) fn as_str(self) -> Option<&'a str> {
        match self {
            Scalar::Str(s) => Some(s),
            Scalar::Bool(_) | Scalar::Int(_) => None,
        }
    }

    /// The owned form, for a caller that is keeping it.
    pub(crate) fn to_value(self) -> Value {
        match self {
            Scalar::Bool(b) => Value::Bool(b),
            Scalar::Int(n) => Value::Int(n),
            Scalar::Str(s) => Value::Str(s.into()),
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Int(n)
    }
}

impl From<u64> for Value {
    /// Saturates at [`i64::MAX`]. Nothing Lessr tunes — line counts, byte
    /// counts, milliseconds — comes within nine exabytes of the clamp, and a
    /// silent wrap would be worse than a clamp that cannot be reached.
    fn from(n: u64) -> Self {
        Value::Int(i64::try_from(n).unwrap_or(i64::MAX))
    }
}

impl From<usize> for Value {
    fn from(n: usize) -> Self {
        Value::from(n as u64)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.into())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s.into_boxed_str())
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s}"),
        }
    }
}

/// A value a stage asked for and could not use.
///
/// `lessr config` prints these beside [`Settings::unread`]: between them they
/// cover both halves of "my setting does nothing" — the key was never read, or
/// it was read and thrown away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rejected<'a> {
    /// The key, spelled as the config file spelled it.
    pub key: &'a str,
    /// The value that could not be used.
    pub value: &'a Value,
    /// The kind the stage asked for.
    pub wanted: ValueKind,
}

/// One key as the map holds it.
struct Entry {
    key: Box<str>,
    value: Value,
    /// Set the first time any accessor names this key, whether or not the
    /// value was usable. A key read and rejected is not an unknown key.
    read: AtomicBool,
    /// The [`ValueKind::tag`] an accessor wanted and did not get, or 0.
    rejected: AtomicU8,
}

impl Entry {
    fn reject(&self, wanted: ValueKind) {
        self.rejected.store(wanted.tag(), Ordering::Relaxed);
    }
}

impl Clone for Entry {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            value: self.value.clone(),
            read: AtomicBool::new(self.read.load(Ordering::Relaxed)),
            rejected: AtomicU8::new(self.rejected.load(Ordering::Relaxed)),
        }
    }
}

/// The tunables of one stage, in the order they were set.
///
/// Ordered because the order is what a human wrote and what the snapshot
/// encodes; a map that reshuffles itself makes two identical configs produce
/// two different files.
#[derive(Clone, Default)]
pub struct Settings {
    entries: Vec<Entry>,
}

impl Settings {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a key, replacing any value already there.
    ///
    /// Replacing keeps the key's position and clears what has been noticed
    /// about it: the old value's read and rejection are not the new one's.
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        let value = value.into();
        if let Some(entry) = self.entries.iter_mut().find(|e| &*e.key == key) {
            entry.value = value;
            entry.read = AtomicBool::new(false);
            entry.rejected = AtomicU8::new(0);
            return;
        }
        self.entries.push(Entry {
            key: key.into(),
            value,
            read: AtomicBool::new(false),
            rejected: AtomicU8::new(0),
        });
    }

    /// The value under a key, without marking it read.
    ///
    /// For tools that print a config. A stage uses the typed accessors, which
    /// is what makes [`Settings::unread`] mean anything.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entry(key).map(|e| &e.value)
    }

    /// A whole number, or `default` if the key is absent or the value is not
    /// one.
    pub fn u64(&self, key: &str, default: u64) -> u64 {
        self.read(key, ValueKind::Int, Value::as_u64)
            .unwrap_or(default)
    }

    /// A flag, or `default` if the key is absent or the value is not one.
    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.read(key, ValueKind::Bool, Value::as_bool)
            .unwrap_or(default)
    }

    /// Text, or `default` if the key is absent or the value is not text.
    ///
    /// The borrow is into the map, so reading a string setting on the hook
    /// path allocates nothing.
    pub fn str<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        match self.entry(key) {
            None => default,
            Some(entry) => {
                entry.read.store(true, Ordering::Relaxed);
                match entry.value.as_str() {
                    Some(s) => s,
                    None => {
                        entry.reject(ValueKind::Str);
                        default
                    }
                }
            }
        }
    }

    /// Keys nothing asked for.
    ///
    /// `lessr config` reports these so a typo in a config file is visible
    /// instead of silently ignored.
    pub fn unread(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| !e.read.load(Ordering::Relaxed))
            .map(|e| &*e.key)
            .collect()
    }

    /// Values a stage asked for and could not use.
    pub fn rejected(&self) -> Vec<Rejected<'_>> {
        self.entries
            .iter()
            .filter_map(|e| {
                let wanted = ValueKind::from_tag(e.rejected.load(Ordering::Relaxed))?;
                Some(Rejected {
                    key: &e.key,
                    value: &e.value,
                    wanted,
                })
            })
            .collect()
    }

    /// The keys, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| &*e.key)
    }

    /// The keys and values, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|e| (&*e.key, &e.value))
    }

    /// Take every key from `other`, overwriting what is here.
    ///
    /// This is how one configuration layer lands on the one below it: per key,
    /// not per stage, so setting one tunable in a repo does not drop the rest.
    pub fn merge(&mut self, other: &Settings) {
        for (key, value) in other.iter() {
            self.set(key, value.clone());
        }
    }

    /// How many keys are set.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is set.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| &*e.key == key)
    }

    /// Mark a key read and convert it, recording the kind that was wanted when
    /// the value cannot be one.
    fn read<T>(&self, key: &str, wanted: ValueKind, as_kind: fn(&Value) -> Option<T>) -> Option<T> {
        let entry = self.entry(key)?;
        entry.read.store(true, Ordering::Relaxed);
        match as_kind(&entry.value) {
            Some(value) => Some(value),
            None => {
                entry.reject(wanted);
                None
            }
        }
    }
}

impl PartialEq for Settings {
    /// Equal when the same keys hold the same values in the same order. What
    /// has been read or rejected is a diagnostic, not part of the value.
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other.iter())
                .all(|((ak, av), (bk, bv))| ak == bk && av == bv)
    }
}

impl Eq for Settings {}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let mut s = Settings::new();
        s.set("max_lines", 200u64);
        s.set("collapse", true);
        s.set("style", "unified");
        s
    }

    #[test]
    fn typed_accessors_read_what_was_set() {
        let s = settings();
        assert_eq!(s.u64("max_lines", 50), 200);
        assert!(s.bool("collapse", false));
        assert_eq!(s.str("style", "context"), "unified");
    }

    #[test]
    fn a_missing_key_gives_the_default() {
        let s = Settings::new();
        assert_eq!(s.u64("max_lines", 50), 50);
        assert!(!s.bool("collapse", false));
        assert_eq!(s.str("style", "context"), "context");
        assert!(s.rejected().is_empty(), "absence is not a rejection");
    }

    #[test]
    fn a_wrong_typed_value_falls_back_and_is_reported() {
        let mut s = Settings::new();
        s.set("max_lines", true);

        assert_eq!(s.u64("max_lines", 50), 50, "the default, not a panic");

        let rejected = s.rejected();
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].key, "max_lines");
        assert_eq!(rejected[0].wanted, ValueKind::Int);
        assert_eq!(rejected[0].value, &Value::Bool(true));
    }

    #[test]
    fn a_malformed_number_falls_back_and_is_reported() {
        let mut s = Settings::new();
        s.set("max_lines", "many");
        s.set("keep_head", -3i64);

        assert_eq!(s.u64("max_lines", 50), 50);
        assert_eq!(s.u64("keep_head", 7), 7, "a negative is not a u64");

        let keys: Vec<_> = s.rejected().iter().map(|r| r.key).collect();
        assert_eq!(keys, ["max_lines", "keep_head"]);
    }

    #[test]
    fn a_number_written_as_text_is_still_a_number() {
        let mut s = Settings::new();
        s.set("max_lines", "200");
        s.set("collapse", "off");
        assert_eq!(s.u64("max_lines", 50), 200);
        assert!(!s.bool("collapse", true));
        assert!(s.rejected().is_empty());
    }

    #[test]
    fn unknown_keys_surface_as_unread() {
        let s = settings();
        s.u64("max_lines", 50);

        assert_eq!(s.unread(), ["collapse", "style"]);
    }

    #[test]
    fn a_key_read_and_rejected_is_not_an_unknown_key() {
        let mut s = Settings::new();
        s.set("max_lines", "many");
        assert_eq!(s.u64("max_lines", 50), 50);

        assert!(s.unread().is_empty(), "it was asked for, just unusable");
        assert_eq!(s.rejected().len(), 1);
    }

    #[test]
    fn get_does_not_count_as_a_read() {
        let s = settings();
        assert_eq!(s.get("style"), Some(&Value::Str("unified".into())));
        assert!(s.unread().contains(&"style"));
    }

    #[test]
    fn keys_keep_the_order_they_were_set_in() {
        let mut s = settings();
        s.set("max_lines", 10u64);
        let keys: Vec<_> = s.keys().collect();
        assert_eq!(keys, ["max_lines", "collapse", "style"]);
        assert_eq!(s.u64("max_lines", 0), 10, "replaced in place");
    }

    #[test]
    fn replacing_a_value_forgets_what_was_noticed_about_the_old_one() {
        let mut s = Settings::new();
        s.set("max_lines", "many");
        assert_eq!(s.u64("max_lines", 50), 50);
        assert_eq!(s.rejected().len(), 1);

        s.set("max_lines", 200u64);
        assert!(s.rejected().is_empty());
        assert_eq!(s.unread(), ["max_lines"]);
    }

    #[test]
    fn merge_lands_key_by_key() {
        let mut lower = settings();
        let mut upper = Settings::new();
        upper.set("max_lines", 10u64);
        upper.set("new_key", 1u64);

        lower.merge(&upper);

        assert_eq!(lower.u64("max_lines", 0), 10, "overwritten");
        assert!(lower.bool("collapse", false), "untouched");
        assert_eq!(lower.u64("new_key", 0), 1, "added");
        let keys: Vec<_> = lower.keys().collect();
        assert_eq!(keys, ["max_lines", "collapse", "style", "new_key"]);
    }

    #[test]
    fn equality_ignores_what_has_been_read() {
        let a = settings();
        let b = settings();
        a.u64("max_lines", 0);
        assert_eq!(a, b);
    }
}
