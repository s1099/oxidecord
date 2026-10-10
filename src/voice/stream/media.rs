//! The stream's UDP side. For the streamer: H.264 frames out as encrypted
//! RTP, viewer feedback in as RTCP. For a viewer: the reverse.
//!
//! Packets are laid out the way Discord's `_rtpsize` modes want them: the
//! RTP header stays in the clear as associated data, and the payload is
//! encrypted, with the tag and a four-byte nonce counter appended. A header
//! extension's four-byte preamble is part of the clear header, but its
//! elements are encrypted with the payload. RTCP is the same with its first
//! eight bytes in the clear.

use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{AeadInPlace as _, KeyInit as _};
use chacha20poly1305::XChaCha20Poly1305;
use tokio::net::UdpSocket;

use crate::platform::capture::nal_units;

pub(super) const AES_MODE: &str = "aead_aes256_gcm_rtpsize";
pub(super) const XCHACHA_MODE: &str = "aead_xchacha20_poly1305_rtpsize";

/// Payload types, as offered in `SELECT_PROTOCOL`.
pub(super) const OPUS_PAYLOAD: u8 = 120;
pub(super) const H264_PAYLOAD: u8 = 101;
pub(super) const RTX_PAYLOAD: u8 = 102;

/// The largest piece of a frame put in one packet. Small enough that the
/// packet, headers and all, clears any path's MTU without fragmenting.
const MAX_PAYLOAD: usize = 1200;

/// Sent packets kept for retransmission. A second of a busy stream, which is
/// longer than any viewer's loss report takes to arrive.
const HISTORY: usize = 1024;

/// Seconds between the NTP epoch (1900) and the Unix one.
const NTP_OFFSET: u64 = 2_208_988_800;

/// The SSRCs the voice server assigned in `READY`.
#[derive(Clone, Copy)]
pub(super) struct Ssrcs {
    pub audio: u32,
    pub video: u32,
    pub rtx: u32,
}

/// What viewers asked of the sender.
#[derive(Default)]
pub(super) struct Feedback {
    /// Someone can't decode the stream until the next keyframe.
    pub keyframe: bool,
    /// Packets someone lost, by sequence number.
    pub lost: Vec<u16>,
}

enum Cipher {
    Aes(Box<Aes256Gcm>),
    XChaCha(XChaCha20Poly1305),
}

impl Cipher {
    fn new(mode: &str, key: &[u8]) -> Result<Self, String> {
        match mode {
            AES_MODE => Aes256Gcm::new_from_slice(key)
                .map(|cipher| Self::Aes(Box::new(cipher)))
                .map_err(|_| "The voice server sent a bad key".into()),
            XCHACHA_MODE => XChaCha20Poly1305::new_from_slice(key)
                .map(Self::XChaCha)
                .map_err(|_| "The voice server sent a bad key".into()),
            other => Err(format!("The voice server chose an unknown cipher: {other}")),
        }
    }

    /// Encrypts `body` in place, appending the tag. The counter goes in the
    /// nonce's first four bytes and the rest stay zero, which is the scheme
    /// the four bytes on the end of each packet describe.
    fn seal(&self, counter: u32, aad: &[u8], body: &mut Vec<u8>) -> Result<(), ()> {
        match self {
            Self::Aes(cipher) => {
                let mut nonce = aes_gcm::Nonce::default();
                nonce[..4].copy_from_slice(&counter.to_be_bytes());
                cipher.encrypt_in_place(&nonce, aad, body).map_err(|_| ())
            }
            Self::XChaCha(cipher) => {
                let mut nonce = chacha20poly1305::XNonce::default();
                nonce[..4].copy_from_slice(&counter.to_be_bytes());
                cipher.encrypt_in_place(&nonce, aad, body).map_err(|_| ())
            }
        }
    }

