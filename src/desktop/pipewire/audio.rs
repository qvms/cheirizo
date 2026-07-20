//! PipeWire audio capture.
//!
//! Captures desktop audio via PipeWire, forwards PCM sample buffers through a
//! bounded channel, and runs on a dedicated thread because PipeWire objects are
//! not Send.

use std::convert::TryInto;
use std::mem;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use pipewire as pw;
use pw::spa;
use pw::spa::param::format::{MediaSubtype, MediaType};
use pw::spa::param::format_utils;
use pw::spa::pod::Pod;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

/// Audio sample format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// 32-bit float (native endian)
    F32,
    /// 16-bit signed integer (native endian)
    I16,
}

impl AudioFormat {
    fn to_spa_format(self) -> spa::param::audio::AudioFormat {
        match self {
            Self::F32 => spa::param::audio::AudioFormat::F32LE,
            Self::I16 => spa::param::audio::AudioFormat::S16LE,
        }
    }

    /// Bytes per single sample (one channel)
    pub fn bytes_per_sample(self) -> usize {
        match self {
            Self::F32 => mem::size_of::<f32>(),
            Self::I16 => mem::size_of::<i16>(),
        }
    }
}

/// Audio capture configuration
#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Sample rate in Hz (default: 48000)
    pub sample_rate: u32,
    /// Number of channels (default: 2)
    pub channels: u32,
    /// Output sample format
    pub format: AudioFormat,
    /// Frames per buffer (default: 1024, ~21ms at 48kHz)
    pub buffer_frames: u32,
    /// Optional PipeWire remote name or absolute socket path.
    pub remote_name: Option<String>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            format: AudioFormat::F32,
            buffer_frames: 1024,
            remote_name: None,
        }
    }
}

/// Typed audio sample buffer
#[derive(Debug, Clone)]
pub enum AudioSamples {
    /// 32-bit float samples
    F32(Vec<f32>),
    /// 16-bit signed integer samples
    I16(Vec<i16>),
}

impl AudioSamples {
    /// Number of samples (all channels combined)
    pub fn len(&self) -> usize {
        match self {
            Self::F32(s) => s.len(),
            Self::I16(s) => s.len(),
        }
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Convert to i16 samples
    pub fn to_i16(&self) -> Vec<i16> {
        match self {
            Self::F32(samples) => samples
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect(),
            Self::I16(samples) => samples.clone(),
        }
    }

    /// Convert to f32 samples
    pub fn to_f32(&self) -> Vec<f32> {
        match self {
            Self::F32(samples) => samples.clone(),
            Self::I16(samples) => samples.iter().map(|&s| s as f32 / 32768.0).collect(),
        }
    }
}

/// Handle to a running audio capture session
pub struct AudioCaptureHandle {
    /// Receiver for captured audio samples
    pub receiver: mpsc::Receiver<AudioSamples>,
    stop_signal: Arc<AtomicBool>,
}

impl AudioCaptureHandle {
    /// Clone the thread-safe stop signal for connection-owned tasks.
    pub fn stop_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop_signal)
    }

    /// Signal the capture thread to stop
    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
    }

    /// Check if capture has been stopped
    pub fn is_stopped(&self) -> bool {
        self.stop_signal.load(Ordering::SeqCst)
    }
}

struct CaptureUserData {
    format: spa::param::audio::AudioInfoRaw,
    output_format: AudioFormat,
    sender: mpsc::Sender<AudioSamples>,
    stop_signal: Arc<AtomicBool>,
    samples_captured: u64,
    samples_dropped: u64,
}

/// PipeWire audio capture engine.
///
/// Captures desktop audio and forwards PCM sample buffers through a channel.
/// Must run on a dedicated thread via [`spawn_audio_capture`].
pub struct AudioCapture {
    config: CaptureConfig,
    sender: mpsc::Sender<AudioSamples>,
    stop_signal: Arc<AtomicBool>,
}

impl AudioCapture {
    /// Create a new capture instance and its handle
    pub fn new(config: CaptureConfig, channel_size: usize) -> (Self, AudioCaptureHandle) {
        let (sender, receiver) = mpsc::channel(channel_size);
        let stop_signal = Arc::new(AtomicBool::new(false));

        let capture = Self {
            config,
            sender,
            stop_signal: Arc::clone(&stop_signal),
        };

        let handle = AudioCaptureHandle {
            receiver,
            stop_signal,
        };

        (capture, handle)
    }

