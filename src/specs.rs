//! Static system inventory: the specification sheet mainstream tools show.
//!
//! CPU-Z shows processor identification, caches, mainboard and memory modules;
//! CrystalDiskInfo shows drive identity and S.M.A.R.T. health; GPU-Z shows the
//! adapter, driver and VRAM. Everything here is read once at startup from local
//! OS sources only, so the inventory is cheap and never leaves the machine.
//!
//! Each field is optional on purpose: a missing value is reported as unavailable
//! instead of being guessed, because the sources differ per platform.

#[cfg(target_os = "macos")]
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
    #[cfg(target_os = "windows")]
    {
        windows::bluetooth()
    }
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
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

pub fn collect() -> SystemSpecs {
    #[cfg(target_os = "macos")]
    {
        macos::collect()
    }
    #[cfg(target_os = "windows")]
    {
        windows::collect()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
        specs.cpu.physical_cores = system.physical_core_count().map(|n| n as u32);
        specs.cpu.logical_cores = Some(system.cpus().len() as u32);
        specs.memory.total_bytes = Some(system.total_memory());
        #[cfg(not(target_os = "windows"))]
        specs.unavailable.push(
            "Cache sizes, memory module details, drive health and battery state are collected only on macOS in this build."
                .into(),
        );
        specs
    }
}

/// SMBIOS memory type codes, as reported by `Win32_PhysicalMemory`. Unknown
/// codes stay numeric instead of being labelled with a guess.
#[cfg(any(target_os = "windows", test))]
fn smbios_memory_kind(code: u64) -> String {
    match code {
        20 => "DDR".into(),
        21 => "DDR2".into(),
        22 => "DDR2 FB-DIMM".into(),
        24 => "DDR3".into(),
        26 => "DDR4".into(),
        27 => "LPDDR".into(),
        28 => "LPDDR2".into(),
        29 => "LPDDR3".into(),
        30 => "LPDDR4".into(),
        34 => "DDR5".into(),
        35 => "LPDDR5".into(),
        other => format!("SMBIOS type {other}"),
    }
}

/// `Win32_Battery.BatteryStatus` codes.
#[cfg(any(target_os = "windows", test))]
fn battery_state(code: u64) -> &'static str {
    match code {
        1 => "discharging",
        2 => "on AC",
        3 => "fully charged",
        4 => "low",
        5 => "critical",
        6 => "charging",
        7 => "charging, high",
        8 => "charging, low",
        9 => "charging, critical",
        11 => "partially charged",
        _ => "state not reported",
    }
}

/// CIM dates look like `20200101000000.000000+000`; the sheet shows the day.
#[cfg(any(target_os = "windows", test))]
fn cim_date(text: &str) -> String {
    let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 {
        return String::new();
    }
    format!("{}-{}-{}", &digits[0..4], &digits[4..6], &digits[6..8])
}

/// Windows exposes model, firmware, memory modules, drive health, battery,
/// Wi-Fi and Bluetooth through CIM/WMI. `sysinfo` covers only the CPU, memory
/// total, volumes and network counters, so one read-only PowerShell pass fills
/// the rest of the specification sheet instead of leaving it blank.
#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use serde_json::Value;
    use std::{os::windows::process::CommandExt, process::Command};

    /// Keeps a console window from flashing up when the query runs at startup.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    const SCRIPT: &str = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'SilentlyContinue'
