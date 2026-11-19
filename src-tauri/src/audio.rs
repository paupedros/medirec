use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const TARGET_RATE: u32 = 16_000;

/// Streaming linear resampler to 16 kHz: never holds audio at the source rate.
pub struct Resampler {
    step: f64, // input samples per output sample
    next: f64, // input index of the next output sample
    i: f64,    // index of the incoming input sample
    prev: f32,
    pub out: Vec<f32>,
}

impl Resampler {
    pub fn new(from: u32) -> Self {
        Self { step: from as f64 / TARGET_RATE as f64, next: 0.0, i: 0.0, prev: 0.0, out: Vec::new() }
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        while self.next <= self.i {
            let frac = (self.next - (self.i - 1.0)) as f32;
            self.out.push(self.prev + (x - self.prev) * frac);
            self.next += self.step;
        }
        self.prev = x;
        self.i += 1.0;
    }
}

/// Decode any supported audio file to 16 kHz mono f32.
pub fn decode_file(path: &Path) -> Result<Vec<f32>, String> {
    let file = File::open(path).map_err(|e| format!("No se pudo abrir el archivo: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|_| "Este tipo de archivo de audio no se puede leer.".to_string())?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("El archivo no contiene audio.")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.ok_or("Audio sin frecuencia.")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|_| "Formato de audio no compatible.".to_string())?;

    let mut rs = Resampler::new(rate);
    let mut buf: Option<SampleBuffer<f32>> = None;
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let Ok(decoded) = decoder.decode(&packet) else { continue };
        let spec = *decoded.spec();
        let ch = spec.channels.count().max(1);
        let cap = decoded.capacity() as u64;
        if buf.as_ref().map_or(true, |b| b.capacity() < cap as usize * ch) {
            buf = Some(SampleBuffer::new(cap, spec));
        }
        let b = buf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        for frame in b.samples().chunks(ch) {
            rs.push(frame.iter().sum::<f32>() / ch as f32);
        }
    }
    if rs.out.is_empty() {
        return Err("No se encontró audio en el archivo.".into());
    }
    Ok(rs.out)
}

/// (all input device names, default device name)
pub fn list_inputs() -> (Vec<String>, Option<String>) {
    let host = cpal::default_host();
    let names = host
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    let default = host.default_input_device().and_then(|d| d.name().ok());
    (names, default)
}

/// 16 kHz mono 16-bit WAV bytes for playback in the UI.
pub fn to_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&TARGET_RATE.to_le_bytes());
    w.extend_from_slice(&(TARGET_RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    w
}

/// Downmix interleaved frames to mono, resample, update the level meter. No allocation
/// beyond the output Vec growing.
fn feed(
    cap: &Mutex<Resampler>,
    level: &AtomicU32,
    paused: &AtomicBool,
    ch: usize,
    data: impl Iterator<Item = f32>,
) {
    if paused.load(Ordering::Relaxed) {
        level.store(0, Ordering::Relaxed);
        return; // drop input while paused
    }
    let mut rs = cap.lock().unwrap();
    let (mut sum, mut n, mut peak) = (0f32, 0usize, 0f32);
    for s in data {
        sum += s;
        n += 1;
        if n == ch {
            let m = sum / ch as f32;
            peak = peak.max(m.abs());
            rs.push(m);
            sum = 0.0;
            n = 0;
        }
    }
    level.store((peak.min(1.0) * 1000.0) as u32, Ordering::Relaxed);
}

/// Microphone recorder running on its own thread (cpal streams are not Send).
/// Dropping it (or calling `finish`) stops the stream.
pub struct Recorder {
    stop: Option<mpsc::Sender<()>>,
    cap: Arc<Mutex<Resampler>>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub level: Arc<AtomicU32>,
    pub paused: Arc<AtomicBool>,
}

impl Recorder {
    /// Seconds actually recorded (pauses excluded).
    pub fn seconds(&self) -> f32 {
        self.cap.lock().unwrap().out.len() as f32 / TARGET_RATE as f32
    }

    pub fn start(device: Option<String>) -> Result<Self, String> {
        let level = Arc::new(AtomicU32::new(0));
        let paused = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<Arc<Mutex<Resampler>>, String>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let (lvl, pz) = (level.clone(), paused.clone());
        let handle = std::thread::spawn(move || {
            let setup = || -> Result<(cpal::Stream, Arc<Mutex<Resampler>>), String> {
                let host = cpal::default_host();
                let chosen = device.as_ref().and_then(|n| {
                    host.input_devices()
                        .ok()?
                        .find(|d| d.name().map(|x| &x == n).unwrap_or(false))
                });
                let dev = chosen
                    .or_else(|| host.default_input_device())
                    .ok_or("No se encontró ningún micrófono.")?;
                let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
                let ch = cfg.channels() as usize;
                let fmt = cfg.sample_format();
                let cap = Arc::new(Mutex::new(Resampler::new(cfg.sample_rate().0)));
                let cfg: cpal::StreamConfig = cfg.into();
                let (c, l, p) = (cap.clone(), lvl.clone(), pz.clone());
                let err = |e| eprintln!("audio error: {e}");
                let stream = match fmt {
                    cpal::SampleFormat::F32 => dev.build_input_stream(
                        &cfg,
                        move |d: &[f32], _: &_| feed(&c, &l, &p, ch, d.iter().copied()),
                        err,
                        None,
                    ),
                    cpal::SampleFormat::I16 => dev.build_input_stream(
                        &cfg,
                        move |d: &[i16], _: &_| {
                            feed(&c, &l, &p, ch, d.iter().map(|s| *s as f32 / 32768.0))
                        },
                        err,
                        None,
                    ),
                    cpal::SampleFormat::U16 => dev.build_input_stream(
                        &cfg,
                        move |d: &[u16], _: &_| {
                            feed(&c, &l, &p, ch, d.iter().map(|s| (*s as f32 - 32768.0) / 32768.0))
                        },
                        err,
                        None,
                    ),
                    _ => return Err("Formato de micrófono no compatible.".into()),
                }
                .map_err(|e| format!("No se pudo usar el micrófono: {e}"))?;
                stream.play().map_err(|e| e.to_string())?;
                Ok((stream, cap))
            };
            match setup() {
                Ok((stream, cap)) => {
                    let _ = ready_tx.send(Ok(cap));
                    let _ = stop_rx.recv(); // blocks until stop or Recorder dropped
                    drop(stream);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        let cap = ready_rx.recv().map_err(|e| e.to_string())??;
        Ok(Self { stop: Some(stop_tx), cap, handle: Some(handle), level, paused })
    }

    /// Stops and returns 16 kHz mono samples.
    pub fn finish(mut self) -> Vec<f32> {
        self.stop.take(); // dropping the sender wakes the thread
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        std::mem::take(&mut self.cap.lock().unwrap().out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_48k_to_16k_keeps_length_and_values() {
        let mut rs = Resampler::new(48_000);
        for i in 0..48_000 {
            rs.push(i as f32);
        }
        assert_eq!(rs.out.len(), 16_000);
        // A ramp must stay a ramp: output k sits at input 3k.
        assert_eq!(rs.out[0], 0.0);
        assert_eq!(rs.out[1], 3.0);
        assert_eq!(rs.out[15_999], 47_997.0);
    }

    #[test]
    fn resampler_44k1_interpolates_between_samples() {
        let mut rs = Resampler::new(44_100);
        for i in 0..44_100 {
            rs.push(i as f32);
        }
        assert!((rs.out.len() as i64 - 16_000).abs() <= 1);
        let k = 1234;
        assert!((rs.out[k] - k as f32 * 44_100.0 / 16_000.0).abs() < 1e-2);
    }
}
