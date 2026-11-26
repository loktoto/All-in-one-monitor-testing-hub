//! Sensor providers.
//!
//! - **macOS**: Reads AppleSMC directly via IOKit FFI (Intel + Apple Silicon key
//!   sets), revealing per-subsystem temperatures and `F<n>Ac` fan speeds.
//! - **Windows**: Reads ACPI thermal zones from the standard-user `Thermal Zone
//!   Information` performance counters (`ROOT\CIMV2`) and elevated `ROOT\WMI`
//!   `MSAcpi_ThermalZoneTemperature` when available.
//! - **Linux**: Reads `/sys/class/hwmon` directly without root privileges
//!   (`coretemp`, `k10temp`, `zenpower`, `amdgpu`, `nvme`, `acpitz`, fan RPM).
//!
//! Every reading is tagged with its raw provider key and provenance source; a
//! value is only reported when it decodes to a plausible number.

#[cfg(any(target_os = "windows", test))]
pub(crate) mod lhm;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExternalThermals {
    #[default]
    Disabled,
    Unavailable,
    Connected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Celsius,
    Rpm,
}

#[derive(Clone, Debug)]
pub struct Sensor {
    /// Raw provider key, shown so an unfamiliar sensor is still traceable.
    pub key: String,
    pub label: String,
    /// Short provenance tag (e.g. `SMC`, `ACPI`, `hwmon`, `sysinfo`).
    pub source: &'static str,
    pub value: Option<f32>,
    pub unit: Unit,
    /// Identified as a core by the provider's hardware channel identity.
    pub cpu_core: bool,
}

#[derive(Default, Debug)]
pub struct Readings {
    pub temps: Vec<Sensor>,
    pub fans: Vec<Sensor>,
    #[cfg(target_os = "linux")]
    core_temps: std::collections::HashMap<(u32, u32), Option<f32>>,
}

