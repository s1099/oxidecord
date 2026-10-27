//! Device helpers shared by the call's audio and video playback, which keep
//! their own streams and buffering.

use cpal::DeviceId;
use cpal::traits::HostTrait as _;

/// Falls back to the system default when `id` is unset or the device was unplugged.
pub fn chosen_output(host: &cpal::Host, id: Option<&str>) -> Option<cpal::Device> {
    id.and_then(|id| id.parse::<DeviceId>().ok())
        .and_then(|id| host.device_by_id(&id))
        .or_else(|| host.default_output_device())
}

/// NaN compares false against every bound, so it must be caught before the clamp.
pub fn sanitize(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1., 1.)
    } else {
        0.
    }
}

/// Calls `$build::<T>` for the device's sample format. Formats left out are ones
/// no host offers as a default.
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
