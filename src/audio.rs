use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Stream, StreamConfig};
use tokio::sync::mpsc;

use crate::{RECV_SAMPLE_RATE, SEND_SAMPLE_RATE};

pub struct AudioInput {
    _stream: Stream,
}

pub struct AudioOutput {
    _stream: Stream,
    _latency_secs: f64,
}

pub fn default_input_device() -> Option<Device> {
    let host = cpal::default_host();
    host.default_input_device()
}

pub fn default_output_device() -> Option<Device> {
    let host = cpal::default_host();
    host.default_output_device()
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
    let native_channels = default_cfg.channels() as usize;

    let stream = dev
        .build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let mono: Vec<i16> = data
                    .chunks(native_channels)
                    .map(|frame| {
                        let sum: f32 = frame.iter().sum();
                        let avg = sum / native_channels as f32;
                        (avg * 32767.0).clamp(-32768.0, 32767.0) as i16
                    })
                    .collect();

                let resampled = if native_rate != SEND_SAMPLE_RATE {
                    resample(&mono, native_rate, SEND_SAMPLE_RATE)
                } else {
                    mono
                };

                let bytes: Vec<u8> = resampled
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect();
                let _ = tx.try_send(bytes);
            },
            |err| {
                tracing::error!("mic stream error: {err}");
            },
            None,
        )
        .map_err(|e| format!("failed to open mic: {e}"))?;

    stream.play().map_err(|e| format!("failed to start mic: {e}"))?;
    Ok(AudioInput { _stream: stream })
}

pub fn open_speaker(
    device: Option<&Device>,
    mut rx: mpsc::Receiver<Vec<u8>>,
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
    let native_channels = default_cfg.channels() as usize;

    let buffer = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::<f32>::new()));
    let buf_write = buffer.clone();

    std::thread::spawn(move || {
        while let Some(bytes) = rx.blocking_recv() {
            let mono_i16: Vec<i16> = bytes
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();

            let resampled = if native_rate != RECV_SAMPLE_RATE {
                resample(&mono_i16, RECV_SAMPLE_RATE, native_rate)
            } else {
                mono_i16
            };

            let mut buf = buf_write.lock().unwrap();
            for s in &resampled {
                let f = *s as f32 / 32768.0;
                for _ in 0..native_channels {
                    buf.push_back(f);
                }
            }
        }
    });

    let stream = dev
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut buf = buffer.lock().unwrap();
                for sample in data.iter_mut() {
                    *sample = buf.pop_front().unwrap_or(0.0);
                }
            },
            |err| {
                tracing::error!("speaker stream error: {err}");
            },
            None,
        )
        .map_err(|e| format!("failed to open speaker: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("failed to start speaker: {e}"))?;

    let latency_secs = 1024.0 / RECV_SAMPLE_RATE as f64;

    Ok(AudioOutput {
        _stream: stream,
        _latency_secs: latency_secs,
    })
}

fn resample(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = (input.len() as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos as usize;
        let frac = src_pos - idx as f64;
        let a = input[idx.min(input.len() - 1)] as f64;
        let b = input[(idx + 1).min(input.len() - 1)] as f64;
        output.push((a + (b - a) * frac) as i16);
    }
    output
}
