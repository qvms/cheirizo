//! H.264 Annex A level selection for desktop encoders.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum H264Level {
    L3_0 = 30,
    L3_1 = 31,
    L3_2 = 32,
    L4_0 = 40,
    L4_1 = 41,
    L4_2 = 42,
    L5_0 = 50,
    L5_1 = 51,
    L5_2 = 52,
}

#[derive(Clone, Copy)]
struct Limits {
    frame: u32,
    rate: u32,
}
impl H264Level {
    const fn limits(self) -> Limits {
        match self {
            Self::L3_0 => Limits {
                frame: 1620,
                rate: 40500,
            },
            Self::L3_1 => Limits {
                frame: 3600,
                rate: 108000,
            },
            Self::L3_2 => Limits {
                frame: 5120,
                rate: 216000,
            },
            Self::L4_0 | Self::L4_1 => Limits {
                frame: 8192,
                rate: 245760,
            },
            Self::L4_2 => Limits {
                frame: 8704,
                rate: 522240,
            },
            Self::L5_0 => Limits {
                frame: 22080,
                rate: 589824,
            },
            Self::L5_1 => Limits {
                frame: 36864,
                rate: 983040,
            },
            Self::L5_2 => Limits {
                frame: 36864,
                rate: 2073600,
            },
        }
    }
    pub const fn max_macroblocks_per_second(&self) -> u32 {
        self.limits().rate
    }
    pub const fn max_frame_macroblocks(&self) -> u32 {
        self.limits().frame
    }
    pub const fn effective_max_mbs_per_sec(&self, _frame_mbs: u32) -> u32 {
        self.limits().rate
    }
    pub const fn to_openh264_level_idc(&self) -> i32 {
        *self as i32
    }

    #[cfg(feature = "h264")]
    pub const fn to_upstream(self) -> openh264::encoder::Level {
        use openh264::encoder::Level;
        match self {
            Self::L3_0 => Level::Level_3_0,
            Self::L3_1 => Level::Level_3_1,
            Self::L3_2 => Level::Level_3_2,
            Self::L4_0 => Level::Level_4_0,
            Self::L4_1 => Level::Level_4_1,
            Self::L4_2 => Level::Level_4_2,
            Self::L5_0 => Level::Level_5_0,
            Self::L5_1 => Level::Level_5_1,
            Self::L5_2 => Level::Level_5_2,
        }
    }
    pub fn for_config(width: u16, height: u16, fps: f32) -> Self {
        let frame = macroblocks(width, height);
        let rate = frame as f32 * fps.max(0.0);
        Self::levels()
            .find(|l| frame <= l.limits().frame && rate <= l.limits().rate as f32)
            .unwrap_or(Self::L5_2)
    }
    pub fn iter_ascending() -> impl Iterator<Item = Self> {
        Self::levels()
    }
    fn levels() -> impl Iterator<Item = Self> {
        [
            Self::L3_0,
            Self::L3_1,
            Self::L3_2,
            Self::L4_0,
            Self::L4_1,
            Self::L4_2,
            Self::L5_0,
            Self::L5_1,
            Self::L5_2,
        ]
        .into_iter()
    }
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::L3_0 => "3.0",
            Self::L3_1 => "3.1",
            Self::L3_2 => "3.2",
            Self::L4_0 => "4.0",
            Self::L4_1 => "4.1",
            Self::L4_2 => "4.2",
            Self::L5_0 => "5.0",
            Self::L5_1 => "5.1",
            Self::L5_2 => "5.2",
        }
    }
}
impl fmt::Display for H264Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Level {}", self.as_str())
    }
}

pub struct LevelConstraints {
    width: u16,
    height: u16,
    macroblocks: u32,
}
impl LevelConstraints {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            macroblocks: macroblocks(width, height),
        }
    }
    pub const fn macroblocks(&self) -> u32 {
        self.macroblocks
    }
    pub fn max_fps_for_level(&self, level: H264Level) -> f32 {
        level.limits().rate as f32 / self.macroblocks.max(1) as f32
    }
    pub fn recommend_level(&self, fps: f32) -> H264Level {
        H264Level::for_config(self.width, self.height, fps)
    }
    pub fn validate(&self, fps: f32, level: H264Level) -> Result<(), ConstraintViolation> {
        let limits = level.limits();
        if self.macroblocks > limits.frame {
            return Err(ConstraintViolation::FrameSizeExceeded {
                macroblocks: self.macroblocks,
                max_macroblocks: limits.frame,
                level,
            });
        }
        let required = (self.macroblocks as f32 * fps.max(0.0)).ceil() as u32;
        if required > limits.rate {
            return Err(ConstraintViolation::MacroblocksPerSecondExceeded {
                required,
                max: limits.rate,
                level,
                resolution: (self.width, self.height),
                fps,
            });
        }
        Ok(())
    }
    pub fn adjust_fps_for_level(&self, target: f32, level: H264Level) -> f32 {
        target.min(self.max_fps_for_level(level))
    }
}
fn macroblocks(width: u16, height: u16) -> u32 {
    u32::from(width).div_ceil(16) * u32::from(height).div_ceil(16)
}

/// Failure to satisfy an H.264 level limit.
///
/// The frame-size and macroblock-rate dimensions are defined by ITU-T H.264
/// (08/2024), Annex A, especially Table A-1. OpenH264 exposes the selected
/// level to the encoder; WRDP checks these published limits before encoding.
#[derive(Debug)]
pub enum ConstraintViolation {
    FrameSizeExceeded {
        macroblocks: u32,
        max_macroblocks: u32,
        level: H264Level,
    },
    MacroblocksPerSecondExceeded {
        required: u32,
        max: u32,
        level: H264Level,
        resolution: (u16, u16),
        fps: f32,
    },
}
impl fmt::Display for ConstraintViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameSizeExceeded {
                macroblocks,
                max_macroblocks,
                level,
            } => write!(
                f,
                "{macroblocks} macroblocks exceed {level} limit {max_macroblocks}"
            ),
            Self::MacroblocksPerSecondExceeded {
                required,
                max,
                level,
                resolution,
                fps,
            } => write!(
                f,
                "{}x{} at {fps:.1} fps requires {required} MB/s; {level} allows {max}",
                resolution.0, resolution.1
            ),
        }
    }
}
impl std::error::Error for ConstraintViolation {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_levels() {
        assert_eq!(H264Level::for_config(1280, 720, 30.0), H264Level::L3_1);
        assert_eq!(H264Level::for_config(1920, 1080, 30.0), H264Level::L4_0);
        assert_eq!(H264Level::for_config(3840, 2160, 30.0), H264Level::L5_1);
    }
    #[test]
    fn constraints_reject_excess_rate() {
        let c = LevelConstraints::new(1920, 1080);
        assert!(c.validate(60.0, H264Level::L4_0).is_err());
        assert!(c.validate(30.0, H264Level::L4_0).is_ok());
    }
}
