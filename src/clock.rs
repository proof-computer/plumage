//! Wall time mapped onto the day being simulated.

use serde::{Deserialize, Serialize};

use crate::day::Day;
use crate::error::{Error, Result};

/// Maps the store's wall clock onto the simulated day.
///
/// Every stored time stays `forthebirds_store::now()`. This clock is consulted
/// only where a time is shown or decided. Speed 1 with equal starts is the
/// identity on and after that start, which is the real lab.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DayClock {
    pub real_start_ms: u64,
    pub sim_start_ms: u64,
    pub speed: u64,
}

impl DayClock {
    /// `sim_start_ms + (real_ms - real_start_ms) * speed`, saturating the
    /// subtraction, the product and the sum. A real time before the start
    /// stays at `sim_start_ms`.
    pub fn sim(&self, real_ms: u64) -> u64 {
        let elapsed = real_ms.saturating_sub(self.real_start_ms);
        self.sim_start_ms
            .saturating_add(elapsed.saturating_mul(self.speed))
    }

    /// Simulated time of the store's current wall clock.
    pub fn now_ms(&self) -> u64 {
        self.sim(forthebirds_store::now())
    }

    /// Inverse of [`Self::sim`], rounded up, so a sleep until `sim_ms` does not
    /// wake before that simulated instant.
    pub fn real(&self, sim_ms: u64) -> u64 {
        if self.speed == 0 {
            return self.real_start_ms;
        }
        let elapsed = sim_ms.saturating_sub(self.sim_start_ms);
        let steps = elapsed.saturating_add(self.speed - 1) / self.speed;
        self.real_start_ms.saturating_add(steps)
    }

    /// The simulated UTC date.
    pub fn day(&self) -> Day {
        Day::from_ms(self.now_ms())
    }

    /// `speed` is 1–100 000. Anything else is `config`.
    pub fn validate(&self) -> Result<()> {
        if !(1..=100_000).contains(&self.speed) {
            return Err(Error::new(
                "config",
                "day clock speed must be from 1 to 100000",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_2880_maps_1250_real_ms_to_one_simulated_hour_and_back() {
        let real_start_ms = 1_700_000_000_000;
        let sim_start_ms = 50_000;
        let clock = DayClock {
            real_start_ms,
            sim_start_ms,
            speed: 2880,
        };
        assert_eq!(clock.sim(real_start_ms + 1_250), sim_start_ms + 3_600_000);
        assert_eq!(clock.real(sim_start_ms + 3_600_000), real_start_ms + 1_250);
    }

    #[test]
    fn speed_outside_1_to_100000_is_config() {
        for speed in [0, 100_001] {
            let error = DayClock {
                real_start_ms: 0,
                sim_start_ms: 0,
                speed,
            }
            .validate()
            .unwrap_err();
            assert_eq!(error.code, "config");
        }
        assert!(
            DayClock {
                real_start_ms: 0,
                sim_start_ms: 0,
                speed: 1,
            }
            .validate()
            .is_ok()
        );
        assert!(
            DayClock {
                real_start_ms: 0,
                sim_start_ms: 0,
                speed: 100_000,
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn speed_one_with_equal_starts_is_the_identity_on_and_after_the_start() {
        let start = 1_700_000_000_000;
        let clock = DayClock {
            real_start_ms: start,
            sim_start_ms: start,
            speed: 1,
        };
        for offset in [0, 1, 1_250, 3_600_000] {
            assert_eq!(clock.sim(start + offset), start + offset);
            assert_eq!(clock.real(start + offset), start + offset);
        }
        assert_eq!(clock.sim(start.saturating_sub(5)), start);
    }

    #[test]
    fn now_ms_uses_the_store_clock() {
        let clock = DayClock {
            real_start_ms: 0,
            sim_start_ms: 0,
            speed: 1,
        };
        let before = forthebirds_store::now();
        let now = clock.now_ms();
        let after = forthebirds_store::now();
        assert!(before <= now && now <= after);
        assert_eq!(clock.day(), Day::from_ms(now));
    }

    #[test]
    fn serde_rejects_an_unknown_field() {
        let parsed = serde_json::from_str::<DayClock>(
            r#"{"real_start_ms":1,"sim_start_ms":1,"speed":1,"extra":true}"#,
        );
        assert!(parsed.is_err());
    }
}
