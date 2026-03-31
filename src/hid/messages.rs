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

/// A complete screen specification sent to the handheld controller.
#[derive(Debug, Clone)]
pub struct ScreenSpec {
    pub screen_id: u8,
    pub encoder_labels: [EncoderLabel; 4],
    pub left_main: DisplayContent,
    pub right_main: DisplayContent,
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
