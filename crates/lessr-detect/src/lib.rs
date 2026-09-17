//! Cache-break detector. Report only; never mutates a request.
//!
//! Stage: detect.
//! Path: proxy.
//! Counting rule: cache_creation_input_tokens on broken turns at the write rate, exact.
//!
//! Not implemented yet. See `docs/MECHANISMS.md` for the contract this crate
//! owes and `docs/business/ROADMAP.md` in the Pro repository for when it is
//! due. Registering nothing is the safe state: a pipeline with no stages is a
//! passthrough.
