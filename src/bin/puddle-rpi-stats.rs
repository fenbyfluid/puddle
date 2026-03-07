use questdb::ingress::{Buffer, Sender, TimestampMicros};
use std::io::{Read, Seek};
use std::path::Path;

fn main() {
    let mut sender = Sender::from_env().unwrap();
    let mut buffer = sender.new_buffer();

    let mut hwmon_reader = HwmonReader::new().unwrap();

    let loop_interval = std::time::Duration::from_secs(1);
    let mut next_tick = std::time::Instant::now() + loop_interval;

    loop {
        let timestamp = TimestampMicros::now();

        hwmon_reader.read_sample(timestamp, &mut buffer).unwrap();
        sender.flush(&mut buffer).unwrap();

        let now = std::time::Instant::now();
        if let Some(remaining) = next_tick.checked_duration_since(now) {
            std::thread::sleep(remaining);
            next_tick += loop_interval;
        } else {
            let late_by = now.duration_since(next_tick);
            eprintln!("Late by {late_by:?}");
            next_tick = now + loop_interval;
        }
    }
}

struct HwmonReader {
    read_buffer: String,
    metrics: Vec<Metric>,
}

struct Metric {
    hwmon_name: &'static str,
    file_name: &'static str,
    column_name: &'static str,
    value_kind: MetricValueKind,
    file: Option<std::fs::File>,
}

enum MetricValueKind {
    I64,
    Bool,
}

impl HwmonReader {
    fn new() -> Result<Self, std::io::Error> {
        let mut instance = Self {
            read_buffer: String::new(),
            metrics: vec![
                Metric::i64("cpu_thermal", "temp1_input", "cpu_thermal_temp"),
                Metric::i64("rp1_adc", "in2_input", "rp1_adc_vbus"),
                Metric::i64("rp1_adc", "temp1_input", "rp1_adc_temp"),
                Metric::bool("rpi_volt", "in0_lcrit_alarm", "rpi_volt_alarm"),
                Metric::i64("pwmfan", "pwm1", "fan_pwm"),
                Metric::i64("pwmfan", "fan1_input", "fan_rpm"),
            ],
        };

        for directory in std::fs::read_dir("/sys/class/hwmon/")? {
            let directory = directory?;

            let name = match std::fs::read_to_string(directory.path().join("name")) {
                Ok(name) => name,
                Err(_) => continue,
            };

            let hwmon_name = name.trim();
            for metric in &mut instance.metrics {
                if metric.hwmon_name == hwmon_name {
                    if metric.file.is_some() {
                        eprintln!("Multiple hwmon entries for {}", metric.hwmon_name);
                        continue;
                    }

                    metric.file = Self::open_file(directory.path().join(metric.file_name));
                }
            }
        }

        for metric in &mut instance.metrics {
            if metric.file.is_none() {
                eprintln!("Missing hwmon entry for {}/{}", metric.hwmon_name, metric.file_name);
            }
        }

        Ok(instance)
    }

    fn read_sample(&mut self, timestamp: TimestampMicros, buffer: &mut Buffer) -> Result<(), std::io::Error> {
        buffer.table("rpi_stats_hwmon").unwrap();

        for metric in &mut self.metrics {
            metric.write_column(buffer, &mut self.read_buffer)?;
        }

        buffer.at(timestamp).unwrap();

        Ok(())
    }

    fn open_file<P: AsRef<Path>>(path: P) -> Option<std::fs::File> {
        match std::fs::File::open(&path) {
            Ok(file) => Some(file),
            Err(_) => {
                eprintln!("Failed to open file: {}", path.as_ref().display());
                None
            }
        }
    }
}

impl Metric {
    const fn i64(hwmon_name: &'static str, file_name: &'static str, column_name: &'static str) -> Self {
        Self { hwmon_name, file_name, column_name, value_kind: MetricValueKind::I64, file: None }
    }

    const fn bool(hwmon_name: &'static str, file_name: &'static str, column_name: &'static str) -> Self {
        Self { hwmon_name, file_name, column_name, value_kind: MetricValueKind::Bool, file: None }
    }

    fn write_column(&mut self, buffer: &mut Buffer, read_buffer: &mut String) -> Result<(), std::io::Error> {
        if let Some(file) = self.file.as_mut() {
            let value = Self::read_file_i64(read_buffer, file)?;

            match self.value_kind {
                MetricValueKind::I64 => {
                    buffer.column_i64(self.column_name, value).unwrap();
                }
                MetricValueKind::Bool => {
                    buffer.column_bool(self.column_name, value != 0).unwrap();
                }
            }
        }

        Ok(())
    }

    fn read_file_i64(buffer: &mut String, file: &mut std::fs::File) -> Result<i64, std::io::Error> {
        buffer.clear();
        file.seek(std::io::SeekFrom::Start(0))?;
        file.read_to_string(buffer)?;

        buffer
            .trim()
            .parse::<i64>()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Failed to parse i64"))
    }
}