    /// Decrypts `body` (ciphertext and tag) in place.
    fn open(&self, counter: [u8; 4], aad: &[u8], body: &mut Vec<u8>) -> Result<(), ()> {
        match self {
            Self::Aes(cipher) => {
                let mut nonce = aes_gcm::Nonce::default();
                nonce[..4].copy_from_slice(&counter);
                cipher.decrypt_in_place(&nonce, aad, body).map_err(|_| ())
            }
            Self::XChaCha(cipher) => {
                let mut nonce = chacha20poly1305::XNonce::default();
                nonce[..4].copy_from_slice(&counter);
                cipher.decrypt_in_place(&nonce, aad, body).map_err(|_| ())
            }
        }
    }
}

/// A received RTP packet, decrypted, with its header extension and padding
/// taken off.
pub(super) struct RtpPacket {
    pub payload_type: u8,
    pub marker: bool,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: Vec<u8>,
}

/// A packet as it went out, before encryption, for resending on request.
struct Sent {
    sequence: u16,
    timestamp: u32,
    marker: bool,
    payload: Vec<u8>,
}

pub(super) struct Media {
    socket: UdpSocket,
    cipher: Option<Cipher>,
    nonce: u32,
    ssrcs: Ssrcs,

    sequence: u16,
    rtx_sequence: u16,
    /// Where the stream's RTP clock starts. Random, as RTP asks, so a stream
    /// can't be lined up against another by its timestamps.
    timestamp_base: u32,
    last_timestamp: u32,
    /// When the last frame went out, which a sender report pins the RTP
    /// clock to.
    last_sent: Option<SystemTime>,
    packets: u32,
    octets: u32,

    history: Vec<Option<Sent>>,
}

