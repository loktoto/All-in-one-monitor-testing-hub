//! Static system inventory: the specification sheet mainstream tools show.
//!
//! CPU-Z shows processor identification, caches, mainboard and memory modules;
//! CrystalDiskInfo shows drive identity and S.M.A.R.T. health; GPU-Z shows the
//! adapter, driver and VRAM. Everything here is read once at startup from local
//! OS sources only, so the inventory is cheap and never leaves the machine.
//!
//! Each field is optional on purpose: a missing value is reported as unavailable
//! instead of being guessed, because the sources differ per platform.

use std::process::Command;

#[derive(Clone, Default)]
pub struct CpuSpec {
    pub name: String,
    pub vendor: String,
    pub architecture: String,
    pub physical_cores: Option<u32>,
    pub logical_cores: Option<u32>,
    /// Apple silicon performance/efficiency split, when the OS exposes it.
    pub performance_cores: Option<u32>,
    pub efficiency_cores: Option<u32>,
    pub base_mhz: Option<u64>,
    pub max_mhz: Option<u64>,
    pub l1_instruction_bytes: Option<u64>,
    pub l1_data_bytes: Option<u64>,
    pub l2_bytes: Option<u64>,
    pub l3_bytes: Option<u64>,
    pub cache_line_bytes: Option<u64>,
}

#[derive(Clone, Default)]
pub struct MemoryModule {
    pub slot: String,
    pub size_bytes: Option<u64>,
    pub kind: String,
    pub speed_mhz: Option<u64>,
    pub manufacturer: String,
    pub part_number: String,
}

#[derive(Clone, Default)]
pub struct MemoryProfile {
    pub total_bytes: Option<u64>,
    pub ecc: Option<bool>,
    pub upgradeable: Option<bool>,
    pub modules: Vec<MemoryModule>,
}

#[derive(Clone, Default)]
pub struct GraphicsSpec {
    pub name: String,
    pub chipset: String,
    pub vendor: String,
    pub device_id: String,
    pub vram_bytes: Option<u64>,
    pub api: String,
    pub bus: String,
}

/// A physical storage device, CrystalDiskInfo style.
#[derive(Clone, Default)]
pub struct DriveSpec {
    pub model: String,
    pub firmware: String,
    pub protocol: String,
    pub link: String,
    pub trim: Option<bool>,
    pub smart: String,
    pub capacity_bytes: Option<u64>,
}

/// Health of one mounted volume, which is what a user can actually point at.
#[derive(Clone, Default)]
pub struct VolumeHealth {
    pub mount: String,
    pub media: String,
    pub protocol: String,
    pub solid_state: Option<bool>,
    pub smart: String,
}

#[derive(Clone, Default)]
pub struct BluetoothSpec {
    pub state: Option<bool>,
    pub transport: String,
    pub chipset: String,
    pub connected_devices: Option<Vec<String>>,
}

#[derive(Clone, Default)]
pub struct SystemSpecs {
    pub os_name: String,
    pub os_version: String,
    pub kernel: String,
    pub model_name: String,
    pub model_id: String,
    pub firmware: String,
    pub board_id: Option<String>,
    pub cpu: CpuSpec,
    pub memory: MemoryProfile,
    pub graphics: Vec<GraphicsSpec>,
    pub drives: Vec<DriveSpec>,
    pub volumes: Vec<VolumeHealth>,
    pub battery: Option<String>,
    pub bluetooth: Option<BluetoothSpec>,
    pub wifi_interface: Option<String>,
    /// Where these values came from, shown so nothing looks authoritative by default.
    pub sources: String,
    pub unavailable: Vec<String>,
}

impl SystemSpecs {
    /// S.M.A.R.T. state for a mount point, when it was reported.
    pub fn volume_health(&self, mount: &str) -> Option<&VolumeHealth> {
        self.volumes.iter().find(|v| v.mount == mount)
    }
}

