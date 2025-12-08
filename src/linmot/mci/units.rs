use crate::reader::{Reader, WireRead};
use crate::writer::{WireWrite, Writer};
use anyhow::Result;
use core::fmt;
use std::ops;

macro_rules! impl_std_ops {
    ($type:ty) => {
        impl ops::Neg for $type {
            type Output = Self;

            fn neg(self) -> Self {
                Self(-self.0)
            }
        }

        impl ops::Add for $type {
            type Output = Self;

            fn add(self, rhs: Self) -> Self {
                Self(self.0 + rhs.0)
            }
        }

        impl ops::Sub for $type {
            type Output = Self;

            fn sub(self, rhs: Self) -> Self {
                Self(self.0 - rhs.0)
            }
        }

        impl ops::AddAssign for $type {
            fn add_assign(&mut self, rhs: Self) {
                self.0 += rhs.0;
            }
        }

        impl ops::SubAssign for $type {
            fn sub_assign(&mut self, rhs: Self) {
                self.0 -= rhs.0;
            }
        }
    };
}

/// Position in units of 0.1 μm
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position(pub i32);

impl Position {
    #[must_use]
    pub const fn from_millimeters(mm: i32) -> Self {
        Self(mm * 10_000)
    }

    #[must_use]
    pub const fn from_millimeters_f64(mm: f64) -> Self {
        Self((mm * 10_000f64) as i32)
    }
}

impl WireRead for Position {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok(Self(i32::read_from(r)?))
    }
}

impl WireWrite for Position {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        self.0.write_to(w)
    }
}

impl fmt::Debug for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let meters = f64::from(self.0) * 1e-7;
        let units = [("m", 1.0), ("mm", 1e-3), ("μm", 1e-6)];
        fmt_scaled(f, meters, &units)
    }
}

impl_std_ops!(Position);

/// Velocity in units of 1e-6 m/s (1 μm/s)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Velocity(pub i32);

impl Velocity {
    #[must_use]
    pub const fn from_millimeters_per_second(mm_per_s: i32) -> Self {
        Self(mm_per_s * 10_000)
    }

    #[must_use]
    pub const fn from_millimeters_per_second_f64(mm_per_s: f64) -> Self {
        Self((mm_per_s * 10_000f64) as i32)
    }

    #[must_use]
    pub const fn from_meters_per_second(m_per_s: i32) -> Self {
        Self(m_per_s * 1_000_000)
    }

    #[must_use]
    pub const fn from_meters_per_second_f64(m_per_s: f64) -> Self {
        Self((m_per_s * 1_000_000f64) as i32)
    }
}

impl WireRead for Velocity {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok(Self(i32::read_from(r)?))
    }
}

impl WireWrite for Velocity {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        self.0.write_to(w)
    }
}

impl fmt::Debug for Velocity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mps = f64::from(self.0) * 1e-6;
        let units = [("m/s", 1.0), ("mm/s", 1e-3), ("μm/s", 1e-6)];
        fmt_scaled(f, mps, &units)
    }
}

impl_std_ops!(Velocity);

/// Acceleration in units of 1e-5 m/s^2 (10 μm/s^2)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Acceleration(pub i32);

impl Acceleration {
    #[must_use]
    pub const fn from_meters_per_second_squared(m_per_s2: i32) -> Self {
        Self(m_per_s2 * 100_000)
    }

    #[must_use]
    pub const fn from_meters_per_second_squared_f64(m_per_s2: f64) -> Self {
        Self((m_per_s2 * 100_000f64) as i32)
    }

    #[must_use]
    pub const fn from_millimeters_per_second_squared(mm_per_s2: i32) -> Self {
        Self(mm_per_s2 * 100)
    }

    #[must_use]
    pub const fn from_millimeters_per_second_squared_f64(mm_per_s2: f64) -> Self {
        Self((mm_per_s2 * 100f64) as i32)
    }
}

impl WireRead for Acceleration {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok(Self(i32::read_from(r)?))
    }
}

impl WireWrite for Acceleration {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        self.0.write_to(w)
    }
}

impl fmt::Debug for Acceleration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mps2 = f64::from(self.0) * 1e-5;
        let units = [("m/s²", 1.0), ("mm/s²", 1e-3), ("μm/s²", 1e-6)];
        fmt_scaled(f, mps2, &units)
    }
}

impl_std_ops!(Acceleration);

/// Jerk in units of 1e-4 m/s^3 (100 μm/s^3)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Jerk(pub i32);

impl Jerk {
    #[must_use]
    pub const fn from_meters_per_second_cubed(m_per_s3: i32) -> Self {
        Self(m_per_s3 * 10_000)
    }

    #[must_use]
    pub const fn from_meters_per_second_cubed_f64(m_per_s3: f64) -> Self {
        Self((m_per_s3 * 10_000f64) as i32)
    }

    #[must_use]
    pub const fn from_millimeters_per_second_cubed(mm_per_s3: i32) -> Self {
        Self(mm_per_s3 * 10)
    }

    #[must_use]
    pub const fn from_millimeters_per_second_cubed_f64(mm_per_s3: f64) -> Self {
        Self((mm_per_s3 * 10f64) as i32)
    }
}

impl WireWrite for Jerk {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        self.0.write_to(w)
    }
}

impl fmt::Debug for Jerk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mps3 = f64::from(self.0) * 1e-4;
        let units = [("m/s³", 1.0), ("mm/s³", 1e-3), ("μm/s³", 1e-6)];
        fmt_scaled(f, mps3, &units)
    }
}

