//! Errors raised while building or running a pipeline.
//!
//! Libraries use `thiserror`; only the binary uses `anyhow` (see
//! `docs/CODE_STYLE.md`).

/// The result type used across this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong in `lessr-core`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A stage was registered under a name already in use. Stage names are the
    /// keys used by [`crate::Position`] and by the receipt, so they must be
    /// unique.
    #[error("a stage named `{0}` is already registered")]
    DuplicateStage(String),

    /// A [`crate::Position`] referred to a stage that is not registered. This
    /// is usually a Pro crate asking to sit next to a free stage that the
    /// caller did not register.
    #[error("no stage named `{0}` is registered to position against")]
    UnknownStage(String),

    /// The handle store could not be read or written.
    #[error("handle store: {0}")]
    HandleStore(#[source] std::io::Error),

    /// A record in the handle store file was truncated or malformed.
    #[error("handle store: record for `{0}` is corrupt")]
    CorruptHandle(String),

    /// Two different contents hashed to the same handle id at every length we
    /// are willing to print. A blake3 prefix collision this long means
    /// something is wrong with the input, not with the odds.
    #[error("handle store: cannot find a free id for this content")]
    HandleCollision,
}
