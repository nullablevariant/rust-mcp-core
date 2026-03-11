//! Clock abstraction for deterministic OAuth expiry and refresh behavior.

use chrono::{DateTime, Utc};

// Wall-clock abstraction used by OAuth token management code.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

// Production clock implementation backed by `Utc::now()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[cfg(test)]
// Inline tests validate private clock behavior local to this module.
mod tests {
    use super::{Clock, SystemClock};
    use chrono::Utc;

    #[test]
    fn system_clock_now_is_close_to_utc_now() {
        let clock = SystemClock;
        let before = Utc::now();
        let now = clock.now();
        let after = Utc::now();

        assert!(now >= before);
        assert!(now <= after);

        let progressed = (0..10_000).any(|_| clock.now() > now);
        assert!(progressed, "system clock should eventually progress");
    }
}
