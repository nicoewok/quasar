use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, Stream, StreamConfig};
use realfft::RealFftPlanner;
use ringbuf::HeapRb;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[allow(dead_code)]
pub struct AudioSystem {
    pub stream: Stream,
    pub fft_spectrum: Arc<Mutex<[f32; 512]>>,
}


pub fn init_audio() -> Result<AudioSystem, Box<dyn std::error::Error>> {
    let host = cpal::default_host();

    let device = host
        .default_input_device()
        .or_else(|| host.default_output_device())
        .ok_or("Failed to find default audio input device")?;

    let device_name = device.name().unwrap_or_else(|_| "Unknown".to_string());
    println!("[Audio] Using input device: {}", device_name);

    let config = device.default_input_config()?;
    println!(
        "[Audio] Default input config: sample_rate={}, channels={}, format={:?}",
        config.sample_rate().0,
        config.channels(),
        config.sample_format()
    );

    let sample_format = config.sample_format();
    let stream_config: StreamConfig = config.into();

    let ring_buffer = HeapRb::<f32>::new(16384);
    let (mut producer, mut consumer) = ring_buffer.split();

    let err_fn = |err| eprintln!("[Audio Error] Stream error: {}", err);

    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| {
                for &sample in data {
                    let _ = producer.push(sample);
                }
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                for &sample in data {
                    let _ = producer.push(sample.to_sample::<f32>());
                }
            },
            err_fn,
            None,
        )?,
        SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                for &sample in data {
                    let _ = producer.push(sample.to_sample::<f32>());
                }
            },
            err_fn,
            None,
        )?,
        sample_format => {
            return Err(format!("Unsupported sample format: {:?}", sample_format).into())
        }
    };

    stream.play()?;

    let fft_spectrum = Arc::new(Mutex::new([0.0f32; 512]));
    let spectrum_clone = Arc::clone(&fft_spectrum);

    thread::spawn(move || {
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(1024);

        let mut input_buf = Vec::with_capacity(1024);
        let mut fft_input = r2c.make_input_vec();
        let mut fft_output = r2c.make_output_vec();
        let mut scratch = r2c.make_scratch_vec();

        let mut smoothed_spectrum = [0.0f32; 512];
        let smoothing_factor = 0.85f32;

        loop {
            while input_buf.len() < 1024 {
                if let Some(sample) = consumer.pop() {
                    input_buf.push(sample);
                } else {
                    break;
                }
            }

            if input_buf.len() >= 1024 {
                fft_input.copy_from_slice(&input_buf[..1024]);
                input_buf.clear();

                // Apply Hanning window to reduce spectral leakage
                for (i, sample) in fft_input.iter_mut().enumerate() {
                    let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 1023.0).cos());
                    *sample *= window;
                }

                if r2c.process_with_scratch(&mut fft_input, &mut fft_output, &mut scratch).is_ok() {
                    // Extract magnitude for the first 512 bins and apply exponential smoothing
                    let norm = 1.0 / (1024.0f32).sqrt();
                    for i in 0..512 {
                        let mag = (fft_output[i].norm() * norm).clamp(0.0, 1.0);
                        smoothed_spectrum[i] = smoothed_spectrum[i] * smoothing_factor
                            + mag * (1.0 - smoothing_factor);
                    }

                    if let Ok(mut lock) = spectrum_clone.lock() {
                        *lock = smoothed_spectrum;
                    }
                }
            } else {
                thread::sleep(Duration::from_millis(5));
            }
        }
    });

    Ok(AudioSystem {
        stream,
        fft_spectrum,
    })
}