impl_std_ops!(Jerk);

/// Current in units of 1 mA
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Current(pub i16);

impl WireRead for Current {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok(Self(i16::read_from(r)?))
    }
}

impl fmt::Debug for Current {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // native: 1 mA
        let ma = f64::from(self.0);
        // Here we pass value in mA and let scaling map to A or mA
        fmt_scaled(f, ma, &[("A", 1000.0), ("mA", 1.0)])
    }
}

impl_std_ops!(Current);

fn fmt_scaled(f: &mut fmt::Formatter<'_>, value: f64, units: &[(&str, f64)]) -> fmt::Result {
    // Pick the first unit whose scaled absolute value is >= 1, or the last unit.
    let abs = value.abs();

    let mut chosen = units.last().copied().unwrap();
    for &(u, scale) in units {
        let scaled = abs / scale;
        if scaled >= 1.0 {
            chosen = (u, scale);
            break;
        }
    }

    let v = value / chosen.1;

    // Show up to 3 decimals, trim trailing zeros.
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    write!(f, "{s}{}", chosen.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only utility to exercise fmt_scaled directly via a Debug implementation
    struct __FmtProbe<'a> {
        pub value: f64,
        pub units: &'a [(&'a str, f64)],
    }

    impl fmt::Debug for __FmtProbe<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt_scaled(f, self.value, self.units)
        }
    }

    #[test]
    fn test_position_conversions() {
        assert_eq!(Position::from_millimeters(100), Position(1_000_000));
        assert_eq!(Position::from_millimeters_f64(0.1), Position(1_000));
    }

    #[test]
    fn test_velocity_conversions() {
        assert_eq!(Velocity::from_millimeters_per_second(1), Velocity(10_000));
        assert_eq!(Velocity::from_millimeters_per_second_f64(0.1), Velocity(1_000));
        assert_eq!(Velocity::from_meters_per_second(1), Velocity(1_000_000));
        assert_eq!(Velocity::from_meters_per_second_f64(0.5), Velocity(500_000));
    }

    #[test]
    fn test_acceleration_conversions() {
        assert_eq!(Acceleration::from_meters_per_second_squared(1), Acceleration(100_000));
        assert_eq!(Acceleration::from_meters_per_second_squared_f64(0.5), Acceleration(50_000));
        assert_eq!(Acceleration::from_millimeters_per_second_squared(1), Acceleration(100));
        assert_eq!(Acceleration::from_millimeters_per_second_squared_f64(0.5), Acceleration(50));
    }

    #[test]
    fn test_jerk_conversions() {
        assert_eq!(Jerk::from_meters_per_second_cubed(1), Jerk(10_000));
        assert_eq!(Jerk::from_meters_per_second_cubed_f64(0.25), Jerk(2_500));
        assert_eq!(Jerk::from_millimeters_per_second_cubed(1), Jerk(10));
        assert_eq!(Jerk::from_millimeters_per_second_cubed_f64(0.5), Jerk(5));
    }

    #[test]
    fn test_fmt_scaled_boundaries_and_alternate() {
        let units = [("k", 1000.0), ("u", 1.0)];
        assert_eq!(format!("{:?}", __FmtProbe { value: 0.0, units: &units }), "0u");
        assert_eq!(format!("{:?}", __FmtProbe { value: 999.0, units: &units }), "999u");
        assert_eq!(format!("{:?}", __FmtProbe { value: 1000.0, units: &units }), "1k");
        assert_eq!(format!("{:?}", __FmtProbe { value: 1001.0, units: &units }), "1.001k");
        assert_eq!(format!("{:?}", __FmtProbe { value: -1001.0, units: &units }), "-1.001k");
    }

    #[test]
    fn test_debug_format_position() {
        assert_eq!(format!("{:?}", Position(10_000_000)), "1m");
        assert_eq!(format!("{:?}", Position(10_000)), "1mm");
        assert_eq!(format!("{:?}", Position(10)), "1μm");
        assert_eq!(format!("{:?}", Position(-10_000)), "-1mm");
    }

    #[test]
    fn test_debug_format_velocity() {
        assert_eq!(format!("{:?}", Velocity(1_000_000)), "1m/s");
        assert_eq!(format!("{:?}", Velocity(1000)), "1mm/s");
        assert_eq!(format!("{:?}", Velocity(1)), "1μm/s");
    }

    #[test]
    fn test_debug_format_acceleration() {
        assert_eq!(format!("{:?}", Acceleration(100_000)), "1m/s²");
        assert_eq!(format!("{:?}", Acceleration(100)), "1mm/s²");
        assert_eq!(format!("{:?}", Acceleration(1)), "10μm/s²");
        assert_eq!(format!("{:?}", Acceleration(-100)), "-1mm/s²");
    }

    #[test]
    fn test_debug_format_jerk() {
        assert_eq!(format!("{:?}", Jerk(10_000)), "1m/s³");
        assert_eq!(format!("{:?}", Jerk(10)), "1mm/s³");
        assert_eq!(format!("{:?}", Jerk(1)), "100μm/s³");
        assert_eq!(format!("{:?}", Jerk(-10)), "-1mm/s³");
    }

    #[test]
    fn test_debug_format_current() {
        assert_eq!(format!("{:?}", Current(2500)), "2.5A");
        assert_eq!(format!("{:?}", Current(500)), "500mA");
        assert_eq!(format!("{:?}", Current(-500)), "-500mA");
        assert_eq!(format!("{:?}", Current(1000)), "1A");
    }
}