impl Media {
    /// Opens the socket and learns the address the voice server sees it at,
    /// which is what `SELECT_PROTOCOL` tells it to send to.
    pub(super) async fn connect(
        ip: IpAddr,
        port: u16,
        ssrcs: Ssrcs,
    ) -> Result<(Self, SocketAddr), String> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|err| format!("Couldn't open a media socket: {err}"))?;
        socket
            .connect((ip, port))
            .await
            .map_err(|err| format!("Couldn't reach the stream server: {err}"))?;

        let external = discover(&socket, ssrcs.audio).await?;

        Ok((
            Self {
                socket,
                cipher: None,
                nonce: rand_u32(),
                ssrcs,
                sequence: rand_u32() as u16,
                rtx_sequence: rand_u32() as u16,
                timestamp_base: rand_u32(),
                last_timestamp: 0,
                last_sent: None,
                packets: 0,
                octets: 0,
                history: (0..HISTORY).map(|_| None).collect(),
            },
            external,
        ))
    }

    pub(super) fn set_key(&mut self, mode: &str, key: &[u8]) -> Result<(), String> {
        self.cipher = Some(Cipher::new(mode, key)?);
        Ok(())
    }

    /// Sends one encoded frame, split across as many packets as it takes.
    ///
    /// Each NAL unit goes in a packet of its own when it fits, and is split
    /// into FU-A fragments when it doesn't (RFC 6184, packetization mode 1).
    /// The marker bit goes on the frame's last packet, which is how a viewer
    /// knows the frame is complete.
    pub(super) async fn send_frame(&mut self, frame: &[u8], timestamp: Duration) {
        let timestamp = self
            .timestamp_base
            .wrapping_add((timestamp.as_micros() * 90 / 1000) as u32);
        self.last_timestamp = timestamp;
        self.last_sent = Some(SystemTime::now());

        for (payload, marker) in packetize(frame) {
            self.send_video(payload, timestamp, marker).await;
        }
    }

    async fn send_video(&mut self, payload: Vec<u8>, timestamp: u32, marker: bool) {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);

        let packet = self.rtp(
            H264_PAYLOAD,
            self.ssrcs.video,
            sequence,
            timestamp,
            marker,
            &payload,
        );
        if let Some(packet) = packet {
            self.packets = self.packets.wrapping_add(1);
            self.octets = self.octets.wrapping_add(payload.len() as u32);
            let _ = self.socket.send(&packet).await;
        }

        self.history[usize::from(sequence) % HISTORY] = Some(Sent {
            sequence,
            timestamp,
            marker,
            payload,
        });
    }

    /// Resends lost packets on the retransmission stream (RFC 4588): the same
    /// payload, prefixed with the sequence number it originally had.
    pub(super) async fn retransmit(&mut self, lost: &[u16]) {
        for &sequence in lost {
            let Some(sent) = &self.history[usize::from(sequence) % HISTORY] else {
                continue;
            };
            // The slot may hold a newer packet by now; that one wasn't lost.
            if sent.sequence != sequence {
                continue;
            }

            let mut payload = Vec::with_capacity(sent.payload.len() + 2);
            payload.extend_from_slice(&sequence.to_be_bytes());
            payload.extend_from_slice(&sent.payload);
            let (timestamp, marker) = (sent.timestamp, sent.marker);

            let rtx_sequence = self.rtx_sequence;
            self.rtx_sequence = self.rtx_sequence.wrapping_add(1);
            if let Some(packet) = self.rtp(
                RTX_PAYLOAD,
                self.ssrcs.rtx,
                rtx_sequence,
                timestamp,
                marker,
                &payload,
            ) {
                let _ = self.socket.send(&packet).await;
            }
        }
    }

    /// Builds and encrypts one RTP packet. There's no header extension:
    /// Discord's server silently drops video carrying one (tried: WebRTC's
    /// playout delay), presumably since none was negotiated.
    fn rtp(
        &mut self,
        payload_type: u8,
        ssrc: u32,
        sequence: u16,
        timestamp: u32,
        marker: bool,
        payload: &[u8],
    ) -> Option<Vec<u8>> {
        let mut header = [0u8; 12];
        // Version 2, no padding, extension or CSRCs.
        header[0] = 0x80;
        header[1] = payload_type | if marker { 0x80 } else { 0 };
        header[2..4].copy_from_slice(&sequence.to_be_bytes());
        header[4..8].copy_from_slice(&timestamp.to_be_bytes());
        header[8..12].copy_from_slice(&ssrc.to_be_bytes());
        self.seal(&header, payload.to_vec())
    }

    /// Encrypts `body` behind the clear `header` and appends the nonce.
    fn seal(&mut self, header: &[u8], mut body: Vec<u8>) -> Option<Vec<u8>> {
        let cipher = self.cipher.as_ref()?;
        let counter = self.nonce;
        self.nonce = self.nonce.wrapping_add(1);
        cipher.seal(counter, header, &mut body).ok()?;

        let mut packet = Vec::with_capacity(header.len() + body.len() + 4);
        packet.extend_from_slice(header);
        packet.extend_from_slice(&body);
        packet.extend_from_slice(&counter.to_be_bytes());
        Some(packet)
    }

    /// Sends an RTCP sender report for the video stream, which is what lets a
    /// viewer map its RTP timestamps onto wall-clock time.
    pub(super) async fn send_report(&mut self) {
        let Some(sent_at) = self.last_sent else {
            return;
        };
        let since_epoch = sent_at.duration_since(UNIX_EPOCH).unwrap_or_default();
        let seconds = since_epoch.as_secs() + NTP_OFFSET;
        let fraction = ((u64::from(since_epoch.subsec_nanos()) << 32) / 1_000_000_000) as u32;

        let mut header = [0u8; 8];
        // Version 2, no reception reports; type 200; six words after this one.
        header[0] = 0x80;
        header[1] = 200;
        header[2..4].copy_from_slice(&6u16.to_be_bytes());
        header[4..8].copy_from_slice(&self.ssrcs.video.to_be_bytes());

        let mut body = Vec::with_capacity(20 + 16);
        body.extend_from_slice(&(seconds as u32).to_be_bytes());
        body.extend_from_slice(&fraction.to_be_bytes());
        body.extend_from_slice(&self.last_timestamp.to_be_bytes());
        body.extend_from_slice(&self.packets.to_be_bytes());
        body.extend_from_slice(&self.octets.to_be_bytes());

        if let Some(packet) = self.seal(&header, body) {
            let _ = self.socket.send(&packet).await;
        }
    }

    /// Asks the sender of `media_ssrc` for a keyframe (a picture loss
    /// indication), for a viewer with nothing it can decode from.
    pub(super) async fn send_picture_loss(&mut self, media_ssrc: u32) {
        let mut header = [0u8; 8];
        // Version 2, format 1 (PLI); type 206; two words after this one.
        header[0] = 0x81;
        header[1] = 206;
        header[2..4].copy_from_slice(&2u16.to_be_bytes());
        header[4..8].copy_from_slice(&self.ssrcs.audio.to_be_bytes());
        if let Some(packet) = self.seal(&header, media_ssrc.to_be_bytes().to_vec()) {
            let _ = self.socket.send(&packet).await;
        }
    }

    /// Asks the sender of `media_ssrc` to resend `lost` (a generic NACK),
    /// which it does on its retransmission stream.
    pub(super) async fn send_nack(&mut self, media_ssrc: u32, lost: &[u16]) {
        let entries = nack_entries(lost);
        if entries.is_empty() {
            return;
        }
        let mut header = [0u8; 8];
        // Version 2, format 1 (generic NACK); type 205.
        header[0] = 0x81;
        header[1] = 205;
        header[2..4].copy_from_slice(&(2 + entries.len() as u16).to_be_bytes());
        header[4..8].copy_from_slice(&self.ssrcs.audio.to_be_bytes());

        let mut body = Vec::with_capacity(4 + entries.len() * 4);
        body.extend_from_slice(&media_ssrc.to_be_bytes());
        for (id, mask) in entries {
            body.extend_from_slice(&id.to_be_bytes());
            body.extend_from_slice(&mask.to_be_bytes());
        }
        if let Some(packet) = self.seal(&header, body) {
            let _ = self.socket.send(&packet).await;
        }
    }

    /// An empty receiver report. A viewer otherwise sends almost nothing, and
    /// this keeps the path to it open through NATs that forget a quiet one.
    pub(super) async fn send_receiver_report(&mut self) {
        let mut header = [0u8; 8];
        // Version 2, no report blocks; type 201; one word after this one.
        header[0] = 0x80;
        header[1] = 201;
        header[2..4].copy_from_slice(&1u16.to_be_bytes());
        header[4..8].copy_from_slice(&self.ssrcs.audio.to_be_bytes());
        if let Some(packet) = self.seal(&header, Vec::new()) {
            let _ = self.socket.send(&packet).await;
        }
    }

    /// Decrypts a received RTP packet. `None` for RTCP, for anything that
    /// fails to decrypt, and for anything too malformed to be RTP.
    pub(super) fn open_rtp(&self, packet: &[u8]) -> Option<RtpPacket> {
        let cipher = self.cipher.as_ref()?;
        if packet.len() < 12 || packet[0] >> 6 != 2 || (200..=206).contains(&packet[1]) {
            return None;
        }

        let csrcs = usize::from(packet[0] & 0x0F);
        let mut clear = 12 + 4 * csrcs;
        let mut extension = 0;
        if packet[0] & 0x10 != 0 {
            let preamble = packet.get(clear..clear + 4)?;
            extension = usize::from(u16::from_be_bytes([preamble[2], preamble[3]])) * 4;
            clear += 4;
        }
        // The tag, at least, has to follow the header, and the counter it.
        if packet.len() < clear + 16 + 4 {
            return None;
        }

        let (aad, rest) = packet.split_at(clear);
        let (body, counter) = rest.split_at(rest.len() - 4);
        let mut body = body.to_vec();
        cipher.open(counter.try_into().ok()?, aad, &mut body).ok()?;

        let mut payload = body.get(extension..)?.to_vec();
        if packet[0] & 0x20 != 0 {
            let padding = usize::from(*payload.last()?);
            payload.truncate(payload.len().checked_sub(padding)?);
        }

        Some(RtpPacket {
            payload_type: packet[1] & 0x7F,
            marker: packet[1] & 0x80 != 0,
            sequence: u16::from_be_bytes([packet[2], packet[3]]),
            timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
            ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
            payload,
        })
    }

    pub(super) async fn recv(&self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.socket.recv(buffer).await
    }

    /// Reads what a received packet asks of the sender. Anything that isn't
    /// RTCP feedback for the video stream is ignored — nobody sends media to
    /// the streamer.
    pub(super) fn feedback(&self, packet: &[u8]) -> Feedback {
        let Some(cipher) = &self.cipher else {
            return Feedback::default();
        };
        // RTCP packet types are 200 to 206; an RTP payload type can't collide
        // with them, marker bit or not, at the ones this connection uses.
        if packet.len() < 8 + 16 + 4 || !(200..=206).contains(&packet[1]) {
            return Feedback::default();
        }

        let (aad, rest) = packet.split_at(8);
        let (body, counter) = rest.split_at(rest.len() - 4);
        let mut body = body.to_vec();
        if cipher
            .open(counter.try_into().unwrap_or_default(), aad, &mut body)
            .is_err()
        {
            return Feedback::default();
        }

        parse_feedback(&[aad, &body].concat(), self.ssrcs.video)
    }
}

