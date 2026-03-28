use crate::CoreEvent;
use anyhow::{Result, anyhow};
use hidapi::{HidApi, HidDevice, HidResult};
use log::{debug, error, info, warn};
use std::sync::mpsc;
use std::time::Duration;

mod messages;

use crate::messages::CoreMessage;
use messages::*;

pub struct Controller {}

impl Controller {
    pub fn new(vendor_id: u16, product_id: u16, inbound_sender: mpsc::Sender<CoreEvent>) -> Result<Self> {
        info!("Starting USB HID controller support ({:04x}:{:04x})", vendor_id, product_id);

        // TODO: Implement all the UI state management, split it out from the low-level stuff already here - how?

        std::thread::spawn(move || {
            if let Err(e) = run_loop(vendor_id, product_id) {
                error!("USB HID device loop failed: {}", e);
            }
        });

        Ok(Self {})
    }

    pub fn handle_message(&self, message: CoreMessage) -> Result<()> {
        // TODO
        Ok(())
    }
}

fn run_loop(vendor_id: u16, product_id: u16) -> Result<()> {
    let mut api = HidApi::new()?;

    loop {
        api.reset_devices()?;
        api.add_devices(vendor_id, product_id)?;

        let device_info = api.device_list().find(|d| d.vendor_id() == vendor_id && d.product_id() == product_id);

        let device_info = match device_info {
            Some(d) => d,
            None => {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };

        info!("Found HID device: {:#?}", device_info);

        let device = match device_info.open_device(&api) {
            Ok(d) => d,
            Err(e) => {
                error!("Failed to open HID device: {}", e);
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };

        if let Err(r) = connect_to_device(&device) {
            error!("Error in USB HID device loop: {}", r);
        }
    }
}

const HID_REPORT_LEN: usize = 64;
const SUPPORTED_FIRMWARE_MAJOR: u8 = 0x01;

fn connect_to_device(device: &HidDevice) -> Result<()> {
    let device_info = read_device_info(device)?;

    info!("Device info: {:#?}", device_info);

    if device_info.firmware_version_major != SUPPORTED_FIRMWARE_MAJOR {
        return Err(anyhow!(
            "Unsupported firmware version {}.{}",
            device_info.firmware_version_major,
            device_info.firmware_version_minor
        ));
    }

    let screen_spec = ScreenSpec {
        screen_id: 1,
        encoder_labels: [
            EncoderLabel { primary: c"One".to_owned(), secondary: c"Two".to_owned() },
            EncoderLabel { primary: c"Three".to_owned(), secondary: c"Four".to_owned() },
            EncoderLabel { primary: c"Five".to_owned(), secondary: c"Six".to_owned() },
            EncoderLabel { primary: c"Seven".to_owned(), secondary: c"Eight".to_owned() },
        ],
        left_main: DisplayContent::Menu {
            title: c"Menu Title".to_owned(),
            items: vec![
                MenuItem { item_id: 1, enabled: true, label: c"Item 1".to_owned() },
                MenuItem { item_id: 2, enabled: false, label: c"Item 2".to_owned() },
                MenuItem { item_id: 3, enabled: true, label: c"Item 3".to_owned() },
                MenuItem { item_id: 4, enabled: true, label: c"Item 4".to_owned() },
                MenuItem { item_id: 5, enabled: true, label: c"Item 5".to_owned() },
            ],
        },
        right_main: DisplayContent::TextLines {
            lines: vec![c"Line 1".to_owned(), c"Line 2".to_owned(), c"Line 3".to_owned()],
        },
    };

    send_screen_update(device, screen_spec)?;

    let mut report_buf = [0u8; HID_REPORT_LEN];

    loop {
        let report = read_input_report(device, -1)?;

        debug!("Input report: {:?}", report);
    }
}

// Mockable trait for testing
trait HidDeviceBackend {
    fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize>;
    fn read_timeout(&self, buf: &mut [u8], timeout: i32) -> HidResult<usize>;
    fn write(&self, buf: &[u8]) -> HidResult<usize>;
}

impl HidDeviceBackend for HidDevice {
    fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize> {
        self.get_feature_report(buf)
    }
    fn read_timeout(&self, buf: &mut [u8], timeout: i32) -> HidResult<usize> {
        self.read_timeout(buf, timeout)
    }
    fn write(&self, buf: &[u8]) -> HidResult<usize> {
        self.write(buf)
    }
}

fn read_device_info<D: HidDeviceBackend>(device: &D) -> Result<DeviceInfo> {
    let mut report_buf = [0u8; HID_REPORT_LEN];

    report_buf[0] = 0x01;
    let len = device.get_feature_report(&mut report_buf)?;

    if len < 4 {
        return Err(anyhow!("DeviceInfo feature report too short"));
    }

    Ok(DeviceInfo {
        firmware_version_major: report_buf[1],
        firmware_version_minor: report_buf[2],
        features: report_buf[3],
    })
}

fn send_screen_update<D: HidDeviceBackend>(device: &D, screen_spec: ScreenSpec) -> Result<()> {
    let payload = screen_spec.encode()?;

    const HEADER_LEN: usize = 4;
    const PAYLOAD_PER_FRAGMENT: usize = HID_REPORT_LEN - HEADER_LEN;

    let fragment_count = (payload.len() + PAYLOAD_PER_FRAGMENT - 1) / PAYLOAD_PER_FRAGMENT;
    if fragment_count > u8::MAX as usize {
        return Err(anyhow!("ScreenSpec too large to transmit ({fragment_count} fragments)"));
    }

    for i in 0..fragment_count {
        let mut report_buf = [0u8; HID_REPORT_LEN];
        report_buf[0] = 0x01;
        report_buf[1] = screen_spec.screen_id;
        report_buf[2] = fragment_count as u8;
        report_buf[3] = i as u8;

        let start = i * PAYLOAD_PER_FRAGMENT;
        let end = ((i + 1) * PAYLOAD_PER_FRAGMENT).min(payload.len());
        let chunk = &payload[start..end];
        report_buf[HEADER_LEN..HEADER_LEN + chunk.len()].copy_from_slice(chunk);

        device.write(&report_buf)?;
    }

    Ok(())
}

impl ScreenSpec {
    fn encode(&self) -> Result<Vec<u8>> {
        let mut payload = vec![];

        for label in &self.encoder_labels {
            payload.extend(label.primary.as_bytes_with_nul());
            payload.extend(label.secondary.as_bytes_with_nul());
        }

        for area in [&self.left_main, &self.right_main] {
            match area {
                DisplayContent::TextLines { lines } => {
                    if lines.len() > u8::MAX as usize {
                        return Err(anyhow!("TextLines too long to transmit ({} lines)", lines.len()));
                    }

                    payload.push(0x00);
                    payload.push(lines.len() as u8);
                    for line in lines.iter() {
                        payload.extend(line.as_bytes_with_nul());
                    }
                }
                DisplayContent::Menu { title, items } => {
                    if items.len() > u8::MAX as usize {
                        return Err(anyhow!("Menu too long to transmit ({} items)", items.len()));
                    }

                    payload.push(0x01);
                    payload.extend(title.as_bytes_with_nul());
                    payload.push(items.len() as u8);
                    for item in items.iter() {
                        if item.item_id > 63 {
                            return Err(anyhow!("Item ID out of range ({} > 63)", item.item_id));
                        }

                        payload.push(item.item_id);
                        payload.push(item.enabled as u8);
                        payload.extend(item.label.as_bytes_with_nul());
                    }
                }
            }
        }

        Ok(payload)
    }
}

fn send_variable_updates<D: HidDeviceBackend>(device: &D, variables: &[VariableEntry]) -> Result<()> {
    const HEADER_LEN: usize = 2; // report_id + count
    const MAX_PAYLOAD: usize = HID_REPORT_LEN - HEADER_LEN;

    let mut report_buf = [0u8; HID_REPORT_LEN];
    let mut offset = HEADER_LEN;
    let mut count: u8 = 0;

    for entry in variables {
        let entry_size = entry.get_encoded_size();

        if entry_size > MAX_PAYLOAD {
            return Err(anyhow!(
                "VariableEntry too large to fit in a single report ({entry_size} bytes, max {MAX_PAYLOAD})"
            ));
        }

        // Flush current report if this entry won't fit
        if offset + entry_size > HID_REPORT_LEN && count > 0 {
            report_buf[0] = 0x02;
            report_buf[1] = count;
            device.write(&report_buf)?;

            report_buf = [0u8; HID_REPORT_LEN];
            offset = HEADER_LEN;
            count = 0;
        }

        entry.encode(&mut report_buf[offset..])?;

        offset += entry_size;
        count += 1;
    }

    // Flush remaining
    if count > 0 {
        report_buf[0] = 0x02;
        report_buf[1] = count;
        device.write(&report_buf)?;
    }

    Ok(())
}

impl VariableEntry {
    fn get_encoded_size(&self) -> usize {
        match self {
            VariableEntry::FixedPoint { .. } => 4, // tag + decimals + i16
            VariableEntry::ShortString { value, .. } => 1 + value.as_bytes_with_nul().len(), // tag + string + nul
            VariableEntry::HardwareControl(..) => 2, // tag + value
        }
    }

    fn encode(&self, buf: &mut [u8]) -> Result<()> {
        match self {
            VariableEntry::FixedPoint { index, decimals, value } => {
                if *index > 31 {
                    return Err(anyhow!("Invalid variable index {}", index));
                }

                buf[0] = (0b00 << 5) | (index & 0x1F);
                buf[1] = *decimals;
                buf[2..4].copy_from_slice(&value.to_le_bytes());
            }
            VariableEntry::ShortString { index, value } => {
                if *index > 31 {
                    return Err(anyhow!("Invalid variable index {}", index));
                }

                buf[0] = (0b01 << 5) | (index & 0x1F);
                let bytes = value.as_bytes_with_nul();
                buf[1..1 + bytes.len()].copy_from_slice(bytes);
            }
            VariableEntry::HardwareControl(hw) => {
                let (index, value) = match hw {
                    HardwareControl::SleepLevel { can_deep_sleep } => (0u8, *can_deep_sleep as u8),
                    HardwareControl::LedRingValue { ring_id, value } => {
                        if *ring_id >= 4 {
                            return Err(anyhow!("Invalid ring ID {}", ring_id));
                        }

                        (1u8 + *ring_id, *value)
                    }
                };

                buf[0] = (0b11 << 5) | (index & 0x1F);
                buf[1] = value;
            }
        }

        Ok(())
    }
}

/// timeout in milliseconds, 0 for non-blocking, -1 for infinite
fn read_input_report<D: HidDeviceBackend>(device: &D, timeout: i32) -> Result<Option<InputReport>> {
    let mut report_buf = [0u8; HID_REPORT_LEN];
    let len = device.read_timeout(&mut report_buf, timeout)?;
    if len == 0 {
        return Ok(None);
    }

    // The protocol only defines a single input report ID currently, ignore anything else
    if report_buf[0] != 0x01 {
        warn!("Ignoring unexpected report ID {}", report_buf[0]);

        return Ok(None);
    }

    if len < 7 {
        return Err(anyhow!("InputReport too short"));
    }

    let event_count = report_buf[6] as usize;
    if (event_count * 2) > (len - 7) {
        return Err(anyhow!("InputReport has invalid event size"));
    }

    let mut events = Vec::with_capacity(event_count);
    for i in 0..event_count {
        let offset = 7 + i * 2;
        let event_type = report_buf[offset];
        let event_data = report_buf[offset + 1];

        if let Some(event) = HidInputEvent::from_raw(event_type, event_data) {
            events.push(event);
        } else {
            warn!("Ignoring unknown input event type {}", event_type);
        }
    }

    Ok(Some(InputReport {
        active_screen_id: report_buf[1],
        encoder_deltas: [report_buf[2] as i8, report_buf[3] as i8, report_buf[4] as i8, report_buf[5] as i8],
        events,
    }))
}

impl HidInputEvent {
    fn from_raw(event_type: u8, event_data: u8) -> Option<Self> {
        match event_type {
            0x00 => Some(HidInputEvent::ButtonClick { encoder_id: event_data }),
            0x01 => Some(HidInputEvent::ButtonDoubleClick { encoder_id: event_data }),
            0x02 => Some(HidInputEvent::ButtonHoldStart { encoder_id: event_data }),
            0x03 => Some(HidInputEvent::ButtonHoldEnd { encoder_id: event_data }),
            0x04 => Some(HidInputEvent::MenuSelect { display_id: event_data >> 6, item_id: event_data & 0x3F }),
            0x05 => Some(HidInputEvent::MenuCancel { display_id: event_data >> 6 }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    struct MockHidDevice {
        input_feature_reports: RefCell<VecDeque<Vec<u8>>>,
        output_reports: RefCell<Vec<Vec<u8>>>,
        input_reports: RefCell<VecDeque<Vec<u8>>>,
    }

    impl MockHidDevice {
        fn new() -> Self {
            Self {
                input_feature_reports: RefCell::new(VecDeque::new()),
                input_reports: RefCell::new(VecDeque::new()),
                output_reports: RefCell::new(Vec::new()),
            }
        }

        fn add_input_feature_report(&self, report: Vec<u8>) {
            self.input_feature_reports.borrow_mut().push_back(report);
        }

        fn add_input_report(&self, report: Vec<u8>) {
            self.input_reports.borrow_mut().push_back(report);
        }

        fn get_output_reports(&self) -> Vec<Vec<u8>> {
            self.output_reports.borrow().clone()
        }
    }

    impl HidDeviceBackend for MockHidDevice {
        fn get_feature_report(&self, buf: &mut [u8]) -> HidResult<usize> {
            if let Some(report) = self.input_feature_reports.borrow_mut().pop_front() {
                let len = report.len().min(buf.len());
                buf[..len].copy_from_slice(&report[..len]);
                Ok(len)
            } else {
                Ok(0)
            }
        }

        fn read_timeout(&self, buf: &mut [u8], _timeout: i32) -> HidResult<usize> {
            if let Some(report) = self.input_reports.borrow_mut().pop_front() {
                let len = report.len().min(buf.len());
                buf[..len].copy_from_slice(&report[..len]);
                Ok(len)
            } else {
                Ok(0)
            }
        }

        fn write(&self, buf: &[u8]) -> HidResult<usize> {
            if buf.len() != HID_REPORT_LEN {
                return Err(hidapi::HidError::HidApiError { message: "Invalid report length".to_string() });
            }
            self.output_reports.borrow_mut().push(buf.to_vec());
            Ok(buf.len())
        }
    }

    #[test]
    fn test_read_device_info_success() {
        let mock = MockHidDevice::new();
        // Report ID 1, Major 1, Minor 2, Features 0xAA, padded to 64
        let mut report = vec![0x01, 0x01, 0x02, 0xAA];
        report.resize(HID_REPORT_LEN, 0);
        mock.add_input_feature_report(report);

        let info = read_device_info(&mock).unwrap();
        assert_eq!(info.firmware_version_major, 1);
        assert_eq!(info.firmware_version_minor, 2);
        assert_eq!(info.features, 0xAA);
    }

    #[test]
    fn test_read_device_info_padded() {
        let mock = MockHidDevice::new();
        // Report ID 1, Major 1, Minor 2, Features 0xAA, followed by some garbage/padding
        let mut report = vec![0u8; 64];
        report[0] = 0x01;
        report[1] = 1;
        report[2] = 2;
        report[3] = 0xAA;
        report[4] = 0xFF; // Padding
        mock.add_input_feature_report(report);

        let info = read_device_info(&mock).unwrap();
        assert_eq!(info.firmware_version_major, 1);
        assert_eq!(info.firmware_version_minor, 2);
        assert_eq!(info.features, 0xAA);
    }

    #[test]
    fn test_read_device_info_too_short() {
        let mock = MockHidDevice::new();
        // ID 1, Major 1, Minor 1, 3 bytes total (too short)
        mock.add_input_feature_report(vec![0x01, 0x01, 0x01]);

        let result = read_device_info(&mock);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "DeviceInfo feature report too short");
    }

    #[test]
    fn test_send_screen_update_fragmentation() {
        let mock = MockHidDevice::new();

        let spec = ScreenSpec {
            screen_id: 1,
            encoder_labels: [
                EncoderLabel { primary: c"One".to_owned(), secondary: c"Two".to_owned() },
                EncoderLabel { primary: c"Three".to_owned(), secondary: c"Four".to_owned() },
                EncoderLabel { primary: c"Five".to_owned(), secondary: c"Six".to_owned() },
                EncoderLabel { primary: c"Seven".to_owned(), secondary: c"Eight".to_owned() },
            ],
            left_main: DisplayContent::Menu {
                title: c"Menu Title".to_owned(),
                items: vec![
                    MenuItem { item_id: 1, enabled: true, label: c"Item 1".to_owned() },
                    MenuItem { item_id: 2, enabled: false, label: c"Item 2".to_owned() },
                    MenuItem { item_id: 3, enabled: true, label: c"Item 3".to_owned() },
                    MenuItem { item_id: 4, enabled: true, label: c"Item 4".to_owned() },
                ],
            },
            right_main: DisplayContent::TextLines {
                lines: vec![c"Line 1".to_owned(), c"Line 2".to_owned(), c"Line 3".to_owned()],
            },
        };

        send_screen_update(&mock, spec).unwrap();

        let reports = mock.get_output_reports();
        assert_eq!(reports.len(), 2);
        assert_eq!(
            reports[0],
            [
                0x01, 0x01, 0x02, 0x00, 0x4F, 0x6E, 0x65, 0x00, 0x54, 0x77, 0x6F, 0x00, 0x54, 0x68, 0x72, 0x65, 0x65,
                0x00, 0x46, 0x6F, 0x75, 0x72, 0x00, 0x46, 0x69, 0x76, 0x65, 0x00, 0x53, 0x69, 0x78, 0x00, 0x53, 0x65,
                0x76, 0x65, 0x6E, 0x00, 0x45, 0x69, 0x67, 0x68, 0x74, 0x00, 0x01, 0x4D, 0x65, 0x6E, 0x75, 0x20, 0x54,
                0x69, 0x74, 0x6C, 0x65, 0x00, 0x04, 0x01, 0x01, 0x49, 0x74, 0x65, 0x6D, 0x20,
            ]
        );
        assert_eq!(
            reports[1],
            [
                0x01, 0x01, 0x02, 0x01, 0x31, 0x00, 0x02, 0x00, 0x49, 0x74, 0x65, 0x6D, 0x20, 0x32, 0x00, 0x03, 0x01,
                0x49, 0x74, 0x65, 0x6D, 0x20, 0x33, 0x00, 0x04, 0x01, 0x49, 0x74, 0x65, 0x6D, 0x20, 0x34, 0x00, 0x00,
                0x03, 0x4C, 0x69, 0x6E, 0x65, 0x20, 0x31, 0x00, 0x4C, 0x69, 0x6E, 0x65, 0x20, 0x32, 0x00, 0x4C, 0x69,
                0x6E, 0x65, 0x20, 0x33, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn test_send_variable_updates() {
        let mock = MockHidDevice::new();
        let vars = vec![
            VariableEntry::FixedPoint { index: 5, decimals: 2, value: 1234 },
            VariableEntry::ShortString { index: 12, value: c"Hello".to_owned() },
            VariableEntry::HardwareControl(HardwareControl::SleepLevel { can_deep_sleep: true }),
            VariableEntry::HardwareControl(HardwareControl::LedRingValue { ring_id: 2, value: 200 }),
        ];

        send_variable_updates(&mock, &vars).unwrap();

        let reports = mock.get_output_reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].len(), HID_REPORT_LEN);
        assert_eq!(
            reports[0],
            [
                0x02, 0x04, 0x05, 0x02, 0xD2, 0x04, 0x2C, 0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x00, 0x60, 0x01, 0x63, 0xC8,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn test_long_variable_updates() {
        let mock = MockHidDevice::new();
        let vars = vec![
            VariableEntry::ShortString { index: 0, value: c"Long string that uses nearly a whole report".to_owned() },
            VariableEntry::ShortString { index: 1, value: c"Long string that uses nearly a whole report".to_owned() },
            VariableEntry::ShortString { index: 2, value: c"Long string that uses nearly a whole report".to_owned() },
        ];

        send_variable_updates(&mock, &vars).unwrap();

        let reports = mock.get_output_reports();
        assert_eq!(reports.len(), 3);
    }

    #[test]
    fn test_too_long_variable_updates() {
        let mock = MockHidDevice::new();
        let vars = vec![VariableEntry::ShortString {
            index: 0,
            value: c"Even longer longer longer string that is longer than a report can store".to_owned(),
        }];

        let result = send_variable_updates(&mock, &vars);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "VariableEntry too large to fit in a single report (73 bytes, max 62)"
        );
    }

    #[test]
    fn test_read_input_report_events() {
        let mock = MockHidDevice::new();
        // ID 1, screen 42, deltas [0,0,0,0], 2 events
        // Event 1: MenuSelect { display_id: 1, item_id: 5 } -> type 0x04, data (1 << 6) | 5 = 0x45
        // Event 2: MenuCancel { display_id: 0 } -> type 0x05, data (0 << 6) = 0x00
        let mut report = vec![0u8; 11];
        report[0] = 0x01;
        report[1] = 42;
        report[6] = 2; // event_count
        report[7] = 0x04; // type MenuSelect
        report[8] = 0x45; // data
        report[9] = 0x05; // type MenuCancel
        report[10] = 0x00; // data

        mock.add_input_report(report);

        let input = read_input_report(&mock, 0).unwrap().unwrap();
        assert_eq!(input.active_screen_id, 42);
        assert_eq!(input.events.len(), 2);

        if let HidInputEvent::MenuSelect { display_id, item_id } = input.events[0] {
            assert_eq!(display_id, 1);
            assert_eq!(item_id, 5);
        } else {
            panic!("Expected MenuSelect event");
        }

        if let HidInputEvent::MenuCancel { display_id } = input.events[1] {
            assert_eq!(display_id, 0);
        } else {
            panic!("Expected MenuCancel event");
        }
    }

    #[test]
    fn test_read_input_report_padded() {
        let mock = MockHidDevice::new();
        // ID 1, screen 7, deltas [1, -2, 3, -4], 1 event, padded to 64
        let mut report = vec![0u8; 64];
        report[0] = 0x01;
        report[1] = 7;
        report[2] = 1;
        report[3] = (-2i8) as u8;
        report[4] = 3;
        report[5] = (-4i8) as u8;
        report[6] = 1; // count
        report[7] = 0; // type ButtonClick
        report[8] = 0; // data (encoder 0)

        mock.add_input_report(report);

        let input = read_input_report(&mock, 0).unwrap().unwrap();
        assert_eq!(input.active_screen_id, 7);
        assert_eq!(input.encoder_deltas, [1, -2, 3, -4]);
        assert_eq!(input.events.len(), 1);
    }

    #[test]
    fn test_read_input_report_too_short() {
        let mock = MockHidDevice::new();
        // Padded to 64, but event_count says 100 which is > (64-7)/2
        let mut report = vec![0u8; 64];
        report[0] = 0x01;
        report[6] = 100;

        mock.add_input_report(report);

        let result = read_input_report(&mock, 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "InputReport has invalid event size");
    }
}