    /// Run the PipeWire main loop for audio capture (blocking).
    ///
    /// Call from a dedicated thread. Connects to the PipeWire daemon,
    /// negotiates audio format, and delivers samples until stopped.
    pub fn start_capture(&self, node_id: Option<u32>) -> Result<()> {
        info!(
            "Starting audio capture: {}Hz, {} channels, format={:?}, node_id={:?}",
            self.config.sample_rate, self.config.channels, self.config.format, node_id
        );

        // PipeWire 0.9 Box types for owned resources
        let mainloop =
            pw::main_loop::MainLoopBox::new(None).context("Failed to create PipeWire MainLoop")?;
        let context = pw::context::ContextBox::new(mainloop.loop_(), None)
            .context("Failed to create PipeWire Context")?;
        let remote_properties = self.config.remote_name.as_ref().map(|remote| {
            pw::properties::properties! {
                *pw::keys::REMOTE_NAME => remote.as_str()
            }
        });
        let core = context
            .connect(remote_properties)
            .context("Failed to connect to PipeWire daemon")?;

        let mut props = pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::NODE_NAME => "wrdp-audio-capture",
            *pw::keys::APP_NAME => "wrdp",
        };

        if let Some(id) = node_id {
            props.insert("target.object", id.to_string());
        }

        props.insert("stream.capture.sink", "true");

        let stream = pw::stream::StreamBox::new(&core, "wrdp-audio-capture", props)
            .context("Failed to create PipeWire stream")?;

        let user_data = CaptureUserData {
            format: spa::param::audio::AudioInfoRaw::default(),
            output_format: self.config.format,
            sender: self.sender.clone(),
            stop_signal: Arc::clone(&self.stop_signal),
            samples_captured: 0,
            samples_dropped: 0,
        };

        let stop_signal_for_callback = Arc::clone(&self.stop_signal);

        let _listener = stream
            .add_local_listener_with_user_data(user_data)
            .state_changed(move |_stream, _user_data, old, new| {
                debug!("Audio stream state: {:?} -> {:?}", old, new);

                match new {
                    pw::stream::StreamState::Error(err) => {
                        error!("Audio stream error: {}", err);
                        stop_signal_for_callback.store(true, Ordering::SeqCst);
                    }
                    pw::stream::StreamState::Streaming => {
                        info!("Audio capture streaming started");
                    }
                    pw::stream::StreamState::Paused => {
                        debug!("Audio stream paused");
                    }
                    _ => {}
                }
            })
            .param_changed(|_stream, user_data, id, param| {
                let Some(param) = param else {
                    return;
                };

                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }

                let (media_type, media_subtype) = match format_utils::parse_format(param) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!("Failed to parse audio format: {:?}", e);
                        return;
                    }
                };

                if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                    debug!(
                        "Ignoring non-raw audio format: {:?}/{:?}",
                        media_type, media_subtype
                    );
                    return;
                }

                if let Err(e) = user_data.format.parse(param) {
                    warn!("Failed to parse audio info: {:?}", e);
                    return;
                }

                info!(
                    "Audio format negotiated: rate={}, channels={}, format={:?}",
                    user_data.format.rate(),
                    user_data.format.channels(),
                    user_data.format.format()
                );
            })
            .process(|stream, user_data| {
                if user_data.stop_signal.load(Ordering::Relaxed) {
                    return;
                }

                let Some(mut buffer) = stream.dequeue_buffer() else {
                    trace!("No buffer available");
                    return;
                };

                let datas = buffer.datas_mut();
                if datas.is_empty() {
                    return;
                }

                let data = &mut datas[0];
                let chunk = data.chunk();
                let size = chunk.size() as usize;

                if size == 0 {
                    return;
                }

                let Some(slice) = data.data() else {
                    return;
                };

                let n_channels = user_data.format.channels() as usize;
                if n_channels == 0 {
                    return;
                }

                let samples = match user_data.format.format() {
                    spa::param::audio::AudioFormat::F32LE
                    | spa::param::audio::AudioFormat::F32BE => {
                        let byte_count = size.min(slice.len());
                        let sample_count = byte_count / mem::size_of::<f32>();
                        let mut f32_samples = Vec::with_capacity(sample_count);

                        for i in 0..sample_count {
                            let start = i * mem::size_of::<f32>();
                            let end = start + mem::size_of::<f32>();
                            if end <= slice.len() {
                                let bytes: [u8; 4] = slice[start..end].try_into().unwrap_or([0; 4]);
                                let sample = if user_data.format.format()
                                    == spa::param::audio::AudioFormat::F32LE
                                {
                                    f32::from_le_bytes(bytes)
                                } else {
                                    f32::from_be_bytes(bytes)
                                };
                                f32_samples.push(sample);
                            }
                        }

                        match user_data.output_format {
                            AudioFormat::F32 => AudioSamples::F32(f32_samples),
                            AudioFormat::I16 => {
                                let i16_samples: Vec<i16> = f32_samples
                                    .iter()
                                    .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                                    .collect();
                                AudioSamples::I16(i16_samples)
                            }
                        }
                    }
                    spa::param::audio::AudioFormat::S16LE
                    | spa::param::audio::AudioFormat::S16BE => {
                        let byte_count = size.min(slice.len());
                        let sample_count = byte_count / mem::size_of::<i16>();
                        let mut i16_samples = Vec::with_capacity(sample_count);

                        for i in 0..sample_count {
                            let start = i * mem::size_of::<i16>();
                            let end = start + mem::size_of::<i16>();
                            if end <= slice.len() {
                                let bytes: [u8; 2] = slice[start..end].try_into().unwrap_or([0; 2]);
                                let sample = if user_data.format.format()
                                    == spa::param::audio::AudioFormat::S16LE
                                {
                                    i16::from_le_bytes(bytes)
                                } else {
                                    i16::from_be_bytes(bytes)
                                };
                                i16_samples.push(sample);
                            }
                        }

                        match user_data.output_format {
                            AudioFormat::I16 => AudioSamples::I16(i16_samples),
                            AudioFormat::F32 => {
                                let f32_samples: Vec<f32> =
                                    i16_samples.iter().map(|&s| s as f32 / 32768.0).collect();
                                AudioSamples::F32(f32_samples)
                            }
                        }
                    }
                    other => {
                        trace!("Unsupported audio format: {:?}", other);
                        return;
                    }
                };

                let sample_count = samples.len();

                // Non-blocking send preserves realtime capture behavior.
                match user_data.sender.try_send(samples) {
                    Ok(()) => {
                        user_data.samples_captured += sample_count as u64;
                        trace!("Captured {} samples", sample_count);
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        user_data.samples_dropped += sample_count as u64;
                        trace!("Dropped {} samples (channel full)", sample_count);
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        user_data.stop_signal.store(true, Ordering::SeqCst);
                        debug!("Audio sample channel closed");
                    }
                }
            })
            .register()
            .context("Failed to register stream listener")?;

        // Build format parameters for negotiation
        let mut audio_info = spa::param::audio::AudioInfoRaw::new();
        audio_info.set_format(self.config.format.to_spa_format());

        let obj = spa::pod::Object {
            type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: spa::param::ParamType::EnumFormat.as_raw(),
            properties: audio_info.into(),
        };

        let pod_bytes: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &spa::pod::Value::Object(obj),
        )
        .context("Failed to serialize audio format pod")?
        .0
        .into_inner();

        let pod = Pod::from_bytes(&pod_bytes).context("Failed to create pod from bytes")?;

        let mut params = [pod];

        let flags = pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS;

        stream
            .connect(spa::utils::Direction::Input, node_id, flags, &mut params)
            .context("Failed to connect PipeWire stream")?;

        info!("Audio capture stream connected, starting main loop");

        let loop_ref = mainloop.loop_();
        while !self.stop_signal.load(Ordering::Relaxed) {
            loop_ref.iterate(std::time::Duration::from_millis(100));
        }

        info!("Audio capture stopped");
        Ok(())
    }

    /// Signal the capture to stop
    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
    }
}

