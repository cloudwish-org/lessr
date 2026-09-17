//! localhost endpoint, streaming passthrough and usage capture.
//!
//! Stage: none.
//! Path: proxy.
//! Counting rule: none of its own; it captures the usage fields the receipt trusts.
//!
//! Not implemented yet. See `docs/MECHANISMS.md` for the contract this crate
//! owes and `docs/business/ROADMAP.md` in the Pro repository for when it is
//! due. Registering nothing is the safe state: a pipeline with no stages is a
//! passthrough.
