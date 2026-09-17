//! Bytes to tokens, with a correction learned from what the provider actually
//! billed.
//!
//! Provider usage fields are the truth. Where a mechanism must estimate —
//! bytes removed before the request is sent — it uses bytes/4 corrected by
//! observed `(bytes sent, input tokens)` pairs. Everything this produces is
//! [`Tokens::Estimate`], and the receipt marks it.

use crate::types::Tokens;

/// Starting point before any observation: the usual English-and-code ratio.
const DEFAULT_BYTES_PER_TOKEN: f64 = 4.0;

/// Ratios outside this range are a parsing bug, not a tokeniser. Clamping keeps
/// one bad sample from poisoning the receipt.
const MIN_BYTES_PER_TOKEN: f64 = 1.5;
/// See [`MIN_BYTES_PER_TOKEN`].
const MAX_BYTES_PER_TOKEN: f64 = 8.0;

/// Converts byte counts into token counts for one provider.
#[derive(Clone, Copy, Debug)]
pub struct Estimator {
    bytes_per_token: f64,
    samples: u32,
}

impl Estimator {
    /// An estimator that has seen nothing yet.
    pub const fn new() -> Self {
        Self {
            bytes_per_token: DEFAULT_BYTES_PER_TOKEN,
            samples: 0,
        }
    }

    /// Estimate the tokens in `bytes`.
    ///
    /// Always [`Tokens::Estimate`]: this is arithmetic on a learned ratio, not
    /// a tokeniser, and the difference belongs on the receipt.
    pub fn tokens(&self, bytes: u64) -> Tokens {
        let n = (bytes as f64 / self.bytes_per_token).round();
        Tokens::Estimate(n as u64)
    }

    /// Fold in one observed `(bytes, tokens)` pair from a real request.
    ///
    /// A running mean, so early samples do not dominate and late ones still
    /// move it. Pairs that imply an impossible ratio are ignored.
    pub fn observe(&mut self, bytes: u64, tokens: u64) {
        if tokens == 0 || bytes == 0 {
            return;
        }
        let ratio = bytes as f64 / tokens as f64;
        if !(MIN_BYTES_PER_TOKEN..=MAX_BYTES_PER_TOKEN).contains(&ratio) {
            return;
        }
        self.samples = self.samples.saturating_add(1);
        let weight = 1.0 / f64::from(self.samples);
        self.bytes_per_token += (ratio - self.bytes_per_token) * weight;
    }

    /// The correction currently in use. The receipt names it under
    /// `lessr gain --explain`.
    pub const fn bytes_per_token(&self) -> f64 {
        self.bytes_per_token
    }

    /// How many real requests have fed the correction.
    pub const fn samples(&self) -> u32 {
        self.samples
    }
}

impl Default for Estimator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_four_bytes_per_token() {
        let e = Estimator::new();
        assert_eq!(e.tokens(4000), Tokens::Estimate(1000));
        assert!(e.tokens(4000).is_estimate());
    }

    #[test]
    fn observation_moves_the_ratio_towards_the_truth() {
        let mut e = Estimator::new();
        for _ in 0..50 {
            e.observe(3000, 1000); // this provider bills 3 bytes to the token
        }
        assert!(
            (e.bytes_per_token() - 3.0).abs() < 0.01,
            "{}",
            e.bytes_per_token()
        );
        assert_eq!(e.tokens(3000), Tokens::Estimate(1000));
    }

    #[test]
    fn nonsense_samples_are_ignored() {
        let mut e = Estimator::new();
        e.observe(1_000_000, 1); // a parsing bug, not a tokeniser
        e.observe(0, 0);
        e.observe(100, 0);
        assert_eq!(e.samples(), 0);
        assert_eq!(e.bytes_per_token(), DEFAULT_BYTES_PER_TOKEN);
    }
}
