//! Audio monitor capture and FFT band analysis.
//!
//! Uses the PulseAudio API (provided by PipeWire on Pop!_OS) to read from the
//! default sink monitor (`@DEFAULT_MONITOR@`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use libpulse_binding as pulse;
use libpulse_simple_binding as pulse_simple;
use rustfft::{num_complex::Complex, FftPlanner};
use tracing::{info, warn};

const SAMPLE_RATE: u32 = 44_100;
/// Length of *real* audio (Hann-weighted) fed into each analysis. This sets the
/// time context the spectrum "sees": 1024 frames ≈ 23 ms at 44.1 kHz, half the
/// previous 2048/≈46 ms. A shorter window localises transients in time better
/// (less smear → onsets and decays read as more immediate) at the cost of
/// coarser *true* frequency resolution (≈43 Hz vs ≈22 Hz). To avoid that
/// coarseness collapsing the lowest log bands into empty/duplicated bins, the
/// window is zero-padded up to `FFT_SIZE` below, which interpolates the spectrum
/// back to ≈22 Hz bin spacing so every one of the 16 log bands still lands on a
/// distinct bin (binning identical to the old 2048-pt setup).
const WINDOW_SAMPLES: usize = 1024;
/// FFT length. `WINDOW_SAMPLES` real samples are zero-padded up to this size, so
/// the bin spacing stays ≈22 Hz (44100/2048) — preserving low-band granularity
/// — while the analysis only carries ≈23 ms of real context.
const FFT_SIZE: usize = 2048;
/// Hop size: how many fresh stereo frames we pull — and slide the analysis
/// window by — each iteration. A smaller hop means the bands are recomputed
/// more often, which is the main lever for responsiveness. 256 frames ≈ 5.8 ms
/// at 44.1 kHz, giving ~172 band updates/sec (was 512/≈11.6 ms/~86 Hz). The
/// 1024-pt window slides over an overlapping ring buffer so resolution is
/// unaffected by the smaller hop. A 1024-pt FFT @172/s is a few hundred µs of
/// CPU — negligible.
const HOP_SAMPLES: usize = 256;
pub const SPECTRUM_BANDS: usize = 16;
const MIN_BAND_HZ: f32 = 40.0;
const MAX_BAND_HZ: f32 = 16_000.0;

/// Asymmetric EMA coefficients, applied once per hop (≈5.8 ms). Attack is
/// effectively instantaneous (1.0): a rising band jumps straight to the freshly
/// measured value so onsets have *zero* smoothing latency. The release is kept
/// gentle (τ ≈ 55 ms) so the decay stays smooth and doesn't pop or flicker;
/// because the hop halved, the release alpha is lowered from 0.18 to 0.10 to
/// hold roughly the same real-time decay constant as before.
const SMOOTH_ATTACK: f32 = 1.0;
const SMOOTH_RELEASE: f32 = 0.10;

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioLevels {
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub energy: f32,
    pub bands: [f32; SPECTRUM_BANDS],
}

#[derive(Clone)]
pub struct AudioAnalyzer {
    levels: Arc<Mutex<AudioLevels>>,
    running: Arc<AtomicBool>,
}

impl AudioAnalyzer {
    pub fn start(sensitivity: f32) -> Result<(Self, JoinHandle<()>)> {
        let levels = Arc::new(Mutex::new(AudioLevels::default()));
        let running = Arc::new(AtomicBool::new(true));
        let levels_thread = Arc::clone(&levels);
        let running_thread = Arc::clone(&running);

        let handle = thread::Builder::new()
            .name("audio-analyzer".into())
            .spawn(move || {
                if let Err(err) = capture_loop(levels_thread, running_thread, sensitivity) {
                    warn!(?err, "audio capture loop exited");
                }
            })
            .context("failed to spawn audio thread")?;

        Ok((
            Self {
                levels,
                running,
            },
            handle,
        ))
    }

