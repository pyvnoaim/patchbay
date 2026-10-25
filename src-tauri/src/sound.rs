//! A remote desktop's sound, played by the webview. IronRDP's audio channel hands over
//! PCM and this passes it to the window, where WebAudio plays it: no audio library here,
//! because `cpal` would bring ALSA's headers into the Linux build and patchbay already
//! ships an engine that plays sound.
//!
//! Only 16-bit PCM is accepted. Windows always offers it, and it is the one format the
//! window can play without a decoder; the cost is bandwidth, about 170 KB/s for CD
//! stereo, which a LAN or a bastion carries without noticing. Volume is not advertised,
//! so the server scales the samples itself and the window plays them as they come.
//! Registered on both transports: Windows 8 and later use the dynamic channel, xrdp and
//! older Windows the static `rdpsnd`.
//!
//! ponytail: PCM only; IronRDP's pure-Rust ADPCM decoder if a slow link wants a smaller
//! stream.

use ironrdp_rdpsnd::client::RdpsndClientHandler;
use ironrdp_rdpsnd::pdu::{AudioFormat, PitchPdu, VolumePdu, WaveFormat};
use std::borrow::Cow;
use std::time::Duration;

/// How far ahead of the clock the window schedules what arrives, which the server is told
/// is the playback delay so it can keep the picture and the sound together.
const LEAD: Duration = Duration::from_millis(100);

/// Where the session's sound goes: an 8-byte header (the sample rate as a little-endian
/// u32, the channel count as a u16, two bytes of nothing), then interleaved s16le samples.
#[derive(Clone)]
pub struct Speaker {
    out: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>,
    /// What the client advertises before the server says what it has; replaced by the
    /// server's own PCM formats in `choose_formats`.
    formats: Vec<AudioFormat>,
}

impl std::fmt::Debug for Speaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Speaker")
    }
}

impl Speaker {
    pub fn new(out: tauri::ipc::Channel<tauri::ipc::InvokeResponseBody>) -> Self {
        Self {
            out,
            formats: vec![pcm(2, 44100)],
        }
    }
}

fn pcm(n_channels: u16, rate: u32) -> AudioFormat {
    AudioFormat {
        format: WaveFormat::PCM,
        n_channels,
        n_samples_per_sec: rate,
        n_avg_bytes_per_sec: rate * u32::from(n_channels) * 2,
        n_block_align: n_channels * 2,
        bits_per_sample: 16,
        data: None,
    }
}

/// The server's formats the window can play: 16-bit PCM, mono or stereo, any rate
/// WebAudio takes (it resamples).
fn playable(server: &[AudioFormat]) -> Vec<AudioFormat> {
    server
        .iter()
        .filter(|f| {
            f.format == WaveFormat::PCM
                && f.bits_per_sample == 16
                && (1..=2).contains(&f.n_channels)
                && (8000..=96000).contains(&f.n_samples_per_sec)
        })
        .cloned()
        .collect()
}

/// One wave block as the window reads it.
fn block(format: &AudioFormat, data: &[u8]) -> Vec<u8> {
    // Whole frames only: a torn last sample would read as a click.
    let frame = usize::from(format.n_channels) * 2;
    let data = &data[..data.len() - data.len() % frame];
    let mut out = Vec::with_capacity(8 + data.len());
    out.extend_from_slice(&format.n_samples_per_sec.to_le_bytes());
    out.extend_from_slice(&format.n_channels.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(data);
    out
}

impl RdpsndClientHandler for Speaker {
    fn get_formats(&self) -> &[AudioFormat] {
        &self.formats
    }

    fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
        playable(server_formats)
    }

    fn wave(&mut self, format: &AudioFormat, _ts: u32, data: Cow<'_, [u8]>) {
        if format.format != WaveFormat::PCM || format.bits_per_sample != 16 || data.is_empty() {
            return;
        }
        let _ = self
            .out
            .send(tauri::ipc::InvokeResponseBody::Raw(block(format, &data)));
    }

    fn playback_latency(&self) -> Duration {
        LEAD
    }

    fn set_volume(&mut self, _: VolumePdu) {}

    fn set_pitch(&mut self, _: PitchPdu) {}

    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_16_bit_pcm_is_taken_and_a_block_is_whole_frames() {
        let mut eight = pcm(2, 22050);
        eight.bits_per_sample = 8;
        let mut adpcm = pcm(2, 44100);
        adpcm.format = WaveFormat::ADPCM;
        let picked = playable(&[adpcm, pcm(2, 44100), eight, pcm(1, 22050), pcm(6, 48000)]);
        let rates: Vec<_> = picked
            .iter()
            .map(|f| (f.n_channels, f.n_samples_per_sec))
            .collect();
        assert_eq!(rates, [(2, 44100), (1, 22050)]);

        // Seven bytes of stereo is one frame and three left over.
        let b = block(&pcm(2, 48000), &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(b[..8], [0x80, 0xbb, 0, 0, 2, 0, 0, 0]);
        assert_eq!(b[8..], [1, 2, 3, 4]);
    }
}
