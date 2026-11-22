use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::{RECV_SAMPLE_RATE, SEND_SAMPLE_RATE};

pub enum PlaybackCmd {
    Audio(Vec<u8>),
    Clear,
}

pub struct AudioInput {
    _stream: Stream,
}

pub struct AudioOutput {
    _stream: Stream,
    queued_samples: Arc<AtomicUsize>,
}

impl AudioOutput {
    pub fn queued_samples_handle(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.queued_samples)
    }
}

pub fn list_input_devices() -> Vec<(String, Device)> {
    let host = cpal::default_host();
    host.input_devices()
        .map(|devs| {
            devs.filter_map(|d| {
                let name = d.name().ok()?;
                Some((name, d))
            })
            .collect()
        })
        .unwrap_or_default()
}

pub fn list_output_devices() -> Vec<(String, Device)> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|devs| {
            devs.filter_map(|d| {
                let name = d.name().ok()?;
                Some((name, d))
            })
            .collect()
        })
        .unwrap_or_default()
}

pub fn find_input_device(name: Option<&str>) -> Option<Device> {
    let target = name.map(str::trim).filter(|s| !s.is_empty())?;
    let lower = target.to_lowercase();
    list_input_devices()
        .into_iter()
        .find(|(n, _)| n.to_lowercase().contains(&lower))
        .map(|(_, d)| d)
}

pub fn find_output_device(name: Option<&str>) -> Option<Device> {
    let target = name.map(str::trim).filter(|s| !s.is_empty())?;
    let lower = target.to_lowercase();
    list_output_devices()
        .into_iter()
        .find(|(n, _)| n.to_lowercase().contains(&lower))
        .map(|(_, d)| d)
}

