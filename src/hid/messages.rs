use std::ffi::CString;

/// HID input event types from the handheld controller.
#[derive(Debug, Clone)]
pub enum HidInputEvent {
    ButtonClick { encoder_id: u8 },
    ButtonDoubleClick { encoder_id: u8 },
    ButtonHoldStart { encoder_id: u8 },
    ButtonHoldEnd { encoder_id: u8 },
    MenuSelect { display_id: u8, item_id: u8 },
    MenuCancel { display_id: u8 },
}

/// A decoded HID input report.
#[derive(Debug, Clone)]
pub struct InputReport {
    pub active_screen_id: u8,
    pub encoder_deltas: [i8; 4],
    pub events: Vec<HidInputEvent>,
}

/// HID DeviceInfo, read via GET_REPORT at connection.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub firmware_version_major: u8,
    pub firmware_version_minor: u8,
    pub features: u8,
}

/// A HID output report to send.
#[derive(Debug, Clone)]
pub enum OutputReport {
    ScreenSpec(ScreenSpec),
    VariableUpdate(Vec<VariableEntry>),
}

/// Content type for a display main area in a ScreenSpec.
#[derive(Debug, Clone)]
pub enum DisplayContent {
    TextLines { top_margin: u8, lines: Vec<CString> },
    Menu { top_margin: u8, title: CString, items: Vec<MenuItem> },
}

/// A single menu item in a ScreenSpec menu.
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub item_id: u8,
    pub enabled: bool,
    pub label: CString,
}

/// Encoder label pair.
#[derive(Debug, Clone)]
pub struct EncoderLabel {
    pub primary: CString,
    pub secondary: CString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenAnimation {
    None,
    OverLeft,
    OverRight,
    OverTop,
    OverBottom,
    MoveLeft,
    MoveRight,
    MoveTop,
    MoveBottom,
    FadeIn,
    FadeOut,
    OutLeft,
    OutRight,
    OutTop,
    OutBottom,
}

impl From<ScreenAnimation> for u8 {
    fn from(value: ScreenAnimation) -> Self {
        match value {
            ScreenAnimation::None => 0,
            ScreenAnimation::OverLeft => 1,
            ScreenAnimation::OverRight => 2,
            ScreenAnimation::OverTop => 3,
            ScreenAnimation::OverBottom => 4,
            ScreenAnimation::MoveLeft => 5,
            ScreenAnimation::MoveRight => 6,
            ScreenAnimation::MoveTop => 7,
            ScreenAnimation::MoveBottom => 8,
            ScreenAnimation::FadeIn => 9,
            ScreenAnimation::FadeOut => 10,
            ScreenAnimation::OutLeft => 11,
            ScreenAnimation::OutRight => 12,
            ScreenAnimation::OutTop => 13,
            ScreenAnimation::OutBottom => 14,
        }
    }
}

/// A complete screen specification sent to the handheld controller.
#[derive(Debug, Clone)]
pub struct ScreenSpec {
    pub screen_id: u8,
    pub encoder_labels: [EncoderLabel; 4],
    pub left_main: DisplayContent,
    pub left_animation_type: ScreenAnimation,
    pub left_animation_duration: u16,
    pub left_animation_delay: u16,
    pub right_main: DisplayContent,
    pub right_animation_type: ScreenAnimation,
    pub right_animation_duration: u16,
    pub right_animation_delay: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HardwareControl {
    LedRingValue { ring_id: u8, value: u8 },
    Reset,
    SleepLevel { can_deep_sleep: bool },
}

/// VariableUpdate entry types (compact format).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VariableEntry {
    FixedPoint { index: u8, decimals: u8, value: i16 },
    ShortString { index: u8, value: CString },
    HardwareControl(HardwareControl),
}
