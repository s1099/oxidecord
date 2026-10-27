//! Rebuilding a watched stream's frames from out-of-order RTP.
//!
//! Packets are held by sequence number until a frame's run is complete, then
//! depacketized (RFC 6184: single units, STAP-A, FU-A) into Annex B. Every unit
//! gets a 4-byte start code because DAVE's sender assumed the receiver rebuilds
//! them that way.
//!
//! Gaps are NACKed at once; one still open after [`GIVE_UP`] is abandoned, its
//! frames dropped and the caller told so it can ask for a keyframe.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// A few round trips, which is all a resend should take.
const GIVE_UP: Duration = Duration::from_millis(300);

/// A gap this far behind is never closing.
const MAX_HELD: usize = 4096;

/// A longer burst is better recovered with a keyframe.
const MAX_NACK: usize = 64;

const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_STAP_A: u8 = 24;
const NAL_FU_A: u8 = 28;

pub(super) struct Frame {
    /// Annex B, four-byte start codes.
    pub data: Vec<u8>,
    /// Decodable on its own: it carries an IDR slice or parameter sets.
    pub keyframe: bool,
}

#[derive(Default)]
pub(super) struct Assembled {
    pub frames: Vec<Frame>,
    pub nack: Vec<u16>,
    /// Frames were dropped; nothing decodes until the next keyframe.
    pub lost: bool,
}

struct Packet {
    timestamp: u32,
    marker: bool,
    payload: Vec<u8>,
    arrived: Instant,
}

#[derive(Default)]
pub(super) struct Assembler {
    packets: BTreeMap<u64, Packet>,
    highest: Option<u64>,
    /// `None` until a frame boundary is seen; joining mid-frame has no start.
    next: Option<u64>,
    nacked: BTreeSet<u64>,
}

impl Assembler {
    pub(super) fn insert(
        &mut self,
        sequence: u16,
        timestamp: u32,
        marker: bool,
        payload: Vec<u8>,
        now: Instant,
    ) {
        let id = self.extend(sequence);
        if self.next.is_some_and(|next| id < next) {
            // A duplicate, or a resend that came too late.
            return;
        }
        if self.packets.len() >= MAX_HELD {
            self.resync();
        }

        self.packets.insert(
            id,
            Packet {
                timestamp,
                marker,
                payload,
                arrived: now,
            },
        );
        if self.next.is_none() && marker {
            // First boundary: the next frame is the first whole one.
            self.packets.retain(|&held, _| held > id);
            self.next = Some(id + 1);
        }
    }

    pub(super) fn assemble(&mut self, now: Instant) -> Assembled {
        let mut out = Assembled::default();
        while let Some(next) = self.next {
            if let Some(end) = self.complete_from(next) {
                let packets: Vec<Packet> = (next..=end)
                    .filter_map(|id| self.packets.remove(&id))
                    .collect();
                if let Some(frame) = depacketize(packets.iter().map(|p| p.payload.as_slice())) {
                    out.frames.push(frame);
                }
                self.next = Some(end + 1);
                continue;
            }

            let Some((&after, packet)) = self.packets.range(next..).next() else {
                break;
            };
            if after != next || self.missing_from(next).is_some() {
                // The oldest packet waiting behind the gap dates it.
                if now.duration_since(packet.arrived) > GIVE_UP {
                    self.skip_gap(next);
                    out.lost = true;
                    continue;
                }
            }
            out.nack = self.report_gaps(next);
            break;
        }
        self.nacked
            .retain(|&id| self.next.is_none_or(|next| id >= next));
        out
    }

    /// A frame ends at its marker, or where the timestamp changes for senders
    /// that omit the marker.
    fn complete_from(&self, next: u64) -> Option<u64> {
        let first = self.packets.get(&next)?;
        let mut id = next;
        loop {
            let packet = self.packets.get(&id)?;
            if packet.timestamp != first.timestamp {
                return Some(id - 1);
            }
            if packet.marker {
                return Some(id);
            }
            id += 1;
        }
    }

    fn missing_from(&self, next: u64) -> Option<u64> {
        let last = *self.packets.keys().next_back()?;
        (next..last).find(|id| !self.packets.contains_key(id))
    }

    fn skip_gap(&mut self, next: u64) {
        let gap = self.missing_from(next).unwrap_or(next);
        let boundary = self
            .packets
            .range(gap..)
            .find(|(_, packet)| packet.marker)
            .map(|(&id, _)| id);
        match boundary {
            Some(end) => {
                self.packets.retain(|&id, _| id > end);
                self.next = Some(end + 1);
            }
            None => self.resync(),
        }
    }

    fn report_gaps(&mut self, next: u64) -> Vec<u16> {
        let Some(&last) = self.packets.keys().next_back() else {
            return Vec::new();
        };
        let mut lost = Vec::new();
        for id in next..last {
            if lost.len() == MAX_NACK {
                break;
            }
            if !self.packets.contains_key(&id) && self.nacked.insert(id) {
                lost.push(id as u16);
            }
        }
        lost
    }

    fn resync(&mut self) {
        self.packets.clear();
        self.nacked.clear();
        self.next = None;
    }