$cs = Get-CimInstance Win32_ComputerSystem
$bios = Get-CimInstance Win32_BIOS
$board = Get-CimInstance Win32_BaseBoard
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$bat = Get-CimInstance Win32_Battery | Select-Object -First 1
$mem = @(Get-CimInstance Win32_PhysicalMemory | ForEach-Object { [pscustomobject]@{ slot = [string]$_.DeviceLocator; size = [uint64]$_.Capacity; type = [uint64]$_.SMBIOSMemoryType; speed = [uint64]$_.Speed; manufacturer = [string]$_.Manufacturer; part = [string]$_.PartNumber } })
$volumes = @(Get-Volume | Where-Object { $_.DriveLetter } | ForEach-Object { [pscustomobject]@{ letter = [string]$_.DriveLetter; fs = [string]$_.FileSystem; health = [string]$_.HealthStatus; size = [uint64]$_.Size; free = [uint64]$_.SizeRemaining } })
$disks = @(Get-PhysicalDisk | ForEach-Object { [pscustomobject]@{ model = [string]$_.FriendlyName; media = [string]$_.MediaType; bus = [string]$_.BusType; health = [string]$_.HealthStatus; size = [uint64]$_.Size } })
$reliability = @(Get-PhysicalDisk | Get-StorageReliabilityCounter | ForEach-Object { [pscustomobject]@{ device = [string]$_.DeviceId; temperature = $_.Temperature; wear = $_.Wear; power_hours = $_.PowerOnHours; read_errors = $_.ReadErrorsTotal; write_errors = $_.WriteErrorsTotal } })
$diskdrives = @(Get-CimInstance Win32_DiskDrive | ForEach-Object { [pscustomobject]@{ model = [string]$_.Model; firmware = [string]$_.FirmwareRevision; interface = [string]$_.InterfaceType; size = [uint64]$_.Size } })
$network = @(Get-NetAdapter | ForEach-Object { [pscustomobject]@{ name = [string]$_.Name; description = [string]$_.InterfaceDescription; media = [string]$_.PhysicalMediaType; status = [string]$_.Status; speed = [string]$_.LinkSpeed } })
$bluetooth = @(Get-PnpDevice -Class Bluetooth | ForEach-Object { [pscustomobject]@{ name = [string]$_.FriendlyName; status = [string]$_.Status } })
$battery = if ($bat) { [pscustomobject]@{ name = [string]$bat.Name; charge = $bat.EstimatedChargeRemaining; status = $bat.BatteryStatus } } else { $null }
[pscustomobject]@{
  manufacturer = [string]$cs.Manufacturer
  model = [string]$cs.Model
  sku = [string]$cs.SystemSKUNumber
  family = [string]$cs.SystemFamily
  bios_vendor = [string]$bios.Manufacturer
  bios_version = [string]$bios.SMBIOSBIOSVersion
  bios_date = [string]$bios.ReleaseDate
  board_manufacturer = [string]$board.Manufacturer
  board_product = [string]$board.Product
  cpu_max_clock = [uint64]$cpu.MaxClockSpeed
  cpu_l2_kb = [uint64]$cpu.L2CacheSize
  cpu_l3_kb = [uint64]$cpu.L3CacheSize
  battery = $battery
  memory = $mem
  volumes = $volumes
  disks = $disks
  reliability = $reliability
  diskdrives = $diskdrives
  network = $network
  bluetooth = $bluetooth
} | ConvertTo-Json -Depth 6 -Compress
"#;

    const BLUETOOTH_SCRIPT: &str = r#"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ErrorActionPreference = 'SilentlyContinue'