/// Splits one encoded frame into RTP payloads, each with whether it carries
/// the marker bit.
///
/// Each NAL unit goes in a packet of its own when it fits, and is split into
/// FU-A fragments when it doesn't (RFC 6184, packetization mode 1). The
/// marker goes on the frame's last packet, which is how a viewer knows the
/// frame is complete.
fn packetize(frame: &[u8]) -> Vec<(Vec<u8>, bool)> {
    let units = nal_units(frame);
    let count = units.len();
    let mut packets = Vec::new();
    for (index, unit) in units.into_iter().enumerate() {
        let last_unit = index + 1 == count;
        if unit.len() <= MAX_PAYLOAD {
            packets.push((unit.to_vec(), last_unit));
            continue;
        }

        let header = unit[0];
        // FU indicator: the unit's F and NRI bits, type 28.
        let indicator = (header & 0xE0) | 28;
        let kind = header & 0x1F;
        let chunks: Vec<&[u8]> = unit[1..].chunks(MAX_PAYLOAD - 2).collect();
        let fragments = chunks.len();
        for (n, chunk) in chunks.into_iter().enumerate() {
            let first = n == 0;
            let last = n + 1 == fragments;
            let fu_header = kind | if first { 0x80 } else { 0 } | if last { 0x40 } else { 0 };
            let mut payload = Vec::with_capacity(chunk.len() + 2);
            payload.push(indicator);
            payload.push(fu_header);
            payload.extend_from_slice(chunk);
            packets.push((payload, last_unit && last));
        }
    }
    packets
}

