//! Sensor providers.
//!
//! `sysinfo` exposes at most a temperature or two on a Mac, because it reads a
//! single SMC key. Tools such as HWMonitor read the SMC directly, which reveals
//! the per-subsystem sensors (CPU proximity, GPU, battery, memory, fans). This
//! module does the same when the platform offers it and returns nothing when it
//! does not, so callers fall back to `sysinfo` instead of showing invented data.
//!
//! Every reading is tagged with the raw SMC key it came from; a value is only
//! reported when the key decodes to a plausible number.

#[derive(Clone, Copy, PartialEq)]
pub enum Unit {
    Celsius,
    Rpm,
}

#[derive(Clone)]
pub struct Sensor {
    /// Raw provider key, shown so an unfamiliar sensor is still traceable.
    pub key: String,
    pub label: String,
    pub value: Option<f32>,
    pub unit: Unit,
}

pub struct Readings {
    pub temps: Vec<Sensor>,
    pub fans: Vec<Sensor>,
}

impl Default for Readings {
    fn default() -> Self {
        Self {
            temps: Vec::new(),
            fans: Vec::new(),
        }
    }
}

pub struct Reader {
    #[cfg(target_os = "macos")]
    inner: mac::Smc,
    #[cfg(target_os = "macos")]
    keys: Vec<KeyEntry>,
}

