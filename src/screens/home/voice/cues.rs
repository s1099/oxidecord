//! Audible membership transitions, separate from snapshots and connection requests.

use twilight_model::id::{Id, marker::ChannelMarker};

use crate::voice::cues::Cue;

#[derive(Default)]
pub(in crate::screens::home) struct CuePolicy {
    target: Option<Id<ChannelMarker>>,
    confirmed: Option<Id<ChannelMarker>>,
    next_attempt: u64,
    attempt: Option<u64>,
    connected: bool,
}

impl CuePolicy {
    pub(in crate::screens::home) fn request(&mut self, channel: Id<ChannelMarker>) {
        self.target = Some(channel);
        self.attempt = None;
        self.connected = false;
    }

    pub(in crate::screens::home) fn connecting(&mut self) -> u64 {
        self.next_attempt += 1;
        self.attempt = Some(self.next_attempt);
        self.connected = false;
        self.next_attempt
    }

    pub(in crate::screens::home) fn accepts(&self, attempt: u64) -> bool {
        self.target.is_some() && self.attempt == Some(attempt)
    }

    pub(in crate::screens::home) fn connected(&mut self, attempt: u64) -> Option<Cue> {
        if !self.accepts(attempt) {
            return None;
        }
        self.connected = true;
        let changed = self.confirmed != self.target;
        self.confirmed = self.target;
        changed.then_some(Cue::Join)
    }

    pub(in crate::screens::home) fn failed(&mut self, attempt: u64) {
        if self.accepts(attempt) {
            self.attempt = None;
            self.connected = false;
        }
    }

    pub(in crate::screens::home) fn end(&mut self) -> Option<Cue> {
        self.target = None;
        self.attempt = None;
        self.connected = false;
        self.confirmed.take().map(|_| Cue::Leave)
    }

    pub(in crate::screens::home) fn peer(
        &self,
        before: Option<Id<ChannelMarker>>,
        after: Option<Id<ChannelMarker>>,
    ) -> Option<Cue> {
        let channel = self.confirmed?;
        if !self.connected || before == after {
            None
        } else if after == Some(channel) {
            Some(Cue::PeerJoin)
        } else if before == Some(channel) {
            Some(Cue::PeerLeave)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: u64) -> Id<ChannelMarker> {
        Id::new(id)
    }

    #[test]
    fn only_confirmed_connections_announce_join_and_leave_once() {
        let mut policy = CuePolicy::default();
        policy.request(channel(1));
        let attempt = policy.connecting();
        assert_eq!(policy.peer(None, Some(channel(1))), None);
        assert_eq!(policy.connected(attempt), Some(Cue::Join));
        assert_eq!(policy.connected(attempt), None);
        assert_eq!(policy.end(), Some(Cue::Leave));
        assert_eq!(policy.end(), None);
        assert_eq!(policy.connected(attempt), None);
    }

    #[test]
    fn cancelled_failed_and_superseded_attempts_stay_quiet() {
        let mut policy = CuePolicy::default();
        policy.request(channel(1));
        let cancelled = policy.connecting();
        assert_eq!(policy.end(), None);
        policy.request(channel(2));
        let failed = policy.connecting();
        assert_eq!(policy.connected(cancelled), None);
        policy.failed(failed);
        assert_eq!(policy.connected(failed), None);
        assert_eq!(policy.end(), None);
    }

    #[test]
    fn peers_only_announce_boundary_crossings_of_the_connected_channel() {
        let mut policy = CuePolicy::default();
        policy.request(channel(1));
        let attempt = policy.connecting();
        policy.connected(attempt);
        assert_eq!(policy.peer(None, Some(channel(1))), Some(Cue::PeerJoin));
        assert_eq!(policy.peer(Some(channel(1)), None), Some(Cue::PeerLeave));
        assert_eq!(
            policy.peer(Some(channel(2)), Some(channel(1))),
            Some(Cue::PeerJoin)
        );
        assert_eq!(
            policy.peer(Some(channel(1)), Some(channel(2))),
            Some(Cue::PeerLeave)
        );
        assert_eq!(policy.peer(Some(channel(2)), Some(channel(3))), None);
        // Repeated state, mute/deafen and speaking do not change membership.
        assert_eq!(policy.peer(Some(channel(1)), Some(channel(1))), None);
    }

    #[test]
    fn moves_announce_one_destination_join_but_same_channel_reconnects_do_not() {
        let mut policy = CuePolicy::default();
        policy.request(channel(1));
        let first = policy.connecting();
        policy.connected(first);
        let reconnect = policy.connecting();
        assert_eq!(policy.connected(reconnect), None);
        policy.request(channel(2));
        assert_eq!(policy.peer(None, Some(channel(1))), None);
        let moved = policy.connecting();
        assert_eq!(policy.connected(reconnect), None);
        assert_eq!(policy.connected(moved), Some(Cue::Join));
        assert_eq!(policy.end(), Some(Cue::Leave));
    }

    #[test]
    fn policy_plays_same_original_join_but_distinct_self_and_peer_departures() {
        let mut policy = CuePolicy::default();
        policy.request(channel(1));
        let attempt = policy.connecting();
        let own_join = policy.connected(attempt).unwrap().clip().unwrap();
        let peer_join = policy.peer(None, Some(channel(1))).unwrap().clip().unwrap();
        let peer_leave = policy.peer(Some(channel(1)), None).unwrap().clip().unwrap();
        let own_leave = policy.end().unwrap().clip().unwrap();
        assert!(std::ptr::eq(own_join, peer_join));
        assert!(!std::ptr::eq(own_leave, peer_leave));
        assert!(!std::ptr::eq(own_join, own_leave));
    }
}
