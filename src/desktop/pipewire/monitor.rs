/// Metadata for one captured desktop stream.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// PipeWire node ID for portal streams; zero for direct-frame streams.
    pub node_id: u32,
    /// Position in the compositor's global coordinate space.
    pub position: (i32, i32),
    /// Width and height in pixels.
    pub size: (u32, u32),
    /// Kind of source represented by this stream.
    pub source_type: SourceType,
}

/// Desktop source represented by a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceType {
    /// Complete monitor or virtual output.
    #[default]
    Monitor,
    /// Window-scoped source.
    Window,
    /// Synthetic virtual source.
    Virtual,
}

/// Geometry used by input coordinate mapping.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// Stable stream/monitor identifier.
    pub id: u32,
    /// Display name used in diagnostics.
    pub name: String,
    /// Position in the global coordinate space.
    pub position: (i32, i32),
    /// Width and height in pixels.
    pub size: (u32, u32),
    /// Nominal refresh rate in hertz.
    pub refresh_rate: u32,
    /// PipeWire node ID, or zero for direct-frame streams.
    pub node_id: u32,
}

impl MonitorInfo {
    pub fn from_stream_info(stream: &StreamInfo, name: String) -> Self {
        Self {
            id: stream.node_id,
            name,
            position: stream.position,
            size: stream.size,
            refresh_rate: 60,
            node_id: stream.node_id,
        }
    }
}