/// Packs lost sequence numbers into NACK entries: a packet id, and a bitmask
/// of which of the sixteen after it were lost too.
fn nack_entries(lost: &[u16]) -> Vec<(u16, u16)> {
    let mut lost = lost.to_vec();
    // Sorted from the oldest, allowing for the count wrapping past zero.
    if let Some(&first) = lost.first() {
        lost.sort_by_key(|&id| id.wrapping_sub(first) as i16);
    }
    lost.dedup();

    let mut entries: Vec<(u16, u16)> = Vec::new();
    for id in lost {
        if let Some((base, mask)) = entries.last_mut() {
            let offset = id.wrapping_sub(*base);
            if (1..=16).contains(&offset) {
                *mask |= 1 << (offset - 1);
                continue;
            }
        }
        entries.push((id, 0));
    }
    entries
}

/// Reads feedback out of a decrypted compound RTCP packet.
fn parse_feedback(compound: &[u8], video_ssrc: u32) -> Feedback {
    let mut feedback = Feedback::default();
    let mut offset = 0;
    while offset + 12 <= compound.len() {
        let format = compound[offset] & 0x1F;
        let kind = compound[offset + 1];
        let words = u16::from_be_bytes([compound[offset + 2], compound[offset + 3]]);
        let length = (usize::from(words) + 1) * 4;
        let end = (offset + length).min(compound.len());
        // Too short to be feedback (a receiver report with nothing in it,
        // say); every packet is at least a word, so skipping always moves on.
        if end < offset + 12 {
            offset += length;
            continue;
        }
        let media_ssrc = u32::from_be_bytes([
            compound[offset + 8],
            compound[offset + 9],
            compound[offset + 10],
            compound[offset + 11],
        ]);

        match (kind, format) {
            // Picture loss names the stream it's about; a full intra request
            // keeps its SSRC further in and leaves this zero.
            (206, 1) if media_ssrc == video_ssrc => feedback.keyframe = true,
            (206, 4) => feedback.keyframe = true,
            // Generic NACK: a lost packet id, and a bitmask of the sixteen
            // after it that were lost too.
            (205, 1) if media_ssrc == video_ssrc => {
                for entry in compound[offset + 12..end].chunks_exact(4) {
                    let id = u16::from_be_bytes([entry[0], entry[1]]);
                    let mask = u16::from_be_bytes([entry[2], entry[3]]);
                    feedback.lost.push(id);
                    for bit in 0..16 {
                        if mask & (1 << bit) != 0 {
                            feedback.lost.push(id.wrapping_add(bit + 1));
                        }
                    }
                }
            }
            _ => {}
        }
        offset += length;
    }
    feedback
}