/// Read the Bluetooth report. Device addresses are discarded; connected names stay in memory.
pub fn bluetooth_status() -> Option<BluetoothSpec> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("system_profiler")
            .args(["-json", "SPBluetoothDataType"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_bluetooth(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub fn collect() -> SystemSpecs {
    #[cfg(target_os = "macos")]
    {
        macos::collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        portable::collect()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use sysinfo::Disks;

    fn run(command: &str, args: &[&str]) -> Option<String> {
        let output = Command::new(command).args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn sysctl(key: &str) -> Option<String> {
        let value = run("sysctl", &["-n", key])?;
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    }

    fn sysctl_num(key: &str) -> Option<u64> {
        sysctl(key)?.parse().ok()
    }

    pub fn collect() -> SystemSpecs {
        let mut specs = SystemSpecs {
            sources: "sysctl · system_profiler · diskutil · pmset".into(),
            ..Default::default()
        };
        specs.os_name = "macOS".into();
        specs.os_version = sysctl("kern.osproductversion").unwrap_or_default();
        specs.kernel = sysctl("kern.osrelease")
            .map(|v| format!("Darwin {v}"))
            .unwrap_or_default();
        specs.model_id = sysctl("hw.model").unwrap_or_default();

        let hardware = run("system_profiler", &["SPHardwareDataType"]);
        if let Some(text) = &hardware {
            specs.model_name = profile_value(text, "Model Name").unwrap_or_default();
            specs.firmware = profile_value(text, "System Firmware Version").unwrap_or_default();
        }

        // Apple silicon reports "hw.optional.arm64" and splits cores by performance
        // level; Intel Macs expose a single cache/frequency set instead.
        let apple_silicon = sysctl("hw.optional.arm64").as_deref() == Some("1");
        let cpu = &mut specs.cpu;
        cpu.name = sysctl("machdep.cpu.brand_string")
            .or_else(|| {
                hardware
                    .as_deref()
                    .and_then(|t| profile_value(t, "Processor Name"))
            })
            .unwrap_or_default();
        cpu.vendor = sysctl("machdep.cpu.vendor").unwrap_or_default();
        cpu.architecture = if apple_silicon {
            "arm64".into()
        } else {
            std::env::consts::ARCH.to_string()
        };
        cpu.physical_cores = sysctl_num("hw.physicalcpu").map(|v| v as u32);
        cpu.logical_cores = sysctl_num("hw.logicalcpu").map(|v| v as u32);
        if apple_silicon {
            cpu.performance_cores = sysctl_num("hw.perflevel0.physicalcpu").map(|v| v as u32);
            cpu.efficiency_cores = sysctl_num("hw.perflevel1.physicalcpu").map(|v| v as u32);
        }
        cpu.base_mhz = sysctl_num("hw.cpufrequency")
            .map(|hz| hz / 1_000_000)
            .filter(|mhz| *mhz > 0);
        cpu.max_mhz = sysctl_num("hw.cpufrequency_max")
            .map(|hz| hz / 1_000_000)
            .filter(|mhz| *mhz > 0)
            .filter(|mhz| cpu.base_mhz != Some(*mhz));
        cpu.l1_instruction_bytes = sysctl_num("hw.l1icachesize");
        cpu.l1_data_bytes = sysctl_num("hw.l1dcachesize");
        cpu.l2_bytes = sysctl_num("hw.l2cachesize");
        cpu.l3_bytes = sysctl_num("hw.l3cachesize");
        cpu.cache_line_bytes = sysctl_num("hw.cachelinesize");

        specs.memory.total_bytes = sysctl_num("hw.memsize");
        if let Some(text) = run("system_profiler", &["SPMemoryDataType"]) {
            let profile = parse_memory_profile(&text);
            specs.memory.ecc = profile.ecc;
            specs.memory.upgradeable = profile.upgradeable;
            if specs.memory.total_bytes.is_none() {
                let sum: u64 = profile.modules.iter().filter_map(|m| m.size_bytes).sum();
                if sum > 0 {
                    specs.memory.total_bytes = Some(sum);
                }
            }
            specs.memory.modules = profile.modules;
        }
        if let Some(text) = run("system_profiler", &["SPDisplaysDataType"]) {
            specs.graphics = parse_graphics_profile(&text);
        }
        if let Some(text) = run("system_profiler", &["SPNVMeDataType"]) {
            specs.drives = parse_storage_profile(&text);
        }
        for mount in mounts() {
            if let Some(text) = run("diskutil", &["info", &mount]) {
                if let Some(mut volume) = parse_diskutil_info(&text) {
                    volume.mount = mount;
                    specs.volumes.push(volume);
                }
            }
        }
        specs.battery = run("pmset", &["-g", "batt"]).and_then(|t| parse_battery(&t));
        specs.bluetooth = bluetooth_status();
        specs.wifi_interface =
            run("networksetup", &["-listallhardwareports"]).and_then(|t| parse_wifi_interface(&t));
        specs.board_id = run("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])
            .and_then(|t| parse_board_id(&t));

        if apple_silicon {
            specs.unavailable.push(
                "Cache levels, CPU clock and memory module details are not exposed by sysctl on Apple silicon."
                    .into(),
            );
        }
        if specs.drives.is_empty() {
            specs.unavailable.push(
                "No NVMe device was reported; other drive types may expose no health data.".into(),
            );
        }
        if specs.volumes.iter().all(|v| v.smart.is_empty()) {
            specs
                .unavailable
                .push("This drive exposes no S.M.A.R.T. state through diskutil.".into());
        }
        specs
    }

    /// Mount points are what `diskutil info` accepts and what the UI displays.
    fn mounts() -> Vec<String> {
        let disks = Disks::new_with_refreshed_list();
        let mut found: Vec<String> = Vec::new();
        for disk in disks.list() {
            let mount = disk.mount_point().to_string_lossy().into_owned();
            // APFS exposes synthetic mounts under /System/Volumes that repeat the
            // same underlying device.
            if mount.is_empty() || mount.starts_with("/System/Volumes/") {
                continue;
            }
            if !found.contains(&mount) {
                found.push(mount);
            }
        }
        found
    }
}

#[cfg(not(target_os = "macos"))]
mod portable {
    use super::*;

    pub fn collect() -> SystemSpecs {
        let mut system = sysinfo::System::new_all();
        system.refresh_all();
        let mut specs = SystemSpecs {
            sources: "sysinfo (OS APIs)".into(),
            ..Default::default()
        };
        specs.os_name = sysinfo::System::name().unwrap_or_else(|| std::env::consts::OS.into());
        specs.os_version = sysinfo::System::os_version().unwrap_or_default();
        specs.kernel = sysinfo::System::kernel_version().unwrap_or_default();
        specs.model_name = sysinfo::System::long_os_version().unwrap_or_default();
        if let Some(cpu) = system.cpus().first() {
            specs.cpu.name = cpu.brand().to_owned();
            specs.cpu.vendor = cpu.vendor_id().to_owned();
            specs.cpu.base_mhz = Some(cpu.frequency()).filter(|v| *v > 0);
        }
        specs.cpu.architecture = std::env::consts::ARCH.into();
        specs.cpu.physical_cores = sysinfo::System::physical_core_count().map(|n| n as u32);
        specs.cpu.logical_cores = Some(system.cpus().len() as u32);
        specs.memory.total_bytes = Some(system.total_memory());
        specs.unavailable.push(
            "Cache sizes, memory module details, drive health and battery state are collected only on macOS in this build."
                .into(),
        );
        specs
    }
}

// ------------------------------------------------------------------- parsing
// These parsers are platform independent so they can be unit tested against
// captured command output.

fn run_section(text: &str) -> Vec<(usize, String, String)> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.split_once(':') {
            Some((key, value)) => lines.push((
                indent,
                key.trim().to_owned(),
                value.trim().trim_end_matches(',').to_owned(),
            )),
            None => lines.push((indent, trimmed.to_owned(), String::new())),
        }
    }
    lines
}

fn profile_value(text: &str, key: &str) -> Option<String> {
    run_section(text)
        .into_iter()
        .find(|(_, k, v)| k == key && !v.is_empty())
        .map(|(_, _, v)| v)
}

/// Decimal units are used by `system_profiler`; exact byte counts win when present.
fn parse_capacity(value: &str) -> Option<u64> {
    if let Some(start) = value.find('(') {
        let inside = &value[start + 1..];
        if let Some(end) = inside.find(" bytes") {
            let digits: String = inside[..end]
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect();
            if let Ok(bytes) = digits.parse::<u64>() {
                return Some(bytes);
            }
        }
    }
    let mut parts = value.split_whitespace();
    let number: f64 = parts.next()?.parse().ok()?;
    let unit = parts.next()?.to_lowercase();
    let multiplier = match unit.as_str() {
        "tb" => 1e12,
        "gb" => 1e9,
        "mb" => 1e6,
        "kb" => 1e3,
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

fn parse_mhz(value: &str) -> Option<u64> {
    value
        .split_whitespace()
        .find_map(|part| part.parse::<u64>().ok())
        .filter(|v| *v > 0)
}

fn parse_memory_profile(text: &str) -> MemoryProfile {
    let mut profile = MemoryProfile::default();
    let mut current: Option<MemoryModule> = None;
    for (indent, key, value) in run_section(text) {
        match key.as_str() {
            "ECC" => profile.ecc = Some(value == "Enabled"),
            "Upgradeable Memory" => profile.upgradeable = Some(value == "Yes"),
            "Size" if current.is_some() => {
                if let Some(module) = current.as_mut() {
                    module.size_bytes = parse_capacity(&value);
                }
            }
            "Type" if current.is_some() => {
                if let Some(module) = current.as_mut() {
                    module.kind = value;
                }
            }
            "Speed" if current.is_some() => {
                if let Some(module) = current.as_mut() {
                    module.speed_mhz = parse_mhz(&value);
                }
            }
            "Manufacturer" if current.is_some() => {
                if let Some(module) = current.as_mut() {
                    module.manufacturer = value;
                }
            }
            "Part Number" if current.is_some() => {
                if let Some(module) = current.as_mut() {
                    module.part_number = value;
                }
            }
            _ => {
                let looks_like_slot = value.is_empty()
                    && (key.contains("BANK") || key.contains("DIMM") || key.starts_with("Slot"));
                if looks_like_slot && indent > 0 {
                    if let Some(module) = current.take() {
                        profile.modules.push(module);
                    }
                    current = Some(MemoryModule {
                        slot: key,
                        ..Default::default()
                    });
                }
            }
        }
    }
    if let Some(module) = current.take() {
        profile.modules.push(module);
    }
    profile
}

fn parse_graphics_profile(text: &str) -> Vec<GraphicsSpec> {
    let mut found: Vec<GraphicsSpec> = Vec::new();
    for (indent, key, value) in run_section(text) {
        if value.is_empty() && indent <= 4 {
            // Section and adapter headers both have an empty value; only adapter
            // names are followed by chipset fields, so skip the known headings.
            if key == "Graphics/Displays" || key == "Displays" {
                continue;
            }
            found.push(GraphicsSpec {
                name: key,
                ..Default::default()
            });
            continue;
        }
        let Some(gpu) = found.last_mut() else {
            continue;
        };
        if indent > 6 {
            // Display properties live below the adapter.
            continue;
        }
        match key.as_str() {
            "Chipset Model" => gpu.chipset = value,
            "Vendor" => gpu.vendor = value,
            "Device ID" | "Device ID:" => gpu.device_id = value,
            "Bus" => gpu.bus = value,
            "VRAM (Dynamic, Max)" | "VRAM (Total)" | "VRAM" => {
                gpu.vram_bytes = parse_capacity(&value)
            }
            "Metal Support" | "Metal" | "API Support" => gpu.api = value,
            _ => {}
        }
    }
    found
}

fn parse_storage_profile(text: &str) -> Vec<DriveSpec> {
    let mut found: Vec<DriveSpec> = Vec::new();
    for (indent, key, value) in run_section(text) {
        if value.is_empty() && indent == 8 && key != "Volumes" {
            found.push(DriveSpec {
                model: key,
                ..Default::default()
            });
            continue;
        }
        let Some(drive) = found.last_mut() else {
            continue;
        };
        if indent != 10 {
            continue;
        }
        match key.as_str() {
            "Model" => drive.model = value,
            "Revision" => drive.firmware = value,
            "Capacity" => drive.capacity_bytes = parse_capacity(&value),
            "TRIM Support" => drive.trim = Some(value == "Yes"),
            "S.M.A.R.T. status" | "S.M.A.R.T. Status" => drive.smart = value,
            "Link Width" => drive.link = value,
            "Link Speed" => {
                if drive.link.is_empty() {
                    drive.link = value;
                } else {
                    drive.link = format!("{} · {}", drive.link, value);
                }
            }
            "Protocol" => drive.protocol = value,
            _ => {}
        }
    }
    found
}

fn parse_diskutil_info(text: &str) -> Option<VolumeHealth> {
    let mut health = VolumeHealth::default();
    for (_, key, value) in run_section(text) {
        match key.as_str() {
            "Device / Media Name" => health.media = value,
            "Protocol" => health.protocol = value,
            "SMART Status" => health.smart = value,
            "Solid State" => health.solid_state = Some(value == "Yes"),
            _ => {}
        }
    }
    if health.smart.is_empty() && health.protocol.is_empty() && health.media.is_empty() {
        return None;
    }
    Some(health)
}

fn parse_board_id(text: &str) -> Option<String> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("\"board-id\" = "))?;
    let value = line.split_once("= <\"")?.1.split_once("\">")?.0;
    if value.is_empty() || value.len() > 128 {
        None
    } else {
        Some(value.to_owned())
    }
}

fn parse_wifi_interface(text: &str) -> Option<String> {
    let mut wifi = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(port) = line.strip_prefix("Hardware Port: ") {
            wifi = port == "Wi-Fi";
        } else if wifi {
            if let Some(device) = line.strip_prefix("Device: ") {
                let device = device.trim();
                if !device.is_empty() {
                    return Some(device.to_owned());
                }
            }
        }
    }
    None
}

fn parse_bluetooth(text: &str) -> Option<BluetoothSpec> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let controller = value
        .get("SPBluetoothDataType")?
        .as_array()?
        .first()?
        .get("controller_properties")?;
    let state = match controller.get("controller_state").and_then(|v| v.as_str()) {
        Some("attrib_on") => Some(true),
        Some("attrib_off") => Some(false),
        _ => None,
    };
    let devices = value
        .get("SPBluetoothDataType")?
        .as_array()?
        .first()?
        .get("device_connected")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .flat_map(|item| item.as_object().into_iter().flat_map(|obj| obj.keys()))
                .take(32)
                .cloned()
                .collect()
        });
    let transport = controller
        .get("controller_transport")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let chipset = controller
        .get("controller_chipset")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    if state.is_none() && transport.is_empty() && chipset.is_empty() {
        return None;
    }
    Some(BluetoothSpec {
        state,
        transport,
        chipset,
        connected_devices: devices,
    })
}

