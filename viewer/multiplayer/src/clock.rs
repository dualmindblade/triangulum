use crate::{ClockEvent, ClockEventKind, ClockState};
use std::time::Instant;

/// Server-owned clock evaluated from a monotonic process epoch. Seeking and
/// rate changes first fold "now" into a fresh anchor, so neither operation
/// creates an accidental discontinuity beyond an explicit seek.
pub struct AuthoritativeClock {
    epoch: Instant,
    anchor_mono_ms: u64,
    anchor_abs_s: f64,
    time_scale: f64,
    sequence: u64,
}

impl AuthoritativeClock {
    pub fn new(absolute_time_s: f64, time_scale: f64) -> Self {
        Self {
            epoch: Instant::now(),
            anchor_mono_ms: 0,
            anchor_abs_s: finite_nonnegative(absolute_time_s, 0.0),
            time_scale: valid_scale(time_scale).unwrap_or(1.0),
            sequence: 0,
        }
    }

    pub fn monotonic_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    fn absolute_at(&self, mono_ms: u64) -> f64 {
        self.anchor_abs_s + (mono_ms as f64 - self.anchor_mono_ms as f64) * 0.001 * self.time_scale
    }

    pub fn state(&self) -> ClockState {
        let mono_ms = self.monotonic_ms();
        ClockState {
            sequence: self.sequence,
            absolute_time_s: self.absolute_at(mono_ms),
            time_scale: self.time_scale,
            server_mono_ms: mono_ms,
        }
    }

    pub fn seek(&mut self, absolute_time_s: f64) -> Result<ClockEvent, String> {
        if !absolute_time_s.is_finite() || absolute_time_s < 0.0 {
            return Err("clock seek must be a finite non-negative number".into());
        }
        let now = self.monotonic_ms();
        self.anchor_mono_ms = now;
        self.anchor_abs_s = absolute_time_s;
        self.sequence += 1;
        Ok(ClockEvent {
            kind: ClockEventKind::Seek,
            state: self.state(),
        })
    }

    pub fn set_time_scale(&mut self, time_scale: f64) -> Result<ClockEvent, String> {
        let time_scale = valid_scale(time_scale)
            .ok_or_else(|| "time scale must be finite and greater than zero".to_string())?;
        let now = self.monotonic_ms();
        let absolute = self.absolute_at(now);
        self.anchor_mono_ms = now;
        self.anchor_abs_s = absolute;
        self.time_scale = time_scale;
        self.sequence += 1;
        Ok(ClockEvent {
            kind: ClockEventKind::TimeScale,
            state: self.state(),
        })
    }
}

fn valid_scale(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0 && value <= 1_000_000.0).then_some(value)
}

fn finite_nonnegative(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        fallback
    }
}

/// Pure client-side two-second convergence law. `elapsed_s` is local
/// monotonic time since receipt of the authoritative state. The target keeps
/// advancing at server scale while the initial error is removed with a
/// smoothstep, avoiding a visible sun/weather jump.
#[derive(Clone, Debug)]
pub struct ClockSlew {
    local_start_s: f64,
    server_start_s: f64,
    time_scale: f64,
    duration_s: f64,
}

impl ClockSlew {
    pub fn new(local_start_s: f64, server: &ClockState, duration_s: f64) -> Self {
        Self {
            local_start_s: finite_nonnegative(local_start_s, 0.0),
            server_start_s: finite_nonnegative(server.absolute_time_s, 0.0),
            time_scale: valid_scale(server.time_scale).unwrap_or(1.0),
            duration_s: if duration_s.is_finite() && duration_s > 0.0 {
                duration_s
            } else {
                2.0
            },
        }
    }

    pub fn sample(&self, elapsed_s: f64) -> f64 {
        let elapsed_s = elapsed_s.max(0.0);
        let server_now = self.server_start_s + elapsed_s * self.time_scale;
        let x = (elapsed_s / self.duration_s).clamp(0.0, 1.0);
        let smooth = x * x * (3.0 - 2.0 * x);
        server_now + (self.local_start_s - self.server_start_s) * (1.0 - smooth)
    }

    pub fn server_sample(&self, elapsed_s: f64) -> f64 {
        self.server_start_s + elapsed_s.max(0.0) * self.time_scale
    }

    pub fn complete(&self, elapsed_s: f64) -> bool {
        elapsed_s >= self.duration_s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slew_starts_local_and_lands_exactly_on_advancing_server() {
        let state = ClockState {
            sequence: 0,
            absolute_time_s: 100.0,
            time_scale: 10.0,
            server_mono_ms: 0,
        };
        let slew = ClockSlew::new(40.0, &state, 2.0);
        assert_eq!(slew.sample(0.0), 40.0);
        assert!(slew.sample(1.0) > 40.0);
        assert!((slew.sample(2.0) - 120.0).abs() < 1e-12);
        assert!((slew.sample(8.0) - 180.0).abs() < 1e-12);
    }

    #[test]
    fn scale_change_is_continuous() {
        let mut clock = AuthoritativeClock::new(25.0, 1.0);
        let before = clock.state().absolute_time_s;
        let event = clock.set_time_scale(60.0).unwrap();
        assert!((event.state.absolute_time_s - before).abs() < 0.02);
        assert_eq!(event.state.sequence, 1);
        assert_eq!(event.state.time_scale, 60.0);
    }

    #[test]
    fn seek_event_is_exact_at_its_monotonic_timestamp() {
        let mut clock = AuthoritativeClock::new(25.0, 1_000_000.0);
        let event = clock.seek(0.0).unwrap();
        assert_eq!(event.state.absolute_time_s, 0.0);
        assert_eq!(
            event.state.at_server_mono_ms(event.state.server_mono_ms),
            0.0
        );
    }
}