$devices = @(Get-PnpDevice -Class Bluetooth | ForEach-Object { [pscustomobject]@{ name = [string]$_.FriendlyName; status = [string]$_.Status } })
ConvertTo-Json -InputObject $devices -Depth 3 -Compress
"#;

    pub fn collect() -> SystemSpecs {
        let mut specs = super::portable::collect();
        specs.sources = "sysinfo · Windows CIM/WMI (read-only)".into();
        if !specs.kernel.is_empty() {
            specs.kernel = format!("build {}", specs.kernel);
        }
        if let Some(value) = query(SCRIPT) {
            apply(&mut specs, &value);
        }
        specs.unavailable.clear();
        if specs.memory.modules.is_empty() {
            specs
                .unavailable
                .push("Memory module records were not reported by the firmware.".into());
        }
        if specs.drives.is_empty() {
            specs
                .unavailable
                .push("No physical drive was reported by the Windows storage provider.".into());
        }
        specs
            .unavailable
            .push("L1 cache and cache-line sizes are not read on Windows in this build.".into());
        specs.unavailable.push(
            "Temperature comes from the firmware's ACPI thermal zones (Thermal Zone Information performance counters, or the WMI class when elevated). Per-core CPU temperature needs a kernel driver and is not read."
                .into(),
        );
        specs
    }

    pub fn bluetooth() -> Option<BluetoothSpec> {
        let value = query(BLUETOOTH_SCRIPT)?;
        bluetooth_from_value(&value)
    }

    fn query(script: &str) -> Option<Value> {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        serde_json::from_slice(&output.stdout).ok()
    }

    fn text(value: Option<&Value>) -> String {
        value
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned()
    }

    fn joined(parts: [String; 2]) -> String {
        parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(super) fn bluetooth_from_value(value: &Value) -> Option<BluetoothSpec> {
        let devices = value.as_array()?;
        let all: Vec<String> = devices
            .iter()
            .map(|device| text(device.get("name")))
            .filter(|name| !name.is_empty())
            .collect();
        // The class also lists driver stacks (enumerators, RFCOMM/TDI services),
        // which describe the transport rather than the controller.
        let controllers: Vec<String> = all
            .iter()
            .filter(|name| {
                let lower = name.to_lowercase();
                !["enumer", "列舉", "rfcomm", "tdi", "protocol", "service"]
                    .iter()
                    .any(|noise| lower.contains(noise))
            })
            .cloned()
            .collect();
        let names = if controllers.is_empty() {
            all
        } else {
            controllers
        };
        // Paired devices also appear in this class; only the controller names
        // are reported, and no device is claimed to be connected.
        let adapters: Vec<String> = names
            .iter()
            .filter(|name| name.to_lowercase().contains("bluetooth"))
            .cloned()
            .collect();
        let names = if adapters.is_empty() { names } else { adapters };
        if names.is_empty() {
            return None;
        }
        let ready = devices
            .iter()
            .any(|device| text(device.get("status")).eq_ignore_ascii_case("OK"));
        Some(BluetoothSpec {
            state: Some(ready),
            transport: String::new(),
            chipset: names.join(", "),
            connected_devices: None,
        })
    }

    pub(super) fn apply(specs: &mut SystemSpecs, v: &Value) {
        specs.model_name = joined([text(v.get("manufacturer")), text(v.get("model"))]);
        // ODM firmwares commonly report an all-zero SKU, which identifies nothing.
        specs.model_id = [text(v.get("sku")), text(v.get("family"))]
            .into_iter()
            .find(|part| !part.is_empty() && part.chars().any(|c| c != '0'))
            .unwrap_or_default();
        let firmware = joined([text(v.get("bios_vendor")), text(v.get("bios_version"))]);
        let date = cim_date(&text(v.get("bios_date")));
        specs.firmware = match (firmware.is_empty(), date.is_empty()) {
            (true, true) => String::new(),
            (false, true) => firmware,
            (true, false) => date,
            (false, false) => format!("{firmware} ({date})"),
        };
        let board = joined([
            text(v.get("board_manufacturer")),
            text(v.get("board_product")),
        ]);
        specs.board_id = (!board.is_empty()).then_some(board);

        // `Win32_Processor` reports cache sizes in KiB and the rated clock.
        let cache = |key: &str| {
            v.get(key)
                .and_then(Value::as_u64)
                .filter(|kb| *kb > 0)
                .map(|kb| kb * 1024)
        };
        if specs.cpu.l2_bytes.is_none() {
            specs.cpu.l2_bytes = cache("cpu_l2_kb");
        }
        if specs.cpu.l3_bytes.is_none() {
            specs.cpu.l3_bytes = cache("cpu_l3_kb");
        }
        // `sysinfo` reports the current clock, not the rated one, so the sheet
        // uses the firmware's rated clock for the base row instead.
        if let Some(mhz) = v
            .get("cpu_max_clock")
            .and_then(Value::as_u64)
            .filter(|mhz| *mhz > 0)
        {
            specs.cpu.base_mhz = Some(mhz);
            specs.cpu.max_mhz = None;
        }

        if let Some(modules) = v.get("memory").and_then(Value::as_array) {
            for module in modules {
                let Some(size) = module
                    .get("size")
                    .and_then(Value::as_u64)
                    .filter(|size| *size > 0)
                else {
                    continue;
                };
                specs.memory.modules.push(MemoryModule {
                    slot: text(module.get("slot")),
                    size_bytes: Some(size),
                    kind: module
                        .get("type")
                        .and_then(Value::as_u64)
                        .map(smbios_memory_kind)
                        .unwrap_or_default(),
                    speed_mhz: module
                        .get("speed")
                        .and_then(Value::as_u64)
                        .filter(|speed| *speed > 0),
                    manufacturer: text(module.get("manufacturer")),
                    part_number: text(module.get("part")),
                });
            }
        }

        if let Some(battery) = v.get("battery").filter(|battery| !battery.is_null()) {
            let state = battery
                .get("status")
                .and_then(Value::as_u64)
                .map(battery_state)
                .unwrap_or("state not reported");
            specs.battery = Some(match battery.get("charge").and_then(Value::as_u64) {
                Some(charge) => format!("{charge}% · {state}"),
                None => state.to_owned(),
            });
        }

        let volume_rows = v.get("volumes").and_then(Value::as_array);
        for volume in volume_rows.into_iter().flatten() {
            let letter = text(volume.get("letter"));
            if letter.is_empty() {
                continue;
            }
            specs.volumes.push(VolumeHealth {
                mount: format!("{letter}:\\"),
                media: text(volume.get("fs")),
                protocol: String::new(),
                solid_state: None,
                smart: text(volume.get("health")),
            });
        }

        let reliability = v
            .get("reliability")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let diskdrives = v
            .get("diskdrives")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, disk) in v
            .get("disks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let model = text(disk.get("model"));
            let mut smart = text(disk.get("health"));
            if let Some(counter) = reliability
                .iter()
                .find(|counter| text(counter.get("device")) == index.to_string())
                .or_else(|| reliability.get(index))
            {
                let mut extras = Vec::new();
                if let Some(temperature) = counter.get("temperature").and_then(Value::as_u64) {
                    if temperature > 0 {
                        extras.push(format!("{temperature} °C"));
                    }
                }
                if let Some(wear) = counter.get("wear").and_then(Value::as_u64) {
                    if wear > 0 {
                        extras.push(format!("wear {wear}%"));
                    }
                }
                if let Some(hours) = counter.get("power_hours").and_then(Value::as_u64) {
                    if hours > 0 {
                        extras.push(format!("{hours} power-on hours"));
                    }
                }
                for (key, label) in [
                    ("read_errors", "read errors"),
                    ("write_errors", "write errors"),
                ] {
                    if let Some(count) = counter.get(key).and_then(Value::as_u64) {
                        if count > 0 {
                            extras.push(format!("{count} {label}"));
                        }
                    }
                }
                if !extras.is_empty() {
                    if !smart.is_empty() {
                        smart.push_str(" · ");
                    }
                    smart.push_str(&extras.join(" · "));
                }
            }
            let firmware = diskdrives
                .iter()
                .find(|drive| text(drive.get("model")) == model)
                .map(|drive| text(drive.get("firmware")))
                .unwrap_or_default();
            specs.drives.push(DriveSpec {
                model,
                firmware,
                protocol: text(disk.get("bus")),
                link: text(disk.get("media")),
                trim: None,
                smart,
                capacity_bytes: disk
                    .get("size")
                    .and_then(Value::as_u64)
                    .filter(|size| *size > 0),
            });
        }

        if let Some(adapters) = v.get("network").and_then(Value::as_array) {
            specs.wifi_interface = adapters
                .iter()
                .find(|adapter| {
                    let media = text(adapter.get("media")).to_lowercase();
                    let description = text(adapter.get("description")).to_lowercase();
                    media.contains("802.11")
                        || description.contains("wi-fi")
                        || description.contains("wireless")
                        || description.contains("wlan")
                })
                .map(|adapter| text(adapter.get("name")))
                .filter(|name| !name.is_empty());
        }

        specs.bluetooth = v.get("bluetooth").and_then(bluetooth_from_value);
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
    fn smbios_and_battery_codes_are_labelled_not_invented() {
        assert_eq!(smbios_memory_kind(26), "DDR4");
        assert_eq!(smbios_memory_kind(34), "DDR5");
        assert_eq!(smbios_memory_kind(99), "SMBIOS type 99");
        assert_eq!(battery_state(1), "discharging");
        assert_eq!(battery_state(2), "on AC");
        assert_eq!(battery_state(0), "state not reported");
        assert_eq!(cim_date("20190612000000.000000+000"), "2019-06-12");
        assert_eq!(cim_date(""), "");
    }

    /// The Windows report is parsed from a captured shape, so the mapping is
    /// tested without depending on the machine's own hardware.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_cim_report_fills_the_specification_sheet() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{
            "manufacturer":"Acer","model":"Swift","sku":"0000000000000000","family":"Swift 5",
            "bios_vendor":"Insyde Corp.","bios_version":"V1.09","bios_date":"20190612000000.000000+000",
            "board_manufacturer":"WL","board_product":"Carlsberg",
            "battery":{"charge":97,"status":2},
            "memory":[
              {"slot":"DIMM0","size":8589934592,"type":26,"speed":2667,"manufacturer":"Micron","part":"X"},
              {"slot":"DIMM1","size":0,"type":26,"speed":0,"manufacturer":"","part":""}
            ],
            "volumes":[{"letter":"C","fs":"NTFS","health":"Healthy"}],
            "disks":[{"model":"WDC SN520","media":"SSD","bus":"NVMe","health":"Healthy","size":512110190592}],
            "reliability":[{"device":"0","temperature":41,"wear":3,"power_hours":1200}],
            "diskdrives":[{"model":"WDC SN520","firmware":"20110000"}],
            "network":[{"name":"Wi-Fi","description":"Intel Wireless-AC","media":"Native 802.11","status":"Up"}],
            "bluetooth":[
              {"name":"Microsoft Bluetooth Enumerator","status":"OK"},
              {"name":"Intel(R) Wireless Bluetooth(R)","status":"OK"}
            ]
        }"#,
        )
        .unwrap();
        let mut specs = SystemSpecs::default();
        windows::apply(&mut specs, &value);
        assert_eq!(specs.model_name, "Acer Swift");
        assert_eq!(specs.model_id, "Swift 5");
        assert_eq!(specs.firmware, "Insyde Corp. V1.09 (2019-06-12)");
        assert_eq!(specs.board_id.as_deref(), Some("WL Carlsberg"));
        assert_eq!(specs.battery.as_deref(), Some("97% · on AC"));
        assert_eq!(specs.memory.modules.len(), 1);
        assert_eq!(specs.memory.modules[0].kind, "DDR4");
        assert_eq!(specs.memory.modules[0].size_bytes, Some(8_589_934_592));
        assert_eq!(specs.volumes[0].mount, "C:\\");
        assert_eq!(specs.volumes[0].smart, "Healthy");
        assert_eq!(specs.drives.len(), 1);
        assert_eq!(specs.drives[0].firmware, "20110000");
        assert_eq!(specs.drives[0].capacity_bytes, Some(512_110_190_592));
        assert!(specs.drives[0].smart.contains("41 °C"));
        assert!(specs.drives[0].smart.contains("wear 3%"));
        assert_eq!(specs.wifi_interface.as_deref(), Some("Wi-Fi"));
        let bluetooth = specs.bluetooth.expect("controller is reported");
        assert_eq!(bluetooth.state, Some(true));
        assert_eq!(bluetooth.chipset, "Intel(R) Wireless Bluetooth(R)");
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