/// Discord's IP discovery: send our SSRC, get back the address and port the
/// server saw the packet come from.
async fn discover(socket: &UdpSocket, ssrc: u32) -> Result<SocketAddr, String> {
    let mut request = [0u8; 74];
    request[0..2].copy_from_slice(&1u16.to_be_bytes());
    request[2..4].copy_from_slice(&70u16.to_be_bytes());
    request[4..8].copy_from_slice(&ssrc.to_be_bytes());
    socket
        .send(&request)
        .await
        .map_err(|err| format!("Couldn't reach the stream server: {err}"))?;

    let mut response = [0u8; 74];
    let length = tokio::time::timeout(Duration::from_secs(5), socket.recv(&mut response))
        .await
        .map_err(|_| "The stream server didn't answer".to_string())?
        .map_err(|err| format!("Couldn't reach the stream server: {err}"))?;
    if length < 74 || u16::from_be_bytes([response[0], response[1]]) != 2 {
        return Err("The stream server's answer made no sense".into());
    }

    let address = &response[8..72];
    let end = address
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(address.len());
    let ip = std::str::from_utf8(&address[..end])
        .ok()
        .and_then(|ip| ip.parse().ok())
        .ok_or("The stream server's answer made no sense")?;
    let port = u16::from_be_bytes([response[72], response[73]]);
    Ok(SocketAddr::new(ip, port))
}