pub struct Reader {
    #[cfg(target_os = "macos")]
    inner: mac::Smc,
    #[cfg(target_os = "macos")]
    keys: Vec<KeyEntry>,
    #[cfg(target_os = "windows")]
    inner: win::Wmi,
    #[cfg(target_os = "linux")]
    inner: linux::Hwmon,
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
    #[cfg(target_os = "windows")]
    {
        win::Wmi::open().map(|inner| Reader { inner })
    }
    #[cfg(target_os = "linux")]
    {
        linux::Hwmon::open().map(|inner| Reader { inner })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

impl Reader {
    pub fn core_temperatures(
        &self,
        readings: &Readings,
        cpus: &[sysinfo::Cpu],
    ) -> Vec<Option<f32>> {
        #[cfg(target_os = "linux")]
        {
            self.inner.core_temperatures(readings, cpus)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (readings, cpus);
            Vec::new()
        }
    }

    pub fn readings(&mut self) -> Readings {
        #[cfg(target_os = "windows")]
        {
            self.inner.readings()
        }
        #[cfg(target_os = "linux")]
        {
            self.inner.readings()
        }
        #[cfg(target_os = "macos")]
        {
            let mut out = Readings::default();
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
                    source: "SMC",
                    value: Some(value as f32),
                    unit: entry.unit,
                    cpu_core: entry.unit == Unit::Celsius && apple_core_key(&entry.key),
                };
                match entry.unit {
                    Unit::Celsius => out.temps.push(sensor),
                    Unit::Rpm => out.fans.push(sensor),
                }
            }
            out.temps.sort_by(|a, b| a.key.cmp(&b.key));
            out.fans.sort_by(|a, b| a.key.cmp(&b.key));
            out
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Readings::default()
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn apple_core_key(key: &str) -> bool {
    matches!(key.as_bytes(), [b'T', b'p' | b'e', a, b] if a.is_ascii_digit() && b.is_ascii_digit())
}

/// `MSAcpi_ThermalZoneTemperature` reports tenths of a degree Kelvin.
#[cfg(any(target_os = "windows", test))]
fn tenths_kelvin_to_celsius(raw: u32) -> f32 {
    raw as f32 / 10.0 - 273.15
}

/// The `Thermal Zone Information` performance counter's plain value is kelvin.
#[cfg(any(target_os = "windows", test))]
fn kelvin_to_celsius(raw: u32) -> f32 {
    raw as f32 - 273.15
}

/// `ACPI\ThermalZone\TZ01_0` is shown as `ACPI zone TZ01`; the raw instance
/// name stays as the tooltip key. Performance-counter instances are named
/// `\_TZ.TZ01`, so both separators are handled.
#[cfg(any(target_os = "windows", test))]
fn thermal_zone_label(instance_name: &str) -> String {
    let leaf = instance_name
        .rsplit(['\\', '.'])
        .next()
        .unwrap_or(instance_name);
    let zone = leaf.split('_').next().unwrap_or(leaf);
    if zone.is_empty() {
        instance_name.to_owned()
    } else {
        format!("ACPI zone {zone}")
    }
}

/// Parse a Linux `hwmon` `temp*_input` value (millidegrees Celsius) into Celsius.
#[cfg(any(target_os = "linux", test))]
fn parse_hwmon_millidegrees(text: &str) -> Option<f32> {
    let milli: i64 = text.trim().parse().ok()?;
    let celsius = milli as f32 / 1000.0;
    (-20.0..=150.0).contains(&celsius).then_some(celsius)
}

/// Parse a Linux `hwmon` `fan*_input` value (RPM).
#[cfg(any(target_os = "linux", test))]
fn parse_hwmon_rpm(text: &str) -> Option<f32> {
    let rpm: f32 = text.trim().parse().ok()?;
    (0.0..=20_000.0).contains(&rpm).then_some(rpm)
}

/// Friendly chip label for common Linux `hwmon` drivers.
#[cfg(any(target_os = "linux", test))]
fn hwmon_chip_label(driver: &str) -> &str {
    match driver.trim() {
        "coretemp" => "Intel CPU",
        "k10temp" | "zenpower" => "AMD CPU",
        "amdgpu" => "AMD GPU",
        "nouveau" => "NVIDIA GPU",
        "nvme" => "NVMe SSD",
        "iwlwifi_1" | "iwlwifi" => "Wi-Fi",
        "acpitz" => "ACPI thermal",
        "BAT0" | "BAT1" => "Battery",
        other => other,
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{
        hwmon_chip_label, parse_hwmon_millidegrees, parse_hwmon_rpm, Readings, Sensor, Unit,
    };
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
    };

    struct HwmonEntry {
        key: String,
        label: String,
        input_path: PathBuf,
        unit: Unit,
        physical_core: Option<(u32, u32)>,
    }

    pub struct Hwmon {
        entries: Vec<HwmonEntry>,
        topology: HashMap<String, (u32, u32)>,
    }

    impl Hwmon {
        pub fn open() -> Option<Hwmon> {
            let root = Path::new("/sys/class/hwmon");
            let mut entries = Vec::new();
            let dirs = fs::read_dir(root).ok()?;
            for dir in dirs.flatten() {
                let path = dir.path();
                let dir_name = dir.file_name().to_string_lossy().into_owned();
                let chip = fs::read_to_string(path.join("name"))
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_else(|_| dir_name.clone());
                let chip_friendly = hwmon_chip_label(&chip).to_owned();
                let package = (chip == "coretemp")
                    .then(|| number_label(&path.join("temp1_label"), "Package id "))
                    .flatten();
                let Ok(files) = fs::read_dir(&path) else {
                    continue;
                };
                for file in files.flatten() {
                    let fname = file.file_name().to_string_lossy().into_owned();
                    if let Some(stem) = fname.strip_suffix("_input") {
                        let unit = if stem.starts_with("temp") {
                            Unit::Celsius
                        } else if stem.starts_with("fan") {
                            Unit::Rpm
                        } else {
                            continue;
                        };
                        let label_path = path.join(format!("{stem}_label"));
                        let sub_label = fs::read_to_string(label_path)
                            .ok()
                            .map(|s| s.trim().to_owned())
                            .filter(|s| !s.is_empty());
                        let physical_core = package.zip(
                            sub_label
                                .as_deref()
                                .and_then(|label| label.strip_prefix("Core "))
                                .and_then(|n| n.parse().ok()),
                        );
                        let label = match sub_label {
                            Some(sub) => format!("{chip_friendly} · {sub}"),
                            None => format!("{chip_friendly} ({stem})"),
                        };
                        entries.push(HwmonEntry {
                            key: format!("{dir_name}:{chip}:{stem}"),
                            label,
                            input_path: file.path(),
                            unit,
                            physical_core,
                        });
                    }
                }
            }
            if entries.is_empty() {
                return None;
            }
            entries.sort_by(|a, b| a.key.cmp(&b.key));
            let topology = if entries.iter().any(|entry| entry.physical_core.is_some()) {
                cpu_topology(Path::new("/sys/devices/system/cpu"))
            } else {
                HashMap::new()
            };
            Some(Hwmon { entries, topology })
        }

        pub fn core_temperatures(
            &self,
            readings: &Readings,
            cpus: &[sysinfo::Cpu],
        ) -> Vec<Option<f32>> {
            if readings.core_temps.is_empty() {
                return Vec::new();
            }
            cpus.iter()
                .map(|cpu| {
                    let ids = self.topology.get(cpu.name())?;
                    readings.core_temps.get(ids).copied().flatten()
                })
                .collect()
        }

        pub fn readings(&mut self) -> Readings {
            let mut out = Readings::default();
            for entry in &self.entries {
                let Ok(raw) = fs::read_to_string(&entry.input_path) else {
                    continue;
                };
                let value = match entry.unit {
                    Unit::Celsius => parse_hwmon_millidegrees(&raw),
                    Unit::Rpm => parse_hwmon_rpm(&raw),
                };
                let Some(value) = value else {
                    continue;
                };
                if let Some(ids) = entry.physical_core {
                    out.core_temps
                        .entry(ids)
                        .and_modify(|value| *value = None)
                        .or_insert(Some(value));
                }
                let sensor = Sensor {
                    key: entry.key.clone(),
                    label: entry.label.clone(),
                    source: "hwmon",
                    value: Some(value),
                    unit: entry.unit,
                    cpu_core: entry.physical_core.is_some(),
                };
                match entry.unit {
                    Unit::Celsius => out.temps.push(sensor),
                    Unit::Rpm => out.fans.push(sensor),
                }
            }
            out
        }
    }

    fn number_label(path: &Path, prefix: &str) -> Option<u32> {
        fs::read_to_string(path)
            .ok()?
            .trim()
            .strip_prefix(prefix)?
            .parse()
            .ok()
    }

    fn cpu_topology(root: &Path) -> HashMap<String, (u32, u32)> {
        let Ok(dirs) = fs::read_dir(root) else {
            return HashMap::new();
        };
        dirs.flatten()
            .filter_map(|dir| {
                let name = dir.file_name().into_string().ok()?;
                name.strip_prefix("cpu")?.parse::<u32>().ok()?;
                let topology = dir.path().join("topology");
                Some((
                    name,
                    (
                        number_label(&topology.join("physical_package_id"), "")?,
                        number_label(&topology.join("core_id"), "")?,
                    ),
                ))
            })
            .collect()
    }
}

/// Windows exposes no per-core CPU temperature to an unprivileged process, but
/// the ACPI thermal zones are readable two ways: the `MSAcpi_ThermalZoneTemperature`
/// WMI class (elevated only) and the `Thermal Zone Information` performance
/// counters, which the OS publishes to standard users. The performance counter
/// is what makes the sensor table useful without administrator rights.
#[cfg(target_os = "windows")]
mod win {
    use super::{
        kelvin_to_celsius, tenths_kelvin_to_celsius, thermal_zone_label, Readings, Sensor, Unit,
    };
    use serde::Deserialize;
    use wmi::WMIConnection;

    #[derive(Deserialize)]
    #[serde(rename = "MSAcpi_ThermalZoneTemperature")]
    #[serde(rename_all = "PascalCase")]
    struct ThermalZone {
        instance_name: String,
        current_temperature: Option<u32>,
    }

    /// `Win32_PerfRawData_Counters_ThermalZoneInformation` in `ROOT\CIMV2`.
    /// `HighPrecisionTemperature` is tenths of a kelvin; `Temperature` is whole
    /// kelvin. Both are firmware thermal-zone readings, not per-core die values.
    #[derive(Deserialize)]
    #[serde(rename = "Win32_PerfRawData_Counters_ThermalZoneInformation")]
    #[serde(rename_all = "PascalCase")]
    struct PerfZone {
        name: String,
        temperature: Option<u32>,
        high_precision_temperature: Option<u32>,
    }

    pub struct Wmi {
        /// `ROOT\WMI`, present only when the ACPI class can actually be read.
        acpi: Option<WMIConnection>,
        /// `ROOT\CIMV2` performance class, readable without elevation.
        perf: Option<WMIConnection>,
    }

    impl Wmi {
        pub fn open() -> Option<Wmi> {
            // Probe once: a denied class fails the query (access denied), so it
            // is dropped here instead of retried on every refresh.
            let acpi = WMIConnection::with_namespace_path("ROOT\\WMI")
                .ok()
                .filter(|c| matches!(c.query::<ThermalZone>(), Ok(z) if !z.is_empty()));
            let perf = WMIConnection::new()
                .ok()
                .filter(|c| matches!(c.query::<PerfZone>(), Ok(z) if !z.is_empty()));
            (acpi.is_some() || perf.is_some()).then_some(Wmi { acpi, perf })
        }

        pub fn readings(&mut self) -> Readings {
            let mut out = Readings::default();
            if let Some(connection) = &self.acpi {
                if let Ok(zones) = connection.query::<ThermalZone>() {
                    for zone in zones {
                        let Some(raw) = zone.current_temperature else {
                            continue;
                        };
                        push_temp(&mut out, zone.instance_name, tenths_kelvin_to_celsius(raw));
                    }
                }
            }
            if out.temps.is_empty() {
                if let Some(connection) = &self.perf {
                    if let Ok(zones) = connection.query::<PerfZone>() {
                        for zone in zones {
                            let celsius = match (zone.high_precision_temperature, zone.temperature)
                            {
                                (Some(tenths), _) if tenths > 0 => tenths_kelvin_to_celsius(tenths),
                                (_, Some(kelvin)) if kelvin > 0 => kelvin_to_celsius(kelvin),
                                _ => continue,
                            };
                            push_temp(&mut out, zone.name, celsius);
                        }
                    }
                }
            }
            out.temps.sort_by(|a, b| a.key.cmp(&b.key));
            out
        }
    }

    fn push_temp(out: &mut Readings, instance_name: String, celsius: f32) {
        if !(-20.0..=150.0).contains(&celsius) {
            return;
        }
        out.temps.push(Sensor {
            label: thermal_zone_label(&instance_name),
            key: instance_name,
            source: "ACPI",
            value: Some(celsius),
            unit: Unit::Celsius,
            cpu_core: false,
        });
    }
}

#[cfg(target_os = "macos")]
struct KeyEntry {
    key: String,
    label: String,
    unit: Unit,
}

/// Reads every key the SMC exposes and keeps the temperature and fan ones.
/// Supports both Intel (`sp78` / `fpe2`) and Apple Silicon (`flt ` / `sp78`)
/// SMC encodings.
#[cfg(target_os = "macos")]
fn enumerate(smc: &mut mac::Smc, count: usize) -> Vec<KeyEntry> {
    let mut found = Vec::new();
    // Older machines expose ~400 keys; Apple Silicon can expose >1000.
    for index in 0..count.min(3000) {
        let Some(key) = smc.key_at(index) else {
            continue;
        };
        let Some((kind, size)) = smc.key_info(&key) else {
            continue;
        };
        let unit = match (kind.as_str(), key.as_str()) {
            ("sp78", _) if key.starts_with('T') => Unit::Celsius,
            ("fpe2", _) if is_fan_key(&key) => Unit::Rpm,
            // Apple Silicon reports temperatures and fans as IEEE-754 floats (`flt `).
            ("flt ", _) if is_fan_key(&key) => Unit::Rpm,
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
/// Covers both Intel Mac (`TC`, `TG`, `TB`, ...) and Apple Silicon SoC
/// prefixes (`Tp` P-core die, `Te` E-core die, `Tg` GPU die, `TR` NAND/SSD).
pub fn label_for(key: &str, unit: Unit) -> String {
    if unit == Unit::Rpm {
        let number = key.as_bytes().get(1).copied().unwrap_or(b'0');
        return format!("Fan {}", number.saturating_sub(b'0'));
    }
    // A firmware key may contain non-ASCII bytes (fourcc is decoded lossily).
    // `&key[..2]` would panic when byte 2 is not a UTF-8 char boundary, so fall
    // back to the whole key — unknown prefixes are returned unchanged below.
    let prefix = match key.get(..2) {
        Some(p) => p,
        None => return key.to_owned(),
    };
    let area = match prefix {
        "TC" => "CPU",
        "Tp" => "CPU P-core",
        "Te" => "CPU E-core",
        "TG" | "Tg" => "GPU",
        "TB" => "Battery",
        "TA" => "Ambient",
        "Tm" | "TM" => "Memory",
        "TN" => "Northbridge",
        "TH" => "Heatsink",
        "TR" => "Storage NAND",
        "Ts" => "Palm rest",
        "TP" => "Power supply",
        "TW" => "Wireless",
        "TI" => "Thunderbolt",
        "Tf" => "Fan",
        "TL" => "Display",
        "TV" => "VRM",
        "TD" => "SoC die",
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
            // Apple Silicon SMC `flt ` values are little-endian IEEE-754 floats.
            ("flt ", 4) => {
                #[cfg(target_arch = "aarch64")]
                let val = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
                #[cfg(not(target_arch = "aarch64"))]
                let val = f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64;
                Some(val)
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
        assert_eq!(label_for("Tp01", Unit::Celsius), "CPU P-core (1)");
        assert_eq!(label_for("Te05", Unit::Celsius), "CPU E-core (5)");
        assert_eq!(label_for("TG0D", Unit::Celsius), "GPU (D)");
        assert_eq!(label_for("Tg05", Unit::Celsius), "GPU (5)");
        assert_eq!(label_for("TB0T", Unit::Celsius), "Battery (T)");
        assert_eq!(label_for("TR0Z", Unit::Celsius), "Storage NAND (Z)");
        assert_eq!(label_for("F0Ac", Unit::Rpm), "Fan 0");
        assert_eq!(label_for("F1Ac", Unit::Rpm), "Fan 1");
        assert_eq!(label_for("ZZ1Z", Unit::Celsius), "ZZ1Z");
    }

    #[test]
    fn apple_core_group_keeps_other_smc_channels_separate() {
        assert!(apple_core_key("Tp01"));
        assert!(apple_core_key("Te05"));
        assert!(!apple_core_key("TC0P"));
        assert!(!apple_core_key("Tp0P"));
        assert!(!apple_core_key("Té05"));
    }

    #[test]
    fn labels_never_panic_on_non_ascii_keys() {
        // A malformed firmware key reaching label_for must not slice a UTF-8
        // byte in half; unknown prefixes fall back to the full key.
        assert_eq!(label_for("€x", Unit::Celsius), "€x");
        assert_eq!(label_for("€TC", Unit::Celsius), "€TC");
        assert_eq!(label_for("é", Unit::Celsius), "é");
    }

    #[test]
    fn linux_hwmon_parsers_decode_and_guard_outliers() {
        assert_eq!(parse_hwmon_millidegrees("54250\n"), Some(54.25));
        assert_eq!(parse_hwmon_millidegrees("250000"), None);
        assert_eq!(parse_hwmon_millidegrees("not-a-number"), None);
        assert_eq!(parse_hwmon_rpm("1850\n"), Some(1850.0));
        assert_eq!(parse_hwmon_rpm("-10"), None);
        assert_eq!(hwmon_chip_label("k10temp"), "AMD CPU");
        assert_eq!(hwmon_chip_label("coretemp"), "Intel CPU");
        assert_eq!(hwmon_chip_label("nvme"), "NVMe SSD");
    }

    #[test]
    fn windows_thermal_zones_are_converted_and_named() {
        // 3331 tenths of a kelvin is 59.95 °C; 3031 is 29.95 °C.
        assert!((tenths_kelvin_to_celsius(3331) - 59.95).abs() < 0.01);
        assert!((tenths_kelvin_to_celsius(3031) - 29.95).abs() < 0.01);
        assert_eq!(
            thermal_zone_label("ACPI\\ThermalZone\\TZ01_0"),
            "ACPI zone TZ01"
        );
        assert_eq!(thermal_zone_label("TZ00"), "ACPI zone TZ00");
        // Performance-counter instances use `\_TZ.TZ01`.
        assert_eq!(thermal_zone_label("\\_TZ.TZ00"), "ACPI zone TZ00");
        assert_eq!(thermal_zone_label("\\_TZ.TZ01"), "ACPI zone TZ01");
        assert!((kelvin_to_celsius(330) - 56.85).abs() < 0.01);
        assert!((tenths_kelvin_to_celsius(3392) - 66.05).abs() < 0.01);
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
