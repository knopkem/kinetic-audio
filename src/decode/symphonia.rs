//! Multi-format audio decoder via `symphonia`.
//!
//! Supports OGG Vorbis, MP3, FLAC, and any other format that symphonia
//! can probe from a byte slice.

use crate::decode::DecodeError;
use crate::math::Frame;

/// Decode any symphonia-supported format from a byte slice.
///
/// Returns `(interleaved stereo f32 frames, sample_rate)`.
pub fn decode_symphonia(bytes: &[u8]) -> Result<(Vec<Frame>, u32), DecodeError> {
    use symphonia::core::audio::AudioBufferRef;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| DecodeError::Other(e.to_string()))?;

    let mut format = probed.format;

    // Select the first audio track.
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| DecodeError::Unsupported("no supported audio track".into()))?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| DecodeError::Unsupported("unknown sample rate".into()))?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| DecodeError::Other(e.to_string()))?;

    let mut frames: Vec<Frame> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(DecodeError::Other(e.to_string())),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder
            .decode(&packet)
            .map_err(|e| DecodeError::Other(e.to_string()))?;

        // Convert any sample format to f32 stereo Frames.
        match decoded {
            AudioBufferRef::F32(buf) => {
                append_f32_buffer(&buf, &mut frames);
            }
            AudioBufferRef::F64(buf) => {
                append_generic(&buf, &mut frames, |s| s as f32);
            }
            AudioBufferRef::S8(buf) => {
                append_generic(&buf, &mut frames, |s| s as f32 / i8::MAX as f32);
            }
            AudioBufferRef::U8(buf) => {
                append_generic(&buf, &mut frames, |s| (s as f32 - 128.0) / 128.0);
            }
            AudioBufferRef::S16(buf) => {
                append_generic(&buf, &mut frames, |s| s as f32 / i16::MAX as f32);
            }
            AudioBufferRef::U16(buf) => {
                append_generic(&buf, &mut frames, |s| (s as f32 - 32768.0) / 32768.0);
            }
            AudioBufferRef::S24(buf) => {
                append_generic(&buf, &mut frames, |s| s.0 as f32 / 8_388_608.0);
            }
            AudioBufferRef::U24(buf) => {
                append_generic(&buf, &mut frames, |s| {
                    (s.0 as f32 - 8_388_608.0) / 8_388_608.0
                });
            }
            AudioBufferRef::S32(buf) => {
                append_generic(&buf, &mut frames, |s| s as f32 / i32::MAX as f32);
            }
            AudioBufferRef::U32(buf) => {
                append_generic(&buf, &mut frames, |s| {
                    (s as f64 - i32::MAX as f64) as f32 / i32::MAX as f32
                });
            }
        }
    }

    Ok((frames, sample_rate))
}

fn append_f32_buffer(buf: &symphonia::core::audio::AudioBuffer<f32>, frames: &mut Vec<Frame>) {
    use symphonia::core::audio::Signal;
    let n = buf.frames();
    let planes = buf.planes();
    let plane_data = planes.planes();
    match plane_data.len() {
        0 => {}
        1 => {
            for &s in &plane_data[0][..n] {
                frames.push(Frame::mono(s));
            }
        }
        _ => {
            let l = &plane_data[0][..n];
            let r = &plane_data[1][..n];
            for i in 0..n {
                frames.push(Frame { l: l[i], r: r[i] });
            }
        }
    }
}

fn append_generic<S, F>(
    buf: &symphonia::core::audio::AudioBuffer<S>,
    frames: &mut Vec<Frame>,
    convert: F,
) where
    S: symphonia::core::sample::Sample + Copy,
    F: Fn(S) -> f32,
{
    use symphonia::core::audio::Signal;
    let n = buf.frames();
    let planes = buf.planes();
    let plane_data = planes.planes();
    match plane_data.len() {
        0 => {}
        1 => {
            for &s in &plane_data[0][..n] {
                frames.push(Frame::mono(convert(s)));
            }
        }
        _ => {
            let l = &plane_data[0][..n];
            let r = &plane_data[1][..n];
            for i in 0..n {
                frames.push(Frame {
                    l: convert(l[i]),
                    r: convert(r[i]),
                });
            }
        }
    }
}
