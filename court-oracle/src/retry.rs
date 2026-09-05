//! Per-case exponential backoff with a give-up circuit breaker.
//!
//! Before this module existed, `poll_once` re-attempted every still-`Filed` case on every poll
//! cycle with no memory of past failures. A case that fails for a fixed, unrecoverable reason
//! (malformed context content, a Claude call that will never succeed for this input, etc.) would
//! re-trigger a fresh, billed Claude API call every single `poll_interval_secs` forever, with no
//! circuit breaker to stop it. This is deliberately simple — an in-memory per-case attempt
//! counter and a computed delay, not a job-queue system — per the scope this was built to.
//!
//! Two separate `RetryTracker` instances are used in `main.rs`: one for the "fully process a
//! `Filed` case" path (Claude call + IPFS publish + `submit_ai_ruling`) and one for the
//! `finalize_ruling` path, since a case can legitimately be backing off on one while succeeding
//! on the other (they're different case_ids at different lifecycle stages in practice, but
//! keeping the trackers separate avoids any cross-talk regardless).
//!
//! Deliberately in-memory only, not persisted across restarts — see `Config::retry_max_attempts`
//! doc comment for why that's an acceptable, non-harmful simplification here (unlike the
//! processed/finalized sets in `state.rs`, losing a retry count on restart just gives a
//! chronically-failing case a fresh set of attempts, not a wasted-but-otherwise-silent redundant
//! success).

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Computes the backoff delay for the `attempt`-th failure (1-indexed: `attempt == 1` is the
/// delay applied after the *first* failure, before the second attempt). Doubles each time
/// (`base * 2^(attempt-1)`), capped at `max`. Saturating throughout so a pathologically large
/// `attempt` (shouldn't happen given `retry_max_attempts` gates this in practice) can never
/// panic or wrap.
pub fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    if attempt == 0 {
        return Duration::ZERO;
    }
    let exponent = (attempt - 1).min(32); // 2^32 seconds already vastly exceeds any sane `max`.
    let multiplier = 1u64.checked_shl(exponent).unwrap_or(u64::MAX);
    let secs = base.as_secs().saturating_mul(multiplier);
    Duration::from_secs(secs).min(max)
}

/// What happened as a result of `RetryTracker::record_failure`, so the caller can log
/// appropriately (a routine "will retry in Ns" vs. a prominent "giving up" that needs an
/// operator's eyes).
#[derive(Debug, PartialEq, Eq)]
pub enum FailureOutcome {
    WillRetry { attempt: u32, delay: Duration },
    GaveUp { attempts: u32 },
}

#[derive(Debug, Clone)]
struct Entry {
    attempts: u32,
    next_attempt_at: Instant,
    given_up: bool,
}

/// Tracks per-`case_id` attempt counts and backoff state. See module doc comment.
pub struct RetryTracker {
    entries: HashMap<u32, Entry>,
    base_delay: Duration,
    max_delay: Duration,
    max_attempts: u32,
}

impl RetryTracker {
    pub fn new(base_delay: Duration, max_delay: Duration, max_attempts: u32) -> Self {
        Self { entries: HashMap::new(), base_delay, max_delay, max_attempts: max_attempts.max(1) }
    }

    /// Whether `case_id` should be attempted right now: true if it has never failed, if its
    /// backoff delay has elapsed, or (defensively) if it isn't marked given-up despite the
    /// bookkeeping saying so. False if it's still within its backoff window or has permanently
    /// given up (until this process restarts, resetting all in-memory state).
    pub fn should_attempt(&self, case_id: u32, now: Instant) -> bool {
        match self.entries.get(&case_id) {
            None => true,
            Some(e) if e.given_up => false,
            Some(e) => now >= e.next_attempt_at,
        }
    }

    /// True if `case_id` has permanently given up (hit `max_attempts`). Distinguished from
    /// `!should_attempt` so callers can tell "still backing off, try again later" apart from
    /// "gave up, will never be retried without a restart" for logging purposes.
    pub fn has_given_up(&self, case_id: u32) -> bool {
        self.entries.get(&case_id).is_some_and(|e| e.given_up)
    }

    /// Clears any tracked failure state for `case_id` — call this once it's successfully fully
    /// processed, so a case_id is never accidentally left in a stale backoff/given-up state
    /// (case_ids are not reused on-chain, but this keeps the map from growing unboundedly across
    /// a long-running process's lifetime, and keeps intent explicit at the call site).
    pub fn record_success(&mut self, case_id: u32) {
        self.entries.remove(&case_id);
    }