    /// Unwraps a 16-bit sequence number to whichever extension is nearest the highest seen.
    fn extend(&mut self, sequence: u16) -> u64 {
        let id = match self.highest {
            // Well clear of zero so an earlier packet can't go negative.
            None => (1 << 32) + u64::from(sequence),
            Some(highest) => {
                let delta = sequence.wrapping_sub(highest as u16) as i16;
                highest.wrapping_add_signed(i64::from(delta))
            }
        };
        self.highest = Some(self.highest.map_or(id, |highest| highest.max(id)));
        id
    }
}

fn depacketize<'a>(payloads: impl Iterator<Item = &'a [u8]>) -> Option<Frame> {
    let mut data = Vec::new();
    let mut keyframe = false;
    let mut start = |data: &mut Vec<u8>, header: u8| {
        let kind = header & 0x1F;
        keyframe |= kind == NAL_IDR || kind == NAL_SPS;
        data.extend_from_slice(&[0, 0, 0, 1, header]);
    };

    for payload in payloads {
        let Some(&header) = payload.first() else {
            continue;
        };
        match header & 0x1F {
            1..=23 => {
                start(&mut data, header);
                data.extend_from_slice(&payload[1..]);
            }
            NAL_STAP_A => {
                let mut rest = &payload[1..];
                while let [high, low, tail @ ..] = rest {
                    let length = usize::from(u16::from_be_bytes([*high, *low]));
                    let Some(unit) = tail.get(..length).filter(|unit| !unit.is_empty()) else {
                        break;
                    };
                    start(&mut data, unit[0]);
                    data.extend_from_slice(&unit[1..]);
                    rest = &tail[length..];
                }
            }
            NAL_FU_A => {
                let Some(&fu_header) = payload.get(1) else {
                    continue;
                };
                if fu_header & 0x80 != 0 {
                    start(&mut data, (header & 0xE0) | (fu_header & 0x1F));
                }
                data.extend_from_slice(&payload[2..]);
            }
            _ => {}
        }
    }

    (!data.is_empty()).then_some(Frame { data, keyframe })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(assembler: &mut Assembler, packets: &[(u16, u32, bool, &[u8])], now: Instant) {
        for &(sequence, timestamp, marker, payload) in packets {
            assembler.insert(sequence, timestamp, marker, payload.to_vec(), now);
        }
    }

    #[test]
    fn frames_come_out_in_order_once_whole() {
        let now = Instant::now();
        let mut assembler = Assembler::default();
        // Tail of a frame joined midway, then two whole ones, the second out of order.
        feed(
            &mut assembler,
            &[
                (9, 0, true, &[0x41, 0]),
                (10, 1, false, &[0x67, 1]),
                (11, 1, true, &[0x65, 2]),
                (13, 2, true, &[0x41, 4]),
            ],
            now,
        );
        let out = assembler.assemble(now);
        assert_eq!(out.frames.len(), 1);
        assert!(out.frames[0].keyframe);
        assert_eq!(
            out.frames[0].data,
            [0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 2]
        );
        assert_eq!(out.nack, [12]);

        feed(&mut assembler, &[(12, 2, false, &[0x41, 3])], now);
        let out = assembler.assemble(now);
        assert_eq!(out.frames.len(), 1);
        assert!(!out.frames[0].keyframe);
        assert_eq!(
            out.frames[0].data,
            [0, 0, 0, 1, 0x41, 3, 0, 0, 0, 1, 0x41, 4]
        );
        assert!(!out.lost);
    }

    #[test]
    fn a_gap_left_open_is_given_up_on() {
        let now = Instant::now();
        let mut assembler = Assembler::default();
        feed(
            &mut assembler,
            &[
                (65534, 0, true, &[0x41]),
                // 65535 never arrives; 0 ends its frame across the wrap.
                (0, 1, true, &[0x41, 1]),
                (1, 2, true, &[0x41, 2]),
            ],
            now,
        );
        let out = assembler.assemble(now);
        assert!(out.frames.is_empty());
        assert_eq!(out.nack, [65535]);
        // Asked for once, not on every pass.
        assert!(assembler.assemble(now).nack.is_empty());

        let out = assembler.assemble(now + GIVE_UP * 2);
        assert!(out.lost);
        assert_eq!(out.frames.len(), 1);
        assert_eq!(out.frames[0].data, [0, 0, 0, 1, 0x41, 2]);
    }

    #[test]
    fn stap_a_and_fu_a_are_unpacked() {
        let stap: &[u8] = &[0x18, 0, 2, 0x67, 9, 0, 2, 0x68, 8];
        let fu_start: &[u8] = &[0x7C, 0x85, 1, 2];
        let fu_end: &[u8] = &[0x7C, 0x45, 3];
        let frame = depacketize([stap, fu_start, fu_end].into_iter()).unwrap();
        assert!(frame.keyframe);
        assert_eq!(
            frame.data,
            [
                0, 0, 0, 1, 0x67, 9, 0, 0, 0, 1, 0x68, 8, 0, 0, 0, 1, 0x65, 1, 2, 3
            ]
        );
    }
}
