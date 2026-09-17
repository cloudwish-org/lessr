//! Re-read dedup
//!
//! Stage:  unchanged files become a hash line, changed files a diff..
//! Path: dedup.
//! Counting rule: hook:full content bytes minus reply bytes, exact.
//!
//! Not implemented yet. See `docs/MECHANISMS.md` for the contract this crate
//! owes and `docs/business/ROADMAP.md` in the Pro repository for when it is
//! due. Registering nothing is the safe state: a pipeline with no stages is a
//! passthrough.