    /// Records a failed attempt for `case_id` and returns what the caller should do/log next.
    pub fn record_failure(&mut self, case_id: u32, now: Instant) -> FailureOutcome {
        let entry = self.entries.entry(case_id).or_insert(Entry {
            attempts: 0,
            next_attempt_at: now,
            given_up: false,
        });
        entry.attempts += 1;

        if entry.attempts >= self.max_attempts {
            entry.given_up = true;
            FailureOutcome::GaveUp { attempts: entry.attempts }
        } else {
            let delay = backoff_delay(entry.attempts, self.base_delay, self.max_delay);
            entry.next_attempt_at = now + delay;
            FailureOutcome::WillRetry { attempt: entry.attempts, delay }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── backoff_delay ────────────────────────────────────────────────────────────────────────

    #[test]
    fn zeroth_attempt_has_no_delay() {
        assert_eq!(backoff_delay(0, Duration::from_secs(60), Duration::from_secs(3600)), Duration::ZERO);
    }

    #[test]
    fn delay_doubles_each_attempt_until_the_cap() {
        let base = Duration::from_secs(60);
        let max = Duration::from_secs(3600);
        assert_eq!(backoff_delay(1, base, max), Duration::from_secs(60));
        assert_eq!(backoff_delay(2, base, max), Duration::from_secs(120));
        assert_eq!(backoff_delay(3, base, max), Duration::from_secs(240));
        assert_eq!(backoff_delay(4, base, max), Duration::from_secs(480));
    }

    #[test]
    fn delay_is_capped_at_max() {
        let base = Duration::from_secs(60);
        let max = Duration::from_secs(300);
        // 2^10 * 60s would be far beyond max without the cap.
        assert_eq!(backoff_delay(10, base, max), max);
    }

    #[test]
    fn delay_never_panics_on_a_large_attempt_count() {
        let base = Duration::from_secs(60);
        let max = Duration::from_secs(3600);
        assert_eq!(backoff_delay(u32::MAX, base, max), max);
    }

    // ── RetryTracker ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn a_never_seen_case_should_be_attempted() {
        let tracker = RetryTracker::new(Duration::from_secs(60), Duration::from_secs(3600), 5);
        assert!(tracker.should_attempt(1, Instant::now()));
        assert!(!tracker.has_given_up(1));
    }

    #[test]
    fn after_one_failure_the_case_backs_off_until_the_delay_elapses() {
        let mut tracker = RetryTracker::new(Duration::from_secs(60), Duration::from_secs(3600), 5);
        let t0 = Instant::now();
        let outcome = tracker.record_failure(1, t0);
        assert_eq!(outcome, FailureOutcome::WillRetry { attempt: 1, delay: Duration::from_secs(60) });

        assert!(!tracker.should_attempt(1, t0 + Duration::from_secs(30)), "still within backoff window");
        assert!(tracker.should_attempt(1, t0 + Duration::from_secs(61)), "backoff window has elapsed");
    }

    #[test]
    fn repeated_failures_increase_the_delay() {
        let mut tracker = RetryTracker::new(Duration::from_secs(60), Duration::from_secs(3600), 5);
        let t0 = Instant::now();
        tracker.record_failure(1, t0);
        let outcome = tracker.record_failure(1, t0 + Duration::from_secs(61));
        assert_eq!(outcome, FailureOutcome::WillRetry { attempt: 2, delay: Duration::from_secs(120) });
    }

    #[test]
    fn gives_up_after_max_attempts_and_stops_being_attemptable() {
        let mut tracker = RetryTracker::new(Duration::from_secs(1), Duration::from_secs(60), 3);
        let mut now = Instant::now();
        for expected_attempt in 1..3 {
            let outcome = tracker.record_failure(1, now);
            assert_eq!(
                outcome,
                FailureOutcome::WillRetry { attempt: expected_attempt, delay: backoff_delay(expected_attempt, Duration::from_secs(1), Duration::from_secs(60)) }
            );
            now += Duration::from_secs(120); // well past any plausible delay in this test
        }
        // Third failure hits max_attempts == 3.
        let outcome = tracker.record_failure(1, now);
        assert_eq!(outcome, FailureOutcome::GaveUp { attempts: 3 });
        assert!(tracker.has_given_up(1));
        assert!(!tracker.should_attempt(1, now + Duration::from_secs(1_000_000)), "given-up cases never retry without a restart");
    }

    #[test]
    fn success_clears_tracked_state_so_a_case_id_could_be_attempted_fresh_again() {
        let mut tracker = RetryTracker::new(Duration::from_secs(60), Duration::from_secs(3600), 2);
        let t0 = Instant::now();
        tracker.record_failure(1, t0);
        assert!(!tracker.should_attempt(1, t0));

        tracker.record_success(1);
        assert!(tracker.should_attempt(1, t0), "clearing failure state should make the case immediately attemptable again");
        assert!(!tracker.has_given_up(1));
    }

    #[test]
    fn different_case_ids_are_tracked_independently() {
        let mut tracker = RetryTracker::new(Duration::from_secs(60), Duration::from_secs(3600), 2);
        let t0 = Instant::now();
        tracker.record_failure(1, t0);
        assert!(!tracker.should_attempt(1, t0));
        assert!(tracker.should_attempt(2, t0), "an unrelated case_id must not be affected");
    }

    #[test]
    fn max_attempts_of_zero_is_treated_as_one_so_the_first_failure_still_gives_up() {
        // Defensive: a misconfigured RETRY_MAX_ATTEMPTS=0 must not mean "never give up" (which
        // `entry.attempts >= 0` would trivially always satisfy on attempt 1 anyway, but this
        // pins that a nonsensical zero config degrades safely rather than behaving oddly).
        let mut tracker = RetryTracker::new(Duration::from_secs(60), Duration::from_secs(3600), 0);
        let outcome = tracker.record_failure(1, Instant::now());
        assert_eq!(outcome, FailureOutcome::GaveUp { attempts: 1 });
    }
}
