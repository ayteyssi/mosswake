//! Per-player inputs waiting for their tick on the server.

use std::collections::BTreeMap;

use mw_core::input::PlayerInput;

/// How far ahead of the server a client may send inputs, in ticks (2 s). Later ones are
/// dropped, which also bounds the memory a client can make the server hold.
pub const MAX_INPUT_LEAD_TICKS: u64 = 120;

/// Inputs received from one player, keyed by the tick they are for.
///
/// Clients send each input several times (redundancy against packet loss): duplicates are
/// harmless. An input for a tick already simulated arrives too late and is dropped. When a
/// tick has no input, the last known one is repeated, so a lost packet does not make the
/// player stop for a frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputBuffer {
    pending: BTreeMap<u64, PlayerInput>,
    last: PlayerInput,
}

impl InputBuffer {
    /// Stores `inputs[i]` for tick `first_tick + i`, given that `current` is the next tick
    /// the server will simulate.
    pub fn insert(&mut self, first_tick: u64, inputs: &[PlayerInput], current: u64) {
        let newest_allowed = current.saturating_add(MAX_INPUT_LEAD_TICKS);
        for (tick, input) in (first_tick..).zip(inputs) {
            if tick > newest_allowed {
                break;
            }
            if tick >= current {
                self.pending.insert(tick, *input);
            }
        }
    }

    /// The input to simulate `tick` with. Forgets everything up to `tick`.
    pub fn take(&mut self, tick: u64) -> PlayerInput {
        let later = self.pending.split_off(&(tick.saturating_add(1)));
        if let Some(input) = self.pending.remove(&tick) {
            self.last = input;
        }
        self.pending = later;
        self.last
    }

    /// Number of inputs waiting (for tests and diagnostics).
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(axis_x: i8) -> PlayerInput {
        PlayerInput {
            axis_x,
            ..Default::default()
        }
    }

    #[test]
    fn inputs_are_used_on_their_tick() {
        let mut buffer = InputBuffer::default();
        buffer.insert(10, &[input(1), input(2), input(3)], 10);
        assert_eq!(buffer.take(10), input(1));
        assert_eq!(buffer.take(11), input(2));
        assert_eq!(buffer.take(12), input(3));
        assert!(buffer.is_empty());
    }

    #[test]
    fn redundant_copies_are_harmless() {
        let mut buffer = InputBuffer::default();
        buffer.insert(10, &[input(1), input(2)], 10);
        buffer.insert(10, &[input(1), input(2), input(3)], 10);
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.take(10), input(1));
        // Next message repeats ticks 11 and 12 and adds 13.
        buffer.insert(11, &[input(2), input(3), input(4)], 11);
        assert_eq!(
            [buffer.take(11), buffer.take(12), buffer.take(13)],
            [input(2), input(3), input(4)]
        );
    }

    #[test]
    fn late_inputs_are_dropped() {
        let mut buffer = InputBuffer::default();
        buffer.insert(5, &[input(1), input(2), input(3)], 7);
        assert_eq!(buffer.len(), 1, "only tick 7 is still to come");
        assert_eq!(buffer.take(7), input(3));
    }

    #[test]
    fn missing_ticks_repeat_the_last_input() {
        let mut buffer = InputBuffer::default();
        assert_eq!(
            buffer.take(0),
            PlayerInput::default(),
            "nothing received yet"
        );
        buffer.insert(1, &[input(5)], 1);
        assert_eq!(buffer.take(1), input(5));
        assert_eq!(buffer.take(2), input(5), "lost packet: keep going");
        buffer.insert(4, &[input(-5)], 3);
        assert_eq!(buffer.take(3), input(5));
        assert_eq!(buffer.take(4), input(-5));
    }

    #[test]
    fn skipped_ticks_are_forgotten() {
        let mut buffer = InputBuffer::default();
        buffer.insert(0, &[input(1), input(2), input(3)], 0);
        assert_eq!(buffer.take(2), input(3), "ticks 0 and 1 were never used");
        assert!(buffer.is_empty());
    }

    #[test]
    fn far_future_inputs_are_dropped() {
        let mut buffer = InputBuffer::default();
        let flood = vec![input(1); 10_000];
        buffer.insert(0, &flood, 0);
        assert_eq!(buffer.len() as u64, MAX_INPUT_LEAD_TICKS + 1);
        buffer.insert(u64::MAX - 1, &[input(1); 4], 0);
        assert_eq!(buffer.len() as u64, MAX_INPUT_LEAD_TICKS + 1, "no overflow");
    }
}