    pub fn levels(&self) -> AudioLevels {
        *self.levels.lock().unwrap()
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for AudioAnalyzer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn capture_loop(
    levels: Arc<Mutex<AudioLevels>>,
    running: Arc<AtomicBool>,
    sensitivity: f32,
) -> Result<()> {
    let spec = pulse::sample::Spec {
        format: pulse::sample::Format::F32le,
        channels: 2,
        rate: SAMPLE_RATE,
    };

    if !spec.is_valid() {
        anyhow::bail!("invalid pulse audio spec");
    }

    // Request a small server-side fragment so the monitor source hands us audio
    // with minimal buffering. `fragsize` is the per-read chunk in bytes; sizing
    // it to exactly one hop keeps capture latency to ~one hop (≈5.8 ms) rather
    // than letting PulseAudio/PipeWire pick a large default fragment. The other
    // fields are left at u32::MAX ("server decides"); `maxlength` is capped to a
    // few hops so a stalled reader can't accumulate a long backlog of stale
    // audio that would then play back as lag.
    let fragsize = (HOP_SAMPLES * spec.channels as usize * std::mem::size_of::<f32>()) as u32;
    let buffer_attr = pulse::def::BufferAttr {
        maxlength: fragsize.saturating_mul(4),
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize,
    };

    let simple = pulse_simple::Simple::new(
        None,
        "cosmic-audio-bg",
        pulse::stream::Direction::Record,
        Some("@DEFAULT_MONITOR@"),
        "monitor",
        &spec,
        None,
        Some(&buffer_attr),
    )
    .context("failed to open @DEFAULT_MONITOR@ — is PipeWire/Pulse running?")?;

    info!("audio capture started from @DEFAULT_MONITOR@");

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    // FFT scratch is FFT_SIZE wide; the upper (FFT_SIZE - WINDOW_SAMPLES)
    // samples are the zero-padding and are reset every iteration (the in-place
    // FFT overwrites them with garbage), so they stay exactly zero.
    let mut buffer = vec![Complex::<f32>::new(0.0, 0.0); FFT_SIZE];
    let window = hann_window(WINDOW_SAMPLES);
    let mut smoothed = AudioLevels::default();
    let mut sample_buf = vec![0u8; HOP_SAMPLES * 2 * std::mem::size_of::<f32>()];
    // Rolling mono window holding the most recent WINDOW_SAMPLES samples. Each
    // read slides it left by one hop and appends the freshly captured frames, so
    // the analysis keeps full WINDOW_SAMPLES context while the bands refresh
    // every hop (overlapping windows) instead of only once per full block.
    let mut mono_window = vec![0.0f32; WINDOW_SAMPLES];

    while running.load(Ordering::SeqCst) {
        match simple.read(&mut sample_buf) {
            Ok(()) => {}
            Err(err) => {
                warn!(?err, "pulse read failed, retrying");
                thread::sleep(Duration::from_millis(50));
                continue;
            }
        }

        let samples: &[f32] = bytemuck::cast_slice(&sample_buf);

        // Slide the window left by one hop, then mix the new stereo frames to
        // mono and append them at the tail.
        mono_window.copy_within(HOP_SAMPLES.., 0);
        let tail = WINDOW_SAMPLES - HOP_SAMPLES;
        for f in 0..HOP_SAMPLES {
            let l = samples[f * 2];
            let r = samples[f * 2 + 1];
            mono_window[tail + f] = (l + r) * 0.5;
        }

        // Apply the Hann window over the real samples, then zero-pad the rest of
        // the FFT buffer. Zero-padding interpolates the spectrum up to FFT_SIZE
        // bins so the low log bands keep distinct bins despite the shorter
        // (1024-sample) real window.
        for i in 0..WINDOW_SAMPLES {
            buffer[i].re = mono_window[i] * window[i];
            buffer[i].im = 0.0;
        }
        for slot in buffer.iter_mut().take(FFT_SIZE).skip(WINDOW_SAMPLES) {
            slot.re = 0.0;
            slot.im = 0.0;
        }

        fft.process(&mut buffer);

        let bin_hz = SAMPLE_RATE as f32 / FFT_SIZE as f32;
        let mut bass = 0.0f32;
        let mut mid = 0.0f32;
        let mut high = 0.0f32;
        let mut energy = 0.0f32;

        for (i, bin) in buffer.iter().enumerate().take(FFT_SIZE / 2) {
            let freq = i as f32 * bin_hz;
            // Normalise by the *real* window length (not the padded FFT length):
            // the Hann sum scales with the number of real samples, so dividing
            // by WINDOW_SAMPLES keeps magnitudes — and thus the sensitivity
            // calibration — identical to the previous full-window setup.
            let magnitude = bin.norm() / WINDOW_SAMPLES as f32;
            energy += magnitude;

            if freq < 250.0 {
                bass += magnitude;
            } else if freq < 4000.0 {
                mid += magnitude;
            } else if freq < 16_000.0 {
                high += magnitude;
            }
        }

        let mut bands = compute_spectrum_bands(&buffer, bin_hz);
        let scale = sensitivity * 12.0;
        // Perceptual compression: lift quiet-but-present frequencies into a
        // visible range so each band clearly tracks the live spectrum instead
        // of staying near zero until the audio is very loud.
        for band in &mut bands {
            *band = (*band * scale).max(0.0).powf(0.6).clamp(0.0, 1.0);
        }
        bass = (bass * scale).max(0.0).powf(0.6).clamp(0.0, 1.0);
        mid = (mid * scale).max(0.0).powf(0.6).clamp(0.0, 1.0);
        high = (high * scale).max(0.0).powf(0.6).clamp(0.0, 1.0);
        energy = (energy * scale * 0.25).clamp(0.0, 1.0);

        smoothed = smooth_levels(
            smoothed,
            AudioLevels {
                bass,
                mid,
                high,
                energy,
                bands,
            },
            SMOOTH_ATTACK,
            SMOOTH_RELEASE,
        );
        *levels.lock().unwrap() = smoothed;
    }

    Ok(())
}

/// Asymmetric exponential smoothing: rising values use `attack` (fast, so
/// onsets show up immediately) and falling values use `release` (slower, so the
/// decay stays smooth and doesn't pop). Pass equal values for plain symmetric
/// smoothing.
fn smooth_levels(
    current: AudioLevels,
    target: AudioLevels,
    attack: f32,
    release: f32,
) -> AudioLevels {
    let ema = |c: f32, t: f32| {
        let alpha = if t > c { attack } else { release };
        c + (t - c) * alpha
    };
    let mut bands = [0.0; SPECTRUM_BANDS];
    for i in 0..SPECTRUM_BANDS {
        bands[i] = ema(current.bands[i], target.bands[i]);
    }
    AudioLevels {
        bass: ema(current.bass, target.bass),
        mid: ema(current.mid, target.mid),
        high: ema(current.high, target.high),
        energy: ema(current.energy, target.energy),
        bands,
    }
}

fn compute_spectrum_bands(buffer: &[Complex<f32>], bin_hz: f32) -> [f32; SPECTRUM_BANDS] {
    let mut bands = [0.0f32; SPECTRUM_BANDS];
    let ratio = MAX_BAND_HZ / MIN_BAND_HZ;

    for b in 0..SPECTRUM_BANDS {
        let t0 = b as f32 / SPECTRUM_BANDS as f32;
        let t1 = (b + 1) as f32 / SPECTRUM_BANDS as f32;
        let f_low = MIN_BAND_HZ * ratio.powf(t0);
        let f_high = MIN_BAND_HZ * ratio.powf(t1);

        let i_low = (f_low / bin_hz).ceil() as usize;
        let i_high = ((f_high / bin_hz).ceil() as usize).min(FFT_SIZE / 2);
        if i_low >= i_high {
            continue;
        }

        let mut sum = 0.0f32;
        for i in i_low..i_high {
            sum += buffer[i].norm() / WINDOW_SAMPLES as f32;
        }
        bands[b] = sum / (i_high - i_low) as f32;
    }

    bands
}

fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| {
            let x = i as f32 / (size - 1) as f32;
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * x).cos())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooth_levels_moves_toward_target() {
        let current = AudioLevels::default();
        let target = AudioLevels {
            bass: 1.0,
            mid: 0.5,
            high: 0.25,
            energy: 0.8,
            bands: [0.5; SPECTRUM_BANDS],
        };
        // Equal attack/release == plain symmetric EMA.
        let next = smooth_levels(current, target, 0.5, 0.5);
        assert!((next.bass - 0.5).abs() < f32::EPSILON);
        assert!((next.mid - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn smooth_levels_attacks_faster_than_it_releases() {
        let zero = AudioLevels::default();
        let high = AudioLevels {
            bass: 1.0,
            ..AudioLevels::default()
        };
        // Rising from 0 -> 1 uses the (faster) attack coefficient.
        let rising = smooth_levels(zero, high, 0.6, 0.18);
        assert!((rising.bass - 0.6).abs() < 1e-6);
        // Falling from 1 -> 0 uses the (slower) release coefficient.
        let falling = smooth_levels(high, zero, 0.6, 0.18);
        assert!((falling.bass - 0.82).abs() < 1e-6);
        // Attack must move more per step than release.
        assert!(rising.bass > 1.0 - falling.bass);
    }
}
