//! Client side of input sending: tags each local input with a server tick and repeats the
//! last few in every message.

use std::collections::VecDeque;

use mw_core::input::PlayerInput;
use mw_net::ClientMessage;

/// Inputs repeated in each message (the newest one and the 3 before).
pub const INPUT_REDUNDANCY: usize = 4;

/// How many ticks ahead of the newest snapshot inputs are sent (3 ticks = 50 ms).
///
/// Redundancy only helps if a repeated input still arrives **before** its tick is
/// simulated: with this lead, an input whose message is lost is delivered by one of the
/// next messages in time, as long as fewer than `INPUT_LEAD_TICKS` messages in a row are
/// lost. The price is the same delay on the player's reactions until client prediction
/// (#14) hides it.
pub const INPUT_LEAD_TICKS: u64 = 3;

/// Turns one local input per tick into [`ClientMessage::Inputs`].
///
/// Tick estimate: the next input is for the tick of the newest snapshot plus
/// [`INPUT_LEAD_TICKS`], then one more per call. On loopback (host mode) that is exactly the
/// lead; a remote client additionally loses half a round trip. Tick synchronization that
/// measures it belongs to client prediction (#14).
#[derive(Debug, Clone, Default)]
pub struct InputSender {
    next_tick: Option<u64>,
    /// `(tick, input)`, contiguous ticks, oldest first.
    history: VecDeque<(u64, PlayerInput)>,
}

impl InputSender {
    /// A snapshot of tick `tick` arrived: the server's next tick is at least `tick`.
    pub fn on_snapshot(&mut self, tick: u64) {
        let target = tick.saturating_add(INPUT_LEAD_TICKS);
        match self.next_tick {
            Some(next) if next >= target => {}
            _ => {
                // Jumping ahead breaks the contiguous history: start over.
                self.next_tick = Some(target);
                self.history.clear();
            }
        }
    }

    /// Records `input` for the next tick and returns the message to send, or `None` before
    /// the first snapshot (the tick is unknown).
    pub fn next(&mut self, input: PlayerInput) -> Option<ClientMessage> {
        let tick = self.next_tick?;
        self.next_tick = Some(tick + 1);
        self.history.push_back((tick, input));
        while self.history.len() > INPUT_REDUNDANCY {
            self.history.pop_front();
        }
        let first_tick = self.history.front().map_or(tick, |(t, _)| *t);
        Some(ClientMessage::Inputs {
            first_tick,
            inputs: self.history.iter().map(|(_, input)| *input).collect(),
        })
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

    fn sent(message: Option<ClientMessage>) -> (u64, Vec<i8>) {
        match message.expect("a message") {
            ClientMessage::Inputs { first_tick, inputs } => {
                (first_tick, inputs.iter().map(|i| i.axis_x).collect())
            }
        }
    }

    #[test]
    fn nothing_is_sent_before_the_first_snapshot() {
        let mut sender = InputSender::default();
        assert!(sender.next(input(1)).is_none());
    }

    #[test]
    fn messages_repeat_the_last_inputs() {
        let mut sender = InputSender::default();
        sender.on_snapshot(100);
        let first = 100 + INPUT_LEAD_TICKS;
        assert_eq!(sent(sender.next(input(1))), (first, vec![1]));
        assert_eq!(sent(sender.next(input(2))), (first, vec![1, 2]));
        sender.next(input(3));
        assert_eq!(sent(sender.next(input(4))), (first, vec![1, 2, 3, 4]));
        assert_eq!(sent(sender.next(input(5))), (first + 1, vec![2, 3, 4, 5]));
    }

    #[test]
    fn older_snapshots_do_not_move_the_tick_back() {
        let mut sender = InputSender::default();
        sender.on_snapshot(10);
        sender.next(input(1));
        sender.next(input(2));
        sender.on_snapshot(11); // slower than our own count: keep going
        sender.on_snapshot(10); // late or duplicate snapshot
        assert_eq!(
            sent(sender.next(input(3))),
            (10 + INPUT_LEAD_TICKS, vec![1, 2, 3])
        );
    }

    #[test]
    fn falling_behind_jumps_ahead_and_restarts_the_history() {
        let mut sender = InputSender::default();
        sender.on_snapshot(10);
        sender.next(input(1));
        sender.on_snapshot(50);
        assert_eq!(
            sent(sender.next(input(2))),
            (50 + INPUT_LEAD_TICKS, vec![2])
        );
    }
}
