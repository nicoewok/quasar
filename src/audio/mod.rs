use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use realfft::RealFftPlanner;
use ringbuf::HeapRb;
/// Exported handle containing thread-safe spectrum buffer and global volume.
#[derive(Clone)]
pub struct AudioHandle {
    /// 512-bin frequency spectrum, values normalized in range [0.0, 1.0].
    pub spectrum: Arc<Mutex<[f32; 512]>>,
    /// Global volume level (RMS amplitude), normalized [0.0, 1.0].
    volume_bits: Arc<AtomicU32>,
}

impl AudioHandle {
    pub fn new() -> Self {
        Self {
            spectrum: Arc::new(Mutex::new([0.0f32; 512])),
            volume_bits: Arc::new(AtomicU32::new(0.0f32.to_bits())),
        }
    }

    pub fn get_volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, vol: f32) {
        self.volume_bits.store(vol.to_bits(), Ordering::Relaxed);
    }
}

pub struct AudioSystem {
    pub handle: AudioHandle,
    _stream: Option<cpal::Stream>,
}

impl AudioSystem {
    /// Initializes the audio stream and DSP processing thread.
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let handle = AudioHandle::new();

        // Lock-free ring buffer (4096 samples capacity)
        let ring_buffer = HeapRb::<f32>::new(4096);
        let (mut producer, consumer) = ring_buffer.split();

        // Set up CPAL audio stream
        let host = cpal::default_host();
        
        // Try default output device or fallback to input device
        let device = host
            .default_output_device()
            .or_else(|| host.default_input_device())
            .ok_or("No audio input/output device found")?;

        let config = device.default_output_config()
            .or_else(|_| device.default_input_config())?;

        let sample_rate = config.sample_rate().0 as f32;
        let channels = config.channels() as usize;

        let err_fn = |err| log::error!("Audio stream error: {}", err);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                device.build_input_stream(
                    &config.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        // Mix multi-channel input to mono
                        for chunk in data.chunks_exact(channels) {
                            let mono: f32 = chunk.iter().sum::<f32>() / (channels as f32);
                            let _ = producer.push(mono);
                        }
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        for chunk in data.chunks_exact(channels) {
                            let mono: f32 = chunk.iter().map(|&s| s as f32 / i16::MAX as f32).sum::<f32>() / (channels as f32);
                            let _ = producer.push(mono);
                        }
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                device.build_input_stream(
                    &config.into(),
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        for chunk in data.chunks_exact(channels) {
                            let mono: f32 = chunk.iter().map(|&s| (s as f32 - u16::MAX as f32 / 2.0) / (u16::MAX as f32 / 2.0)).sum::<f32>() / (channels as f32);
                            let _ = producer.push(mono);
                        }
                    },
                    err_fn,
                    None,
                )
            }
            _ => return Err("Unsupported sample format".into()),
        };

        let stream = match stream {
            Ok(s) => {
                let _ = s.play();
                Some(s)
            }
            Err(e) => {
                log::warn!("Could not start audio stream: {}. Running DSP loop with synthetic silence.", e);
                None
            }
        };

        // Spawn DSP processing thread
        let handle_clone = handle.clone();
        thread::spawn(move || {
            dsp_loop(consumer, handle_clone, sample_rate);
        });

        Ok(Self {
            handle,
            _stream: stream,
        })
    }
}

/// Real-time DSP processing loop reading 1024-sample windows from ring buffer.
fn dsp_loop(
    mut consumer: ringbuf::HeapConsumer<f32>,
    handle: AudioHandle,
    sample_rate: f32,
) {
    const WINDOW_SIZE: usize = 1024;
    const NUM_BINS: usize = 512;

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);
    
    let mut indata = vec![0.0f32; WINDOW_SIZE];
    let mut spectrum_prev = [0.0f32; NUM_BINS];

    // Pre-calculate Hann window coefficients
    let hann_window: Vec<f32> = (0..WINDOW_SIZE)
        .map(|n| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (WINDOW_SIZE - 1) as f32).cos()))
        .collect();

    // Pre-calculate logarithmic bin mappings (20 Hz to 20 kHz)
    let min_freq = 20.0f32;
    let max_freq = 20000.0f32.min(sample_rate / 2.0);
    let bin_frequencies: Vec<f32> = (0..NUM_BINS)
        .map(|i| {
            min_freq * (max_freq / min_freq).powf(i as f32 / (NUM_BINS - 1) as f32)
        })
        .collect();

    let nyquist = sample_rate / 2.0;

    loop {
        // Drain sample buffer into working window
        let mut samples_read = 0;
        let mut temp_buf = [0.0f32; 128];
        while samples_read < WINDOW_SIZE {
            let read = consumer.pop_slice(&mut temp_buf);
            if read == 0 {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            for &s in &temp_buf[..read] {
                if samples_read < WINDOW_SIZE {
                    indata[samples_read] = s;
                    samples_read += 1;
                }
            }
        }

        // Calculate global RMS volume
        let rms_sum: f32 = indata.iter().map(|&x| x * x).sum();
        let volume = (rms_sum / WINDOW_SIZE as f32).sqrt().clamp(0.0, 1.0);
        handle.set_volume(volume);

        // Apply Hann windowing
        let mut windowed_input: Vec<f32> = indata.iter().zip(hann_window.iter()).map(|(&x, &w)| x * w).collect();
        let mut out_spectrum = fft.make_output_vec();

        if fft.process(&mut windowed_input, &mut out_spectrum).is_ok() {
            // Calculate frequency magnitudes and convert to dB
            let num_fft_bins = out_spectrum.len(); // WINDOW_SIZE / 2 + 1 = 513
            let fft_mags: Vec<f32> = out_spectrum
                .iter()
                .map(|c| {
                    let mag = c.norm();
                    // 20 * log10(amplitude) normalized from dB [-60, 0] to [0.0, 1.0]
                    let db = 20.0 * (mag.max(1e-6)).log10();
                    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
                })
                .collect();

            // Logarithmic binning mapping to 512 target bins
            let mut target_bins = [0.0f32; NUM_BINS];
            for (i, &target_freq) in bin_frequencies.iter().enumerate() {
                let bin_idx = ((target_freq / nyquist) * (num_fft_bins - 1) as f32).round() as usize;
                let bin_idx = bin_idx.clamp(0, num_fft_bins - 1);
                target_bins[i] = fft_mags[bin_idx];
            }

            // Exponential decay smoothing (alpha = 0.75 for decay, fast attack)
            let mut smoothed_bins = [0.0f32; NUM_BINS];
            for i in 0..NUM_BINS {
                let target = target_bins[i];
                let prev = spectrum_prev[i];
                let alpha = if target >= prev { 0.2 } else { 0.8 }; // Fast attack, slow decay
                smoothed_bins[i] = alpha * prev + (1.0 - alpha) * target;
            }
            spectrum_prev = smoothed_bins;

            // Write output spectrum to shared thread-safe buffer
            if let Ok(mut spec_guard) = handle.spectrum.lock() {
                *spec_guard = smoothed_bins;
            }
        }

        // Shift window by 512 samples for 50% overlap
        indata.copy_within(512..WINDOW_SIZE, 0);
    }
}