/// Enough randomness for RTP's starting points, which only need to be
/// unpredictable, not secret.
fn rand_u32() -> u32 {
    use std::hash::{BuildHasher as _, RandomState};
    RandomState::new().hash_one(SystemTime::now()) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(payloads: &[(Vec<u8>, bool)]) -> Vec<u8> {
        payloads
            .iter()
            .map(|(payload, _)| payload[0] & 0x1F)
            .collect()
    }

    #[test]
    fn small_units_go_one_per_packet_with_the_marker_on_the_last() {
        let frame = [
            &[0, 0, 0, 1, 0x67, 1, 2][..],
            &[0, 0, 0, 1, 0x68, 3][..],
            &[0, 0, 1, 0x65, 4, 5, 6][..],
        ]
        .concat();
        let packets = packetize(&frame);
        assert_eq!(kinds(&packets), [7, 8, 5]);
        assert_eq!(packets[0].0, [0x67, 1, 2]);
        let markers: Vec<bool> = packets.iter().map(|(_, marker)| *marker).collect();
        assert_eq!(markers, [false, false, true]);
    }

    #[test]
    fn a_large_unit_is_split_into_fu_a_fragments() {
        let mut unit = vec![0x65];
        unit.extend((0..3000).map(|n| (n % 251) as u8 + 2));
        let frame = [&[0, 0, 0, 1][..], &unit].concat();

        let packets = packetize(&frame);
        assert_eq!(packets.len(), 3);
        for (n, (payload, marker)) in packets.iter().enumerate() {
            assert!(payload.len() <= MAX_PAYLOAD);
            // Type 28 with the unit's NRI, then start/end bits over its type.
            assert_eq!(payload[0], 0x60 | 28);
            let start = n == 0;
            let end = n == packets.len() - 1;
            assert_eq!(
                payload[1],
                5 | if start { 0x80 } else { 0 } | if end { 0x40 } else { 0 }
            );
            assert_eq!(*marker, end);
        }
        let rebuilt: Vec<u8> = std::iter::once(0x65)
            .chain(
                packets
                    .iter()
                    .flat_map(|(payload, _)| payload[2..].iter().copied()),
            )
            .collect();
        assert_eq!(rebuilt, unit);
    }

    #[test]
    fn a_unit_ending_in_zeros_keeps_them() {
        // DAVE ciphertext can end in zero bytes; only the one zero that makes
        // the next start code four bytes long belongs to it.
        let frame = [&[0, 0, 0, 1, 0x41, 9, 0][..], &[0, 0, 0, 1, 0x41, 7][..]].concat();
        let packets = packetize(&frame);
        assert_eq!(packets[0].0, [0x41, 9, 0]);
        assert_eq!(packets[1].0, [0x41, 7]);
    }

    #[test]
    fn feedback_reads_picture_loss_and_nacks_for_the_video() {
        let video: u32 = 0x1234_5678;
        let mut compound = Vec::new();
        // A receiver report first, which is ignored.
        compound.extend([0x80, 201, 0, 1, 0, 0, 0, 9]);
        // PLI.
        compound.extend([0x81, 206, 0, 2, 0, 0, 0, 9]);
        compound.extend(video.to_be_bytes());
        // NACK for 100, plus 101 and 103 by bitmask.
        compound.extend([0x81, 205, 0, 3, 0, 0, 0, 9]);
        compound.extend(video.to_be_bytes());
        compound.extend([0, 100, 0b0000_0000, 0b0000_0101]);

        let feedback = parse_feedback(&compound, video);
        assert!(feedback.keyframe);
        assert_eq!(feedback.lost, [100, 101, 103]);
    }

    #[test]
    fn nacks_pack_into_ids_and_masks_the_feedback_parser_reads_back() {
        let lost = [100, 103, 101, 130, 65535, 0];
        let entries = nack_entries(&lost);
        assert_eq!(entries, [(65535, 1), (100, 0b101), (130, 0)]);

        let video: u32 = 7;
        let mut compound = vec![0x81, 205, 0, 2 + entries.len() as u8, 0, 0, 0, 9];
        compound.extend(video.to_be_bytes());
        for (id, mask) in &entries {
            compound.extend(id.to_be_bytes());
            compound.extend(mask.to_be_bytes());
        }
        let mut parsed = parse_feedback(&compound, video).lost;
        parsed.sort_unstable();
        let mut expected = lost.to_vec();
        expected.sort_unstable();
        assert_eq!(parsed, expected);
    }

    #[test]
    fn feedback_for_another_stream_or_malformed_is_ignored() {
        let mut compound = vec![0x81, 206, 0, 2, 0, 0, 0, 9];
        compound.extend(42u32.to_be_bytes());
        // A length that claims less than the header it's in.
        compound.extend([0x81, 205, 0, 0, 0, 0, 0, 9, 0, 0, 0, 1]);
        let feedback = parse_feedback(&compound, 7);
        assert!(!feedback.keyframe);
        assert!(feedback.lost.is_empty());
    }
}