/// Spawn audio capture on a dedicated thread
///
/// Returns a handle with a receiver for audio samples. The capture runs
/// until the handle is dropped or `stop()` is called.
///
/// # Arguments
///
/// * `config` - Audio capture configuration
/// * `node_id` - Optional PipeWire node ID (from portal session)
/// * `channel_size` - Bounded channel capacity for sample buffers
pub fn spawn_audio_capture(
    config: CaptureConfig,
    node_id: Option<u32>,
    channel_size: usize,
) -> Result<AudioCaptureHandle> {
    let (capture, handle) = AudioCapture::new(config, channel_size);

    std::thread::Builder::new()
        .name("pipewire-audio".into())
        .spawn(move || {
            pw::init();

            if let Err(e) = capture.start_capture(node_id) {
                error!("Audio capture error: {:#}", e);
            }

            // SAFETY: Called once per init(), after all PipeWire resources dropped
            unsafe {
                pw::deinit();
            }
        })
        .context("Failed to spawn audio capture thread")?;

    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_formats_report_storage_width() {
        for (format, width) in [(AudioFormat::I16, 2), (AudioFormat::F32, 4)] {
            assert_eq!(format.bytes_per_sample(), width);
        }
    }

    #[test]
    fn default_capture_is_stereo_at_48_khz() {
        let actual = CaptureConfig::default();
        assert_eq!((actual.sample_rate, actual.channels), (48_000, 2));
        assert_eq!(actual.format, AudioFormat::F32);
    }

    #[test]
    fn conversion_preserves_zero_and_half_scale() {
        let integer = AudioSamples::F32(vec![0.0, -0.5, 0.5]).to_i16();
        assert_eq!(integer[0], 0);
        assert!((i32::from(integer[1]) + 16_383).abs() <= 1);
        assert!((i32::from(integer[2]) - 16_383).abs() <= 1);

        let float = AudioSamples::I16(vec![0, -16_384, 16_384]).to_f32();
        for (actual, expected) in float.into_iter().zip([0.0, -0.5, 0.5]) {
            assert!((actual - expected).abs() < 0.001);
        }
    }

    #[test]
    fn empty_buffer_and_stop_handle_have_observable_state() {
        let samples = AudioSamples::I16(Vec::new());
        assert_eq!(samples.len(), 0);
        assert!(samples.is_empty());

        let (_, handle) = AudioCapture::new(CaptureConfig::default(), 1);
        assert!(!handle.is_stopped());
        handle.stop();
        assert!(handle.is_stopped());
    }
}
