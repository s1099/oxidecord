//! Device plumbing shared by the call's audio ([`crate::voice`]) and a video's
//! ([`crate::platform::video`]). The two keep their own streams and buffering;
//! only the conversions to whatever the device wants live here.

/// Keeps a sample inside the range the device conversions assume.
///
/// A NaN compares false against every bound, so it has to be caught before the
/// clamp rather than by it.
pub fn sanitize(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1., 1.)
    } else {
        0.
    }
}

/// Opens a stream in whichever sample format the device reports.
///
/// `build` names a function generic over the sample type, and the arguments
/// after it are handed to it unchanged. The formats left out are the ones no
/// host offers as a device default.
macro_rules! in_device_format {
    ($format:expr, $build:ident $(, $arg:expr)* $(,)?) => {
        match $format {
            ::cpal::SampleFormat::F32 => $build::<f32>($($arg),*),
            ::cpal::SampleFormat::F64 => $build::<f64>($($arg),*),
            ::cpal::SampleFormat::I8 => $build::<i8>($($arg),*),
            ::cpal::SampleFormat::I16 => $build::<i16>($($arg),*),
            ::cpal::SampleFormat::I32 => $build::<i32>($($arg),*),
            ::cpal::SampleFormat::U8 => $build::<u8>($($arg),*),
            ::cpal::SampleFormat::U16 => $build::<u16>($($arg),*),
            ::cpal::SampleFormat::U32 => $build::<u32>($($arg),*),
            _ => None,
        }
    };
}
pub(crate) use in_device_format;