fn parse_battery(text: &str) -> Option<String> {
    let mut power = None;
    let mut charge = None;
    for line in text.lines() {
        if let Some(start) = line.find('\'') {
            if let Some(end) = line[start + 1..].find('\'') {
                power = Some(line[start + 1..start + 1 + end].to_owned());
            }
        }
        if let Some(index) = line.find('%') {
            let digits: String = line[..index]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !digits.is_empty() {
                let state = line[index..]
                    .split(';')
                    .nth(1)
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_default();
                charge = Some((digits, state));
            }
        }
    }
    let (percent, state) = charge?;
    let mut out = format!("{percent}%");
    if !state.is_empty() {
        out.push_str(&format!(" · {state}"));
    }
    if let Some(power) = power {
        out.push_str(&format!(" · {power}"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMORY: &str = r#"
Memory:

    Memory Slots:

      ECC: Disabled
      Upgradeable Memory: No

        BANK 0/ChannelA-DIMM0:

          Size: 8 GB
          Type: DDR4
          Speed: 2400 MHz
          Status: OK
          Manufacturer: Micron
          Part Number: 4ATS1G64HZ-2G6E1
          Serial Number: -

        BANK 2/ChannelB-DIMM0:

          Size: 8 GB
          Type: DDR4
          Speed: 2400 MHz
          Manufacturer: Micron
          Part Number: 4ATS1G64HZ-2G6E1
"#;

    const GRAPHICS: &str = r#"
Graphics/Displays:

    Intel UHD Graphics 620:

      Chipset Model: Intel UHD Graphics 620
      Type: GPU
      Bus: Built-In
      VRAM (Dynamic, Max): 1536 MB
      Vendor: Intel
      Device ID: 0x3e9b
      Metal Support: Metal 3
      Displays:
        Display:
          Resolution: 1920 x 1080
          Main Display: Yes
"#;

    const NVME: &str = r#"
NVMExpress:

    Generic SSD Controller:

        WDC PC SN520 SDAPNUW-512G-1014:

          Capacity: 512.11 GB (512,110,190,592 bytes)
          TRIM Support: Yes
          Model: WDC PC SN520 SDAPNUW-512G-1014
          Revision: 20110000
          Link Width: x2
          Link Speed: 8.0 GT/s
          BSD Name: disk0
          S.M.A.R.T. status: Verified
          Volumes:
            NO NAME:
              Capacity: 209.7 MB
"#;

    const DISKUTIL: &str = r#"
   Device Identifier:         disk1s4s1
   Volume Name:               Macintosh HD
   Mount Point:               /
   Protocol:                  PCI-Express
   SMART Status:              Verified
   Disk Size:                 256.0 GB (256000000000 Bytes)
   Device Location:           Internal
   Solid State:               Yes
"#;

    #[test]
    fn memory_modules_are_split_per_slot() {
        let profile = parse_memory_profile(MEMORY);
        assert_eq!(profile.ecc, Some(false));
        assert_eq!(profile.upgradeable, Some(false));
        assert_eq!(profile.modules.len(), 2);
        assert_eq!(profile.modules[0].slot, "BANK 0/ChannelA-DIMM0");
        assert_eq!(profile.modules[0].size_bytes, Some(8_000_000_000));
        assert_eq!(profile.modules[0].kind, "DDR4");
        assert_eq!(profile.modules[0].speed_mhz, Some(2400));
        assert_eq!(profile.modules[1].manufacturer, "Micron");
        assert_eq!(profile.modules[1].part_number, "4ATS1G64HZ-2G6E1");
    }

    #[test]
    fn graphics_profile_keeps_adapter_fields_and_ignores_displays() {
        let gpus = parse_graphics_profile(GRAPHICS);
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "Intel UHD Graphics 620");
        assert_eq!(gpus[0].vendor, "Intel");
        assert_eq!(gpus[0].device_id, "0x3e9b");
        assert_eq!(gpus[0].vram_bytes, Some(1_536_000_000));
        assert_eq!(gpus[0].api, "Metal 3");
        assert_eq!(gpus[0].bus, "Built-In");
        assert!(!gpus[0].name.contains("Resolution"));
    }

    #[test]
    fn nvme_profile_reads_identity_and_health() {
        let drives = parse_storage_profile(NVME);
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].model, "WDC PC SN520 SDAPNUW-512G-1014");
        assert_eq!(drives[0].firmware, "20110000");
        assert_eq!(drives[0].smart, "Verified");
        assert_eq!(drives[0].trim, Some(true));
        assert_eq!(drives[0].link, "x2 · 8.0 GT/s");
        assert_eq!(drives[0].capacity_bytes, Some(512_110_190_592));
    }

    #[test]
    fn diskutil_volume_health_is_read_without_guessing() {
        let health = parse_diskutil_info(DISKUTIL).unwrap();
        assert_eq!(health.protocol, "PCI-Express");
        assert_eq!(health.smart, "Verified");
        assert_eq!(health.solid_state, Some(true));
        // The mount point is filled in by the caller from the volume it queried.
        assert!(health.mount.is_empty());
        assert!(parse_diskutil_info("   Volume Name: test\n").is_none());
    }

    #[test]
    fn motherboard_id_uses_only_the_named_registry_field() {
        assert_eq!(
            parse_board_id("  \"board-id\" = <\"Mac-TEST\">\n"),
            Some("Mac-TEST".into())
        );
        assert_eq!(parse_board_id("  \"model\" = <\"Mac-TEST\">\n"), None);
    }

    #[test]
    fn wifi_identification_uses_os_port_mapping_only() {
        assert_eq!(
            parse_wifi_interface(
                "Hardware Port: Ethernet\nDevice: en1\n\nHardware Port: Wi-Fi\nDevice: en0\n"
            ),
            Some("en0".into())
        );
        assert_eq!(
            parse_wifi_interface("Hardware Port: Ethernet\nDevice: en0\n"),
            None
        );
    }

    #[test]
    fn bluetooth_status_is_optional_and_does_not_collect_device_identity() {
        let info = parse_bluetooth(r#"{"SPBluetoothDataType":[{"controller_properties":{"controller_state":"attrib_on","controller_transport":"USB","controller_address":"ignored"}}]}"#).unwrap();
        assert_eq!(info.state, Some(true));
        assert_eq!(info.transport, "USB");
        assert!(info.connected_devices.is_none());
        let connected = parse_bluetooth(r#"{"SPBluetoothDataType":[{"controller_properties":{"controller_state":"attrib_on"},"device_connected":[{"Example device":{"device_address":"ignored"}}]}]}"#).unwrap();
        assert_eq!(
            connected.connected_devices,
            Some(vec!["Example device".into()])
        );
        assert!(parse_bluetooth(r#"{"SPBluetoothDataType":[]}"#).is_none());
        assert!(parse_bluetooth("invalid").is_none());
    }

    #[test]
    fn battery_summary_is_compact_and_honest() {
        let text = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=6946915)\t100%; charged; 0:00 remaining present: true\n";
        assert_eq!(
            parse_battery(text).as_deref(),
            Some("100% · charged · AC Power")
        );
        assert!(parse_battery("garbage").is_none());
    }

    /// Runs the real collector so a broken parser is caught while it is cheap.
    /// It asserts only what every platform can guarantee, never device details.
    #[test]
    fn collection_completes_and_reports_its_sources() {
        let specs = collect();
        assert!(!specs.sources.is_empty());
        #[cfg(target_os = "macos")]
        assert_eq!(specs.os_name, "macOS");
        println!(
            "inventory: {} {} · {} · {} physical / {} logical cores · {} modules · {} drives · {} volumes · battery {:?}",
            specs.os_name,
            specs.os_version,
            specs.cpu.name,
            specs.cpu.physical_cores.unwrap_or(0),
            specs.cpu.logical_cores.unwrap_or(0),
            specs.memory.modules.len(),
            specs.drives.len(),
            specs.volumes.len(),
            specs.battery
        );
    }

    #[test]
    fn capacities_use_exact_bytes_when_available() {
        assert_eq!(
            parse_capacity("512.11 GB (512,110,190,592 bytes)"),
            Some(512_110_190_592)
        );
        assert_eq!(parse_capacity("256.0 GB"), Some(256_000_000_000));
        assert_eq!(parse_capacity("1536 MB"), Some(1_536_000_000));
        assert_eq!(parse_capacity("unknown"), None);
    }
}