/// Opens the platform sensor provider, if one is available to an unprivileged app.
pub fn open() -> Option<Reader> {
    #[cfg(target_os = "macos")]
    {
        let mut smc = mac::Smc::open()?;
        // A wrong count means the connection answered but not with SMC data, so
        // refuse to interpret the rest rather than decode garbage later.
        let count = smc.read_number("#KEY")?;
        if !(50.0..=5000.0).contains(&count) {
            return None;
        }
        let keys = enumerate(&mut smc, count as usize);
        if keys.is_empty() {
            return None;
        }
        Some(Reader { inner: smc, keys })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

impl Reader {
    pub fn readings(&mut self) -> Readings {
        let mut out = Readings::default();
        #[cfg(target_os = "macos")]
        {
            for entry in &self.keys {
                let Some(value) = self.inner.read_number(&entry.key) else {
                    continue;
                };
                let plausible = match entry.unit {
                    Unit::Celsius => (-20.0..=150.0).contains(&value),
                    Unit::Rpm => (0.0..=20_000.0).contains(&value),
                };
                if !plausible {
                    continue;
                }
                let sensor = Sensor {
                    key: entry.key.clone(),
                    label: entry.label.clone(),
                    value: Some(value as f32),
                    unit: entry.unit,
                };
                match entry.unit {
                    Unit::Celsius => out.temps.push(sensor),
                    Unit::Rpm => out.fans.push(sensor),
                }
            }
            out.temps.sort_by(|a, b| a.key.cmp(&b.key));
            out.fans.sort_by(|a, b| a.key.cmp(&b.key));
        }
        out
    }
}

#[cfg(target_os = "macos")]
struct KeyEntry {
    key: String,
    label: String,
    unit: Unit,
}

/// Reads every key the SMC exposes and keeps the temperature and fan ones.
#[cfg(target_os = "macos")]
fn enumerate(smc: &mut mac::Smc, count: usize) -> Vec<KeyEntry> {
    let mut found = Vec::new();
    // Older machines expose ~400 keys; the cap keeps a bad response from stalling startup.
    for index in 0..count.min(2000) {
        let Some(key) = smc.key_at(index) else {
            continue;
        };
        let Some((kind, size)) = smc.key_info(&key) else {
            continue;
        };
        let unit = match (kind.as_str(), key.as_str()) {
            ("sp78", _) if key.starts_with('T') => Unit::Celsius,
            ("fpe2", _) if is_fan_key(&key) => Unit::Rpm,
            // Battery and power sensors sometimes decode as plain floats.
            ("flt ", _) if key.starts_with('T') => Unit::Celsius,
            _ => continue,
        };
        if size != 2 && !(kind == "flt " && size == 4) {
            continue;
        }
        found.push(KeyEntry {
            label: label_for(&key, unit),
            key,
            unit,
        });
    }
    found
}

#[cfg(target_os = "macos")]
fn is_fan_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.len() == 4
        && bytes[0] == b'F'
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'A'
        && bytes[3] == b'c'
}

/// SMC keys are four characters: the first two identify the subsystem.
pub fn label_for(key: &str, unit: Unit) -> String {
    if unit == Unit::Rpm {
        let number = key.as_bytes().get(1).copied().unwrap_or(b'0');
        return format!("Fan {}", number.saturating_sub(b'0'));
    }
    let bytes = key.as_bytes();
    let prefix = if bytes.len() >= 2 {
        &key[..2]
    } else {
        return key.to_owned();
    };
    let area = match prefix {
        "TC" => "CPU",
        "TG" => "GPU",
        "TB" => "Battery",
        "TA" => "Ambient",
        "Tm" => "Memory",
        "TN" => "Northbridge",
        "TH" => "Heatsink",
        "Ts" => "Palm rest",
        "Tp" => "Power supply",
        "TW" => "Wireless",
        "TI" => "Thunderbolt",
        "Tf" => "Fan",
        "TL" => "Display",
        "TV" => "VRM",
        _ => return key.to_owned(),
    };
    // Last character distinguishes neighbouring sensors in the same area.
    let suffix = key.chars().last().unwrap_or('0');
    format!("{area} ({suffix})")
}

#[cfg(target_os = "macos")]
mod mac {
    use std::os::raw::{c_char, c_int, c_void};

    const KERNEL_INDEX_SMC: u32 = 2;
    const CMD_READ_BYTES: u8 = 5;
    const CMD_READ_INDEX: u8 = 8;
    const CMD_READ_KEYINFO: u8 = 9;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Vers {
        major: u8,
        minor: u8,
        build: u8,
        reserved: u8,
        release: u16,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct PLimit {
        version: u16,
        length: u16,
        cpu_plimit: u32,
        gpu_plimit: u32,
        mem_plimit: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct KeyInfo {
        data_size: u32,
        data_type: u32,
        data_attributes: u8,
    }
    /// Mirrors `SMCKeyData_t` from Apple's open-source `AppleSMC` client code.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct KeyData {
        key: u32,
        vers: Vers,
        p_limit: PLimit,
        key_info: KeyInfo,
        result: u8,
        status: u8,
        data8: u8,
        data32: u32,
        bytes: [u8; 32],
    }

    const STRUCT_SIZE: usize = std::mem::size_of::<KeyData>();

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
        fn IOServiceOpen(service: u32, owning_task: u32, kind: u32, connection: *mut u32) -> c_int;
        fn IOServiceClose(connection: u32) -> c_int;
        fn IOConnectCallStructMethod(
            connection: u32,
            selector: u32,
            input: *const c_void,
            input_size: usize,
            output: *mut c_void,
            output_size: *mut usize,
        ) -> c_int;
        fn mach_task_self() -> u32;
    }

    pub struct Smc {
        connection: u32,
    }

    impl Smc {
        pub fn open() -> Option<Smc> {
            unsafe {
                let matching = IOServiceMatching(b"AppleSMC\0".as_ptr() as *const c_char);
                if matching.is_null() {
                    return None;
                }
                let service = IOServiceGetMatchingService(0, matching);
                if service == 0 {
                    return None;
                }
                let mut connection = 0u32;
                if IOServiceOpen(service, mach_task_self(), 0, &mut connection) != 0 {
                    return None;
                }
                Some(Smc { connection })
            }
        }

        fn call(&self, input: KeyData) -> Option<KeyData> {
            let mut output = KeyData::default();
            let mut size = STRUCT_SIZE;
            let result = unsafe {
                IOConnectCallStructMethod(
                    self.connection,
                    KERNEL_INDEX_SMC,
                    &input as *const KeyData as *const c_void,
                    STRUCT_SIZE,
                    &mut output as *mut KeyData as *mut c_void,
                    &mut size,
                )
            };
            (result == 0).then_some(output)
        }

        /// Raw key at an index, used for the one-time enumeration.
        pub fn key_at(&mut self, index: usize) -> Option<String> {
            let mut input = KeyData::default();
            input.data8 = CMD_READ_INDEX;
            input.data32 = index as u32;
            let output = self.call(input)?;
            Some(fourcc_to_string(output.key))
        }

        /// Value type and payload size for a key, before reading its bytes.
        pub fn key_info(&mut self, key: &str) -> Option<(String, u32)> {
            let code = fourcc(key)?;
            let mut input = KeyData::default();
            input.key = code;
            input.data8 = CMD_READ_KEYINFO;
            let output = self.call(input)?;
            Some((
                fourcc_to_string(output.key_info.data_type),
                output.key_info.data_size,
            ))
        }

        pub fn read_number(&mut self, key: &str) -> Option<f64> {
            let code = fourcc(key)?;
            let mut info = KeyData::default();
            info.key = code;
            info.data8 = CMD_READ_KEYINFO;
            let info = self.call(info)?;
            let size = info.key_info.data_size as usize;
            if size == 0 || size > 32 {
                return None;
            }
            let mut read = KeyData::default();
            read.key = code;
            read.key_info.data_size = size as u32;
            read.data8 = CMD_READ_BYTES;
            let read = self.call(read)?;
            decode(
                &read.bytes[..size],
                &fourcc_to_string(info.key_info.data_type),
            )
        }
    }

    impl Drop for Smc {
        fn drop(&mut self) {
            unsafe {
                IOServiceClose(self.connection);
            }
        }
    }

    fn fourcc(key: &str) -> Option<u32> {
        let bytes = key.as_bytes();
        (bytes.len() == 4).then(|| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn fourcc_to_string(code: u32) -> String {
        String::from_utf8_lossy(&code.to_be_bytes()).into_owned()
    }

    fn decode(bytes: &[u8], kind: &str) -> Option<f64> {
        match (kind, bytes.len()) {
            // Signed fixed point 7.8, the format used by temperature keys.
            ("sp78", 2) => Some(i16::from_be_bytes([bytes[0], bytes[1]]) as f64 / 256.0),
            // Unsigned fixed point 14.2, the format used by fan RPM keys.
            ("fpe2", 2) => Some(u16::from_be_bytes([bytes[0], bytes[1]]) as f64 / 4.0),
            ("flt ", 4) => {
                Some(f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64)
            }
            ("ui8 ", 1) => Some(bytes[0] as f64),
            ("ui16", 2) => Some(u16::from_be_bytes([bytes[0], bytes[1]]) as f64),
            ("ui32", 4) => {
                Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64)
            }
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn fixed_point_values_decode() {
            // 73.0 °C is 0x4900 in sp78.
            assert_eq!(decode(&[0x49, 0x00], "sp78"), Some(73.0));
            // 1200 rpm is 0x12C0 in fpe2.
            assert_eq!(decode(&[0x12, 0xC0], "fpe2"), Some(1200.0));
            assert_eq!(decode(&[0x00, 0x00], "sp78"), Some(0.0));
            assert_eq!(decode(&[0x01], "ui8 "), Some(1.0));
            assert_eq!(decode(&[0x01, 0x02], "unknown"), None);
            assert_eq!(STRUCT_SIZE, 80, "AppleSMC expects an 80-byte key struct");
        }

        #[test]
        fn keys_round_trip_through_fourcc() {
            assert_eq!(fourcc_to_string(fourcc("TC0P").unwrap()), "TC0P");
            assert_eq!(fourcc_to_string(fourcc("#KEY").unwrap()), "#KEY");
            assert!(fourcc("toolong").is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_the_subsystem() {
        assert_eq!(label_for("TC0P", Unit::Celsius), "CPU (P)");
        assert_eq!(label_for("TG0D", Unit::Celsius), "GPU (D)");
        assert_eq!(label_for("TB0T", Unit::Celsius), "Battery (T)");
        assert_eq!(label_for("F0Ac", Unit::Rpm), "Fan 0");
        assert_eq!(label_for("F1Ac", Unit::Rpm), "Fan 1");
        assert_eq!(label_for("ZZ1Z", Unit::Celsius), "ZZ1Z");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn smc_provider_reports_plausible_readings() {
        // This machine may restrict SMC access; absence is allowed, nonsense is not.
        if let Some(mut reader) = open() {
            let readings = reader.readings();
            println!(
                "smc: {} temperature sensors, {} fans",
                readings.temps.len(),
                readings.fans.len()
            );
            for sensor in readings.temps.iter().take(8) {
                println!("  {} {} = {:?}", sensor.key, sensor.label, sensor.value);
            }
            assert!(readings.temps.iter().all(|s| {
                s.value
                    .map(|v| (-20.0..=150.0).contains(&v))
                    .unwrap_or(true)
            }));
        }
    }
}