pub fn open_mic(device: Option<&Device>, tx: mpsc::Sender<Vec<u8>>) -> Result<AudioInput, String> {
    let host = cpal::default_host();
    let dev = match device {
        Some(d) => d.clone(),
        None => host
            .default_input_device()
            .ok_or("no input device available")?,
    };

    let default_cfg = dev
        .default_input_config()
        .map_err(|e| format!("no default input config: {e}"))?;

    tracing::info!(
        "mic default config: {}ch {}Hz {:?}",
        default_cfg.channels(),
        default_cfg.sample_rate().0,
        default_cfg.sample_format()
    );

    let config = StreamConfig {
        channels: default_cfg.channels(),
        sample_rate: default_cfg.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let native_rate = default_cfg.sample_rate().0;
    let native_channels = (default_cfg.channels() as usize).max(1);
    let sample_format = default_cfg.sample_format();

    let err_fn = |err| {
        tracing::error!("mic stream error: {err}");
    };

    let stream = match sample_format {
        SampleFormat::F32 => dev.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mono: Vec<i16> = data
                    .chunks(native_channels)
                    .map(|frame| {
                        let avg = frame.iter().sum::<f32>() / frame.len().max(1) as f32;
                        (avg * 32767.0).clamp(-32768.0, 32767.0) as i16
                    })
                    .collect();
                emit_mic_chunk(&mono, native_rate, &tx);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => dev.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                let mono: Vec<i16> = data
                    .chunks(native_channels)
                    .map(|frame| {
                        let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                        (sum / frame.len().max(1) as i32) as i16
                    })
                    .collect();
                emit_mic_chunk(&mono, native_rate, &tx);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => dev.build_input_stream(
            &config,
            move |data: &[u16], _: &cpal::InputCallbackInfo| {
                let mono: Vec<i16> = data
                    .chunks(native_channels)
                    .map(|frame| {
                        let sum: i32 = frame.iter().map(|&s| (s as i32) - 32768).sum();
                        (sum / frame.len().max(1) as i32) as i16
                    })
                    .collect();
                emit_mic_chunk(&mono, native_rate, &tx);
            },
            err_fn,
            None,
        ),
        other => {
            return Err(format!("unsupported mic sample format: {other:?}"));
        }
    }
    .map_err(|e| format!("failed to open mic: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("failed to start mic: {e}"))?;
    Ok(AudioInput { _stream: stream })
}

fn emit_mic_chunk(mono: &[i16], native_rate: u32, tx: &mpsc::Sender<Vec<u8>>) {
    let resampled = if native_rate != SEND_SAMPLE_RATE {
        resample_filtered(mono, native_rate, SEND_SAMPLE_RATE)
    } else {
        mono.to_vec()
    };

    let mut bytes = Vec::with_capacity(resampled.len() * 2);
    for s in resampled {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    let _ = tx.try_send(bytes);
}

pub fn open_speaker(
    device: Option<&Device>,
    mut rx: mpsc::Receiver<PlaybackCmd>,
) -> Result<AudioOutput, String> {
    let host = cpal::default_host();
    let dev = match device {
        Some(d) => d.clone(),
        None => host
            .default_output_device()
            .ok_or("no output device available")?,
    };

    let default_cfg = dev
        .default_output_config()
        .map_err(|e| format!("no default output config: {e}"))?;

    tracing::info!(
        "speaker default config: {}ch {}Hz {:?}",
        default_cfg.channels(),
        default_cfg.sample_rate().0,
        default_cfg.sample_format()
    );

    let config = StreamConfig {
        channels: default_cfg.channels(),
        sample_rate: default_cfg.sample_rate(),
        buffer_size: cpal::BufferSize::Default,
    };

    let native_rate = default_cfg.sample_rate().0;
    let native_channels = (default_cfg.channels() as usize).max(1);
    let sample_format = default_cfg.sample_format();

    // Cap speaker buffer to ~12 seconds of audio to prevent unbounded backlog
    let max_buf_samples = (native_rate as usize) * native_channels * 12;

    let buffer = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(
        native_rate as usize * native_channels,
    )));
    let queued_samples = Arc::new(AtomicUsize::new(0));

    let buf_write = Arc::clone(&buffer);
    let queued_write = Arc::clone(&queued_samples);

    std::thread::spawn(move || {
        while let Some(cmd) = rx.blocking_recv() {
            match cmd {
                PlaybackCmd::Clear => {
                    if let Ok(mut buf) = buf_write.lock() {
                        buf.clear();
                        queued_write.store(0, Ordering::Release);
                    }
                }
                PlaybackCmd::Audio(bytes) => {
                    if bytes.len() < 2 {
                        continue;
                    }
                    let mono_i16: Vec<i16> = bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&c| i16::from_le_bytes(c))
                        .collect();

                    let resampled = if native_rate != RECV_SAMPLE_RATE {
                        resample_filtered(&mono_i16, RECV_SAMPLE_RATE, native_rate)
                    } else {
                        mono_i16
                    };

                    if let Ok(mut buf) = buf_write.lock() {
                        let incoming = resampled.len() * native_channels;
                        if buf.len() + incoming > max_buf_samples {
                            let overflow = (buf.len() + incoming).saturating_sub(max_buf_samples);
                            let drop_n = overflow.min(buf.len());
                            buf.drain(..drop_n);
                        }
                        for s in &resampled {
                            let f = *s as f32 / 32768.0;
                            for _ in 0..native_channels {
                                buf.push_back(f);
                            }
                        }
                        queued_write.store(buf.len(), Ordering::Release);
                    }
                }
            }
        }
    });

    let err_fn = |err| {
        tracing::error!("speaker stream error: {err}");
    };

    let queued_read = Arc::clone(&queued_samples);
    let stream = match sample_format {
        SampleFormat::F32 => dev.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut buf) = buffer.lock() {
                    for sample in data.iter_mut() {
                        *sample = buf.pop_front().unwrap_or(0.0);
                    }
                    queued_read.store(buf.len(), Ordering::Release);
                } else {
                    data.fill(0.0);
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => dev.build_output_stream(
            &config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut buf) = buffer.lock() {
                    for sample in data.iter_mut() {
                        let f = buf.pop_front().unwrap_or(0.0);
                        *sample = (f * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    }
                    queued_read.store(buf.len(), Ordering::Release);
                } else {
                    data.fill(0);
                }
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => dev.build_output_stream(
            &config,
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                if let Ok(mut buf) = buffer.lock() {
                    for sample in data.iter_mut() {
                        let f = buf.pop_front().unwrap_or(0.0);
                        *sample = ((f * 32767.0).clamp(-32768.0, 32767.0) as i32 + 32768) as u16;
                    }
                    queued_read.store(buf.len(), Ordering::Release);
                } else {
                    data.fill(32768);
                }
            },
            err_fn,
            None,
        ),
        other => {
            return Err(format!("unsupported speaker sample format: {other:?}"));
        }
    }
    .map_err(|e| format!("failed to open speaker: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("failed to start speaker: {e}"))?;

    Ok(AudioOutput {
        _stream: stream,
        queued_samples,
    })
}

/// Linear resampler with a simple anti-aliasing box-average pre-filter when downsampling.
fn resample_filtered(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = (input.len() as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(out_len);

    if to_rate < from_rate {
        // Downsampling: average window around each source sample to suppress aliasing
        let step = from_rate as f64 / to_rate as f64;
        let half_win = (step * 0.5).ceil() as usize;
        for i in 0..out_len {
            let center = (i as f64 * step).round() as usize;
            let lo = center.saturating_sub(half_win);
            let hi = (center + half_win).min(input.len() - 1);
            let mut sum: i64 = 0;
            let mut count: i64 = 0;
            for &s in &input[lo..=hi] {
                sum += s as i64;
                count += 1;
            }
            output.push((sum / count.max(1)) as i16);
        }
    } else {
        // Upsampling: linear interpolation
        for i in 0..out_len {
            let src_pos = i as f64 / ratio;
            let idx = src_pos as usize;
            let frac = src_pos - idx as f64;
            let a = input[idx.min(input.len() - 1)] as f64;
            let b = input[(idx + 1).min(input.len() - 1)] as f64;
            output.push((a + (b - a) * frac) as i16);
        }
    }
    output
}
