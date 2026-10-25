//! Bundled Discord recordings, decoded once on the cue worker, never the callback.

use std::io::{Cursor, ErrorKind};
use std::sync::OnceLock;

use anyhow::{Context as _, ensure};
use symphonia::core::audio::{Channels, SampleBuffer};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;

use super::Cue;

const JOIN: &[u8] = include_bytes!("../../../assets/sounds/user_join.mp3");
const LEAVE: &[u8] = include_bytes!("../../../assets/sounds/user_leave.mp3");
const DISCONNECT: &[u8] = include_bytes!("../../../assets/sounds/disconnect.mp3");

pub(crate) struct Clip {
    pub(super) samples: Box<[f32]>,
    pub(super) sample_rate: u32,
}

impl Clip {
    pub(super) fn frames(&self) -> usize {
        self.samples.len() / 2
    }
}

pub(super) fn get(cue: Cue) -> Option<&'static Clip> {
    static JOIN_CLIP: OnceLock<Option<Clip>> = OnceLock::new();
    static LEAVE_CLIP: OnceLock<Option<Clip>> = OnceLock::new();
    static DISCONNECT_CLIP: OnceLock<Option<Clip>> = OnceLock::new();
    let (cache, bytes, asset) = match cue {
        Cue::Join | Cue::PeerJoin => (&JOIN_CLIP, JOIN, "user_join.mp3"),
        Cue::Leave => (&DISCONNECT_CLIP, DISCONNECT, "disconnect.mp3"),
        Cue::PeerLeave => (&LEAVE_CLIP, LEAVE, "user_leave.mp3"),
    };
    cache
        .get_or_init(|| match decode(bytes) {
            Ok(clip) => Some(clip),
            Err(error) => {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "voice_cue_decode_failed",
                        "cue": format!("{cue:?}"),
                        "asset": asset,
                        "error": error.to_string(),
                    })
                );
                None
            }
        })
        .as_ref()
}

fn decode(bytes: &'static [u8]) -> anyhow::Result<Clip> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let options = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let mut format = symphonia::default::get_probe()
        .format(&hint, source, &options, &Default::default())?
        .format;
    let track = format.default_track().context("cue has no audio track")?;
    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &Default::default())?;
    let mut samples = Vec::new();
    let mut sample_rate = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        ensure!(
            spec.rate > 0 && spec.channels == Channels::FRONT_LEFT | Channels::FRONT_RIGHT,
            "cue must have a valid stereo layout"
        );
        ensure!(
            sample_rate.is_none_or(|rate| rate == spec.rate),
            "cue sample rate changed"
        );
        sample_rate = Some(spec.rate);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        ensure!(
            buffer.samples().iter().all(|sample| sample.is_finite()),
            "cue contains non-finite PCM"
        );
        samples.extend_from_slice(buffer.samples());
    }
    ensure!(!samples.is_empty(), "cue contains no PCM");
    Ok(Clip {
        samples: samples.into_boxed_slice(),
        sample_rate: sample_rate.context("cue has no sample rate")?,
    })
}

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    use super::*;

    #[test]
    fn bundled_originals_match_provenance_and_decode_finite_gapless_stereo() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../assets/sounds/provenance.json")).unwrap();
        for (bytes, asset) in [JOIN, LEAVE, DISCONNECT]
            .into_iter()
            .zip(manifest["assets"].as_array().unwrap())
        {
            assert_eq!(bytes.len() as u64, asset["bytes"].as_u64().unwrap());
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), asset["sha256"]);
            let clip = decode(bytes).unwrap();
            assert_eq!(clip.sample_rate, 44_100);
            assert_eq!(clip.samples.len() % 2, 0);
            assert_eq!(
                clip.frames() as u64,
                asset["decodedFrames"].as_u64().unwrap()
            );
            assert!(clip.samples.iter().all(|sample| sample.is_finite()));
            assert!(clip.samples.iter().any(|sample| sample.abs() > 0.01));
        }
    }

    #[test]
    fn decode_failure_has_no_synthesized_fallback_and_success_is_cached() {
        assert!(decode(b"not an MP3 recording").is_err());
        assert!(decode(&JOIN[..100]).is_err());
        let join = get(Cue::Join).unwrap();
        assert!(std::ptr::eq(join, get(Cue::Join).unwrap()));
        assert!(std::ptr::eq(join, get(Cue::PeerJoin).unwrap()));
        assert!(!std::ptr::eq(join, get(Cue::PeerLeave).unwrap()));
        assert!(!std::ptr::eq(
            get(Cue::Leave).unwrap(),
            get(Cue::PeerLeave).unwrap()
        ));
    }

    #[test]
    fn selected_pcm_matches_the_original_recording_for_each_event() {
        for (cue, original) in [
            (Cue::Join, JOIN),
            (Cue::PeerJoin, JOIN),
            (Cue::Leave, DISCONNECT),
            (Cue::PeerLeave, LEAVE),
        ] {
            let expected = decode(original).unwrap();
            let selected = get(cue).unwrap();
            assert_eq!(selected.sample_rate, expected.sample_rate);
            assert_eq!(selected.samples, expected.samples);
        }
    }
}
