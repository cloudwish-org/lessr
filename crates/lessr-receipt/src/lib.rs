//! Recorder (channel and background thread), SQLite WAL, the queries behind lessr gain.
//!
//! Stage: none.
//! Path: off the hot path.
//! Counting rule: none of its own; it stores what the stages count.
//!
//! Not implemented yet. See `docs/MECHANISMS.md` for the contract this crate
//! owes and `docs/business/ROADMAP.md` in the Pro repository for when it is
//! due. Registering nothing is the safe state: a pipeline with no stages is a
//! passthrough.
