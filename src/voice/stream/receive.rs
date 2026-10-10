//! Putting a watched stream's frames back together from RTP.
//!
//! Packets arrive out of order and some not at all. They're held by sequence
//! number until a frame's run is complete — every packet from the one after
//! the last frame's marker up to this one's — and only then depacketized
//! (RFC 6184: single units, STAP-A, FU-A) into the Annex B the DAVE
//! decryptor and the decoder read. Each unit gets a four-byte start code,
//! since that's what DAVE's sender assumed the receiver would rebuild.
//!
//! A gap is reported for retransmission as soon as it's seen. One that's
//! still open after [`GIVE_UP`] is abandoned: the frames it held are dropped
//! and the caller is told, so it can ask for a keyframe to start over from.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long a gap waits for a retransmission before the frame it's in is
/// given up on. A few round trips, which is all a resend should take.
const GIVE_UP: Duration = Duration::from_millis(300);

/// Packets held at most. A gap this far behind is never closing.
const MAX_HELD: usize = 4096;

/// Lost packets reported in one NACK. A burst longer than this is better
/// recovered from with a keyframe than packet by packet.
const MAX_NACK: usize = 64;

const NAL_IDR: u8 = 5;
const NAL_SPS: u8 = 7;
const NAL_STAP_A: u8 = 24;
const NAL_FU_A: u8 = 28;

/// One reassembled frame.
pub(super) struct Frame {
    /// Annex B, four-byte start codes.
    pub data: Vec<u8>,
    /// Decodable on its own: it carries an IDR slice or parameter sets.
    pub keyframe: bool,
}

/// What came of the packets so far.
#[derive(Default)]
pub(super) struct Assembled {
    pub frames: Vec<Frame>,
    /// Packets to ask the sender for again.
    pub nack: Vec<u16>,
    /// Frames were given up on; what follows can't be decoded until the next
    /// keyframe.
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
    /// Held packets, by extended sequence number.
    packets: BTreeMap<u64, Packet>,
    /// The highest sequence number seen, extended past wrapping.
    highest: Option<u64>,
    /// Where the next frame starts. `None` until a frame boundary has been
    /// seen, since a stream joined mid-frame has no start to begin at.
    next: Option<u64>,
    /// Packets already asked for, so a gap isn't reported on every pass.
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
            // The first boundary: the frame after this one is the first whole
            // one.
            self.packets.retain(|&held, _| held > id);
            self.next = Some(id + 1);
        }
    }

    /// Takes every frame that's complete, and says what's missing.
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

            // Incomplete: there's a gap at or after `next`, if anything at
            // all is held beyond it.
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

    /// Where the frame starting at `next` ends, if every packet of it is
    /// here. A frame ends at its marker — or, for a sender that leaves the
    /// marker off, where the timestamp changes.
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

    /// The first missing packet between `next` and the highest held.
    fn missing_from(&self, next: u64) -> Option<u64> {
        let last = *self.packets.keys().next_back()?;
        (next..last).find(|id| !self.packets.contains_key(id))
    }

    /// Drops everything up to the end of the frame the gap is in, and starts
    /// again at the frame after it.
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

    /// Lost packets not yet asked for, as sequence numbers.
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

    /// Places a sixteen-bit sequence number on a line that doesn't wrap,
    /// taking it as whichever is nearer the highest seen.
    fn extend(&mut self, sequence: u16) -> u64 {
        let id = match self.highest {
            // Started well clear of zero, so a packet from just before the
            // first can't go negative.
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

/// Rebuilds a frame from its packets' payloads. `None` if nothing in them
/// was a NAL unit.
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
        // The tail of a frame joined midway, then two whole ones, the second
        // arriving out of order.
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
