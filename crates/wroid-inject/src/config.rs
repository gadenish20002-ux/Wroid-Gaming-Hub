use thiserror::Error;

pub const DEFAULT_SLOT_COUNT: u16 = 10;
pub const MAX_SLOT_COUNT: u16 = 32;
pub(crate) const MAX_EVENTS_PER_FRAME: usize = MAX_SLOT_COUNT as usize * 4 + 3;
const DEVICE_NAME: &str = "Wroid Gaming Touchscreen";
const MAX_DEVICE_NAME_LEN: usize = 78;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceConfig {
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slot_count: u16,
}

impl DeviceConfig {
    pub fn new(width: u32, height: u32) -> Result<Self, DeviceConfigError> {
        Self::with_name_and_slots(DEVICE_NAME, width, height, DEFAULT_SLOT_COUNT)
    }

    pub fn with_slots(
        width: u32,
        height: u32,
        slot_count: u16,
    ) -> Result<Self, DeviceConfigError> {
        Self::with_name_and_slots(DEVICE_NAME, width, height, slot_count)
    }

    pub fn with_name_and_slots(
        name: impl Into<String>,
        width: u32,
        height: u32,
        slot_count: u16,
    ) -> Result<Self, DeviceConfigError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DeviceConfigError::EmptyName);
        }
        if name.len() > MAX_DEVICE_NAME_LEN {
            return Err(DeviceConfigError::NameTooLong {
                length: name.len(),
                max: MAX_DEVICE_NAME_LEN,
            });
        }
        if width == 0 || width > i32::MAX as u32 {
            return Err(DeviceConfigError::InvalidWidth { width });
        }
        if height == 0 || height > i32::MAX as u32 {
            return Err(DeviceConfigError::InvalidHeight { height });
        }
        if !(1..=MAX_SLOT_COUNT).contains(&slot_count) {
            return Err(DeviceConfigError::InvalidSlotCount {
                slot_count,
                max: MAX_SLOT_COUNT,
            });
        }

        Ok(Self {
            name,
            width,
            height,
            slot_count,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn slot_count(&self) -> u16 {
        self.slot_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeviceConfigError {
    #[error("uinput device name must not be empty")]
    EmptyName,
    #[error("uinput device name is {length} bytes; maximum is {max}")]
    NameTooLong { length: usize, max: usize },
    #[error("touch width must be between 1 and 2147483647; got {width}")]
    InvalidWidth { width: u32 },
    #[error("touch height must be between 1 and 2147483647; got {height}")]
    InvalidHeight { height: u32 },
    #[error("touch slot count must be between 1 and {max}; got {slot_count}")]
    InvalidSlotCount { slot_count: u16, max: u16 },
}
