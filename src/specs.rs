//! Static system inventory: the specification sheet mainstream tools show.
//!
//! CPU-Z shows processor identification, caches, mainboard and memory modules;
//! CrystalDiskInfo shows drive identity and S.M.A.R.T. health; GPU-Z shows the
//! adapter, driver and VRAM. Everything here is read once at startup from local
//! OS sources only, so the inventory is cheap and never leaves the machine.
//!
//! Each field is optional on purpose: a missing value is reported as unavailable
//! instead of being guessed, because the sources differ per platform.

use serde::{Deserialize, Serialize};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
    /// Package and stepping identifiers, CPU-Z style.
    pub socket: String,
    pub stepping: String,
    pub l1_instruction_bytes: Option<u64>,
    pub l1_data_bytes: Option<u64>,
    pub l2_bytes: Option<u64>,
    pub l3_bytes: Option<u64>,
    pub cache_line_bytes: Option<u64>,
    /// CPU-Z style family/model/stepping line, as described by the firmware.
    pub description: String,
    /// Vendor processor identifier (`ProcessorId`).
    pub processor_id: String,
    /// Clock the OS sampled at collection time, which moves with load.
    pub current_mhz: Option<u64>,
    /// Instruction-set extensions the OS reports (for example SSE2, AVX2).
    pub instructions: String,
    /// Firmware-enabled virtualization, when exposed.
    pub virtualization: Option<bool>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryModule {
    pub slot: String,
    pub size_bytes: Option<u64>,
    pub kind: String,
    pub speed_mhz: Option<u64>,
    pub manufacturer: String,
    pub part_number: String,
    pub serial: String,
    /// Speed the firmware actually configured, which can differ from the rating.
    pub configured_mhz: Option<u64>,
    pub form_factor: String,
    pub bank: String,
    /// Bus widths in bits. A 72-bit total width on a 64-bit data bus means ECC.
    pub data_width: Option<u64>,
    pub total_width: Option<u64>,
    pub configured_voltage_mv: Option<u64>,
    pub min_voltage_mv: Option<u64>,
    pub max_voltage_mv: Option<u64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryProfile {
    pub total_bytes: Option<u64>,
    pub ecc: Option<bool>,
    pub upgradeable: Option<bool>,
    pub slots_total: Option<u32>,
    pub slots_used: Option<u32>,
    pub max_capacity_bytes: Option<u64>,
    pub modules: Vec<MemoryModule>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsSpec {
    pub name: String,
    pub chipset: String,
    pub vendor: String,
    pub device_id: String,
    pub vram_bytes: Option<u64>,
    pub api: String,
    pub bus: String,
    /// Installed display driver version and release date, GPU-Z style.
    pub driver: String,
    pub driver_date: String,
    /// PCI bus/device/function address parsed from the device instance path.
    pub pci: String,
    pub refresh: String,
    pub status: String,
    /// Video BIOS version and date when the display driver registered them.
    pub bios: String,
    pub bios_date: String,
    pub current_mode: String,
}

/// One installed driver record, as listed by the OS driver store.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DriverSpec {
    pub name: String,
    pub class_name: String,
    pub version: String,
    pub date: String,
    pub provider: String,
    pub signed: Option<bool>,
}

/// A display attached to the machine, decoded from its EDID identity block.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitorSpec {
    pub manufacturer: String,
    pub model: String,
    pub serial: String,
    pub year: Option<u32>,
    pub week: Option<u32>,
}

/// One network adapter, kept as a physical "part" rather than a traffic counter.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkAdapterSpec {
    pub name: String,
    pub description: String,
    pub status: String,
    pub speed: String,
}

/// A physical storage device, CrystalDiskInfo style.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DriveSpec {
    pub model: String,
    pub firmware: String,
    pub protocol: String,
    pub link: String,
    pub trim: Option<bool>,
    pub smart: String,
    pub serial: String,
    pub capacity_bytes: Option<u64>,
}

/// Health of one mounted volume, which is what a user can actually point at.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeHealth {
    pub mount: String,
    pub media: String,
    pub protocol: String,
    pub solid_state: Option<bool>,
    pub smart: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BluetoothSpec {
    pub state: Option<bool>,
    pub transport: String,
    pub chipset: String,
    pub connected_devices: Option<Vec<String>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
    /// OS-reported graphics driver build and current display mode.
    pub gpu_driver: String,
    pub gpu_driver_date: String,
    pub display: String,
    pub bluetooth: Option<BluetoothSpec>,
    pub wifi_interface: Option<String>,
    /// Drivers installed in the OS driver store, filtered to hardware classes.
    pub drivers: Vec<DriverSpec>,
    /// Attached displays, decoded from their EDID identity blocks.
    pub monitors: Vec<MonitorSpec>,
    /// Physical network adapters (not the live traffic counters).
    pub network_adapters: Vec<NetworkAdapterSpec>,
    pub board_vendor: String,
    pub board_model: String,
    pub board_version: String,
    pub chassis: String,
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
            .args(["-json", "-timeout", "5", "SPBluetoothDataType"])
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

// ------------------------------------------------------------ cached inventory
// Collecting the inventory shells out to slow OS tools (WMI, SMART, driver
// store), which can take many seconds. The last result is cached so the next
// launch can paint the Details page immediately and refresh in the background.

/// Schema version of the on-disk cache. Bump it when a field changes meaning so
/// an old file is ignored instead of being misread.
const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct InventoryCache {
    version: u32,
    /// Unix seconds when the inventory was collected.
    cached_at: u64,
    specs: SystemSpecs,
}

/// Read a previously saved inventory. Returns the specs and their collection
/// time, or `None` for a missing, unreadable or outdated file.
pub fn load_cache(path: &Path) -> Option<(SystemSpecs, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let cache: InventoryCache = serde_json::from_str(&text).ok()?;
    (cache.version == CACHE_VERSION).then_some((cache.specs, cache.cached_at))
}

/// Persist the inventory for the next launch. The write goes through a
/// temporary file and an atomic rename, so a crash cannot truncate the cache.
pub fn save_cache(path: &Path, specs: &SystemSpecs) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let cache = InventoryCache {
        version: CACHE_VERSION,
        cached_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default(),
        specs: specs.clone(),
    };
    let text = serde_json::to_string(&cache).map_err(|e| format!("serialise: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))?;
    Ok(())
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

/// True when the physical host is an Apple silicon Mac, even under Rosetta.
#[cfg(target_os = "macos")]
pub use macos::is_apple_silicon;
#[cfg(not(target_os = "macos"))]
pub fn is_apple_silicon() -> bool {
    false
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::io::Read;
    use std::time::{Duration, Instant};
    use sysinfo::Disks;

    fn run(command: &str, args: &[&str]) -> Option<String> {
        run_with_timeout(command, args, Duration::from_secs(5))
    }

    /// `system_profiler` can hang on a misbehaving device tree. Never let that
    /// stall the inventory thread indefinitely; missing fields degrade to
    /// "unavailable" in the UI.
    fn run_with_timeout(command: &str, args: &[&str], timeout: Duration) -> Option<String> {
        let mut child = Command::new(command)
            .args(args)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                if !status.success() {
                    return None;
                }
                let mut bytes = Vec::new();
                stdout.read_to_end(&mut bytes).ok()?;
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn sysctl(key: &str) -> Option<String> {
        let value = run("sysctl", &["-n", key])?;
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    }

    fn sysctl_num(key: &str) -> Option<u64> {
        sysctl(key)?.parse().ok()
    }

    /// Physical host architecture, even when this process is translated by Rosetta.
    pub fn is_apple_silicon() -> bool {
        sysctl("hw.optional.arm64").as_deref() == Some("1")
            || sysctl("sysctl.proc_translated").as_deref() == Some("1")
            || std::env::consts::ARCH == "aarch64"
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

        let hardware = run("system_profiler", &["-timeout", "5", "SPHardwareDataType"]);
        if let Some(text) = &hardware {
            specs.model_name = profile_value(text, "Model Name").unwrap_or_default();
            specs.firmware = profile_value(text, "System Firmware Version").unwrap_or_default();
        }

        // Apple silicon reports "hw.optional.arm64" and splits cores by performance
        // level; Intel Macs expose a single cache/frequency set instead.
        let apple_silicon = is_apple_silicon();
        let cpu = &mut specs.cpu;
        cpu.name = sysctl("machdep.cpu.brand_string")
            .or_else(|| {
                hardware
                    .as_deref()
                    .and_then(|t| profile_value(t, "Processor Name"))
            })
            .or_else(|| hardware.as_deref().and_then(|t| profile_value(t, "Chip")))
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
        if let Some(text) = run("system_profiler", &["-timeout", "5", "SPMemoryDataType"]) {
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
        if let Some(text) = run("system_profiler", &["-timeout", "5", "SPDisplaysDataType"]) {
            specs.graphics = parse_graphics_profile(&text);
        }
        if let Some(text) = run("system_profiler", &["-timeout", "5", "SPNVMeDataType"]) {
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
            specs.cpu.current_mhz = Some(cpu.frequency()).filter(|v| *v > 0);
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

/// PCI vendor/device identifiers from a Windows device instance path, such as
/// `PCI\VEN_10DE&DEV_1F08&SUBSYS_...`. Unrecognised paths stay unchanged.
#[cfg(any(target_os = "windows", test))]
fn pci_ids(device_path: &str) -> String {
    fn field(path: &str, key: &str) -> Option<String> {
        let start = path.find(key)? + key.len();
        let value: String = path[start..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        (value.len() == 4).then(|| value.to_uppercase())
    }
    match (field(device_path, "VEN_"), field(device_path, "DEV_")) {
        (Some(vendor), Some(device)) => format!("{vendor}:{device}"),
        _ => String::new(),
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
$caches = @(Get-CimInstance Win32_CacheMemory -ErrorAction SilentlyContinue)
$memArray = Get-CimInstance Win32_PhysicalMemoryArray -ErrorAction SilentlyContinue | Select-Object -First 1
$cpuClock = (Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0' -ErrorAction SilentlyContinue).'~MHz'
$l1dKb = [uint64]($caches | Where-Object { $_.Level -eq 3 } | Measure-Object -Property MaxCacheSize -Maximum).Maximum
if (-not $l1dKb) { $l1dKb = [uint64]($caches | Where-Object { $_.Level -eq 3 } | Measure-Object -Property InstalledSize -Maximum).Maximum }
$l1iKb = 0
$gpuReg = @(Get-ChildItem 'HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}' -ErrorAction SilentlyContinue | ForEach-Object {
  $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
  if ($p.DriverDesc) { [pscustomobject]@{ name = [string]$p.DriverDesc; bios = [string]$p.BiosVersion; bios_date = [string]$p.BiosDate; memory = [uint64]$p.'HardwareInformation.qwMemorySize' } }
})
$pnpDisplay = @(Get-CimInstance Win32_PnPEntity -Filter "PNPClass='Display'" -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{ id = [string]$_.PNPDeviceID; location = [string]$_.LocationInformation } })
$gpus = @(Get-CimInstance Win32_VideoController | ForEach-Object {
  $pnp = [string]$_.PNPDeviceID
  $location = ($pnpDisplay | Where-Object { $_.id -eq $pnp } | Select-Object -First 1).location
  [pscustomobject]@{
    name = [string]$_.Name
    vendor = [string]$_.AdapterCompatibility
    chipset = [string]$_.VideoProcessor
    pnp = $pnp
    pci = [string]$location
    driver = [string]$_.DriverVersion
    driver_date = [string]$_.DriverDate
    vram = [uint64]$_.AdapterRAM
    mode = if ($_.CurrentHorizontalResolution -and $_.CurrentVerticalResolution) { "$($_.CurrentHorizontalResolution) x $($_.CurrentVerticalResolution)" } else { [string]$_.VideoModeDescription }
    refresh = [string]$_.CurrentRefreshRate
    status = [string]$_.Status
  }
})
$chassis = Get-CimInstance Win32_SystemEnclosure -ErrorAction SilentlyContinue | Select-Object -First 1
$chassisCode = "$(@($chassis.ChassisTypes)[0])"
$chassisName = switch ($chassisCode) { '3' {'Desktop'} '4' {'Low-profile desktop'} '5' {'Pizza box'} '6' {'Mini tower'} '7' {'Tower'} '8' {'Portable'} '9' {'Laptop'} '10' {'Notebook'} '11' {'Handheld'} '12' {'Docking station'} '13' {'All-in-one'} '14' {'Sub-notebook'} '15' {'Space-saving'} '16' {'Lunch box'} '17' {'Main system chassis'} '23' {'Rack mount'} '24' {'Sealed case'} '30' {'Tablet'} '31' {'Convertible'} '32' {'Detachable'} '35' {'Mini PC'} '36' {'Stick PC'} default {''} }
$bat = Get-CimInstance Win32_Battery | Select-Object -First 1
$mem = @(Get-CimInstance Win32_PhysicalMemory | ForEach-Object {
  $form = switch ("$($_.FormFactor)") { '8' { 'DIMM' } '12' { 'SODIMM' } '13' { 'SRIMM' } '14' { 'SMDIMM' } default { '' } }
  [pscustomobject]@{ slot = [string]$_.DeviceLocator; bank = [string]$_.BankLabel; size = [uint64]$_.Capacity; type = [uint64]$_.SMBIOSMemoryType; speed = [uint64]$_.Speed; configured = [uint64]$_.ConfiguredClockSpeed; manufacturer = [string]$_.Manufacturer; part = [string]$_.PartNumber; serial = [string]$_.SerialNumber; form = $form; data_width = [uint64]$_.DataWidth; total_width = [uint64]$_.TotalWidth; voltage = [uint64]$_.ConfiguredVoltage; min_voltage = [uint64]$_.MinVoltage; max_voltage = [uint64]$_.MaxVoltage }
})
$volumes = @(Get-Volume | Where-Object { $_.DriveLetter } | ForEach-Object { [pscustomobject]@{ letter = [string]$_.DriveLetter; fs = [string]$_.FileSystem; health = [string]$_.HealthStatus; size = [uint64]$_.Size; free = [uint64]$_.SizeRemaining } })
# The Storage cmdlets (Get-PhysicalDisk, Get-StorageReliabilityCounter) take
# about 12 s on a typical laptop and the reliability counters are refused to
# standard users, so the same inventory comes from the CIM class directly.
$phys = @(Get-CimInstance -Namespace root/Microsoft/Windows/Storage -ClassName MSFT_PhysicalDisk)
# Loading the Storage module (Get-Volume above) rewrites these as friendly
# strings, while a bare CIM query returns the numeric codes; accept both.
$disks = @($phys | ForEach-Object {
  $name = [string]$_.FriendlyName
  $size = [uint64]$_.Size
  $media = "$($_.MediaType)"
  switch ($media) { '3' { $media = 'HDD' } '4' { $media = 'SSD' } '5' { $media = 'SCM' } }
  $bus = "$($_.BusType)"
  switch ($bus) { '3' { $bus = 'ATA' } '7' { $bus = 'USB' } '8' { $bus = 'RAID' } '10' { $bus = 'SAS' } '11' { $bus = 'SATA' } '12' { $bus = 'SD' } '17' { $bus = 'NVMe' } }
  $health = "$($_.HealthStatus)"
  switch ($health) { '0' { $health = 'Healthy' } '1' { $health = 'Warning' } '2' { $health = 'Unhealthy' } }
  [pscustomobject]@{ model = $name; media = $media; bus = $bus; health = $health; size = $size }
})
$reliability = @()
$diskdrives = @(Get-CimInstance Win32_DiskDrive | ForEach-Object { [pscustomobject]@{ model = [string]$_.Model; firmware = [string]$_.FirmwareRevision; serial = [string]$_.SerialNumber; interface = [string]$_.InterfaceType; size = [uint64]$_.Size } })
$network = @(Get-NetAdapter | ForEach-Object { [pscustomobject]@{ name = [string]$_.Name; description = [string]$_.InterfaceDescription; media = [string]$_.PhysicalMediaType; status = [string]$_.Status; speed = [string]$_.LinkSpeed } })
$bluetooth = @(Get-PnpDevice -Class Bluetooth | ForEach-Object { [pscustomobject]@{ name = [string]$_.FriendlyName; status = [string]$_.Status } })
$decode = { param($arr) (($arr | Where-Object { $_ -gt 0 } | ForEach-Object { [char]$_ }) -join '').Trim() }
$monitors = @(Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorID -ErrorAction SilentlyContinue | ForEach-Object {
  $m = $_
  [pscustomobject]@{ manufacturer = (& $decode $m.ManufacturerName); model = (& $decode $m.UserFriendlyName); serial = (& $decode $m.SerialNumberID); year = [uint64]$m.YearOfManufacture; week = [uint64]$m.WeekOfManufacture }
})
$drivers = @(Get-CimInstance Win32_PnPSignedDriver -ErrorAction SilentlyContinue | Where-Object { $_.DeviceClass -in @('DISPLAY','NET','HDC','SCSIADAPTER','USB','BLUETOOTH','BATTERY','MONITOR','MEDIA','MOUSE','KEYBOARD','FIRMWARE','CAMERA') } | Sort-Object DeviceClass, DeviceName | Select-Object -First 300 | ForEach-Object {
  [pscustomobject]@{ name = [string]$_.DeviceName; class = [string]$_.DeviceClass; version = [string]$_.DriverVersion; date = [string]$_.DriverDate; provider = [string]$_.DriverProviderName; signed = [bool]$_.IsSigned }
})
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
  board_version = [string]$board.Version
  chassis = $chassisName
  cpu_description = [string]$cpu.Description
  cpu_id = [string]$cpu.ProcessorId
  cpu_instructions = [string]$cpu.InstructionSet
  cpu_virtualization = [bool]$cpu.VirtualizationFirmwareEnabled
  cpu_current_clock = [uint64]$cpuClock
  cpu_threads = [uint64]$cpu.ThreadCount
  cpu_max_clock = [uint64]$cpu.MaxClockSpeed
  cpu_l1_data_kb = $l1dKb
  cpu_l1_instruction_kb = $l1iKb
  cpu_l2_kb = [uint64]$cpu.L2CacheSize
  cpu_l3_kb = [uint64]$cpu.L3CacheSize
  cpu_socket = [string]$cpu.SocketDesignation
  cpu_stepping = [string]$cpu.Stepping
  mem_slots = [uint64]$memArray.MemoryDevices
  mem_max_capacity_kb = [uint64]$memArray.MaxCapacity
  mem_ecc = [uint64]$memArray.MemoryErrorCorrection
  gpus = $gpus
  gpu_reg = $gpuReg
  display = if (@($gpus).Count -gt 0) { $g0 = @($gpus)[0]; if ($g0.mode) { "$($g0.mode)" + $(if ($g0.refresh) { " @ $($g0.refresh) Hz" } else { '' }) } else { '' } } else { '' }
  monitors = $monitors
  drivers = $drivers
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
        // PowerShell start-up alone costs a couple of seconds here, so the
        // query overlaps the sysinfo pass instead of following it.
        let pending = std::thread::spawn(|| query(SCRIPT));
        let mut specs = super::portable::collect();
        specs.sources = "sysinfo · Windows CIM/WMI (read-only)".into();
        if !specs.kernel.is_empty() {
            specs.kernel = format!("build {}", specs.kernel);
        }
        if let Some(value) = pending.join().ok().flatten() {
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
        if specs.cpu.l1_data_bytes.is_none() && specs.cpu.l1_instruction_bytes.is_none() {
            specs
                .unavailable
                .push("L1 cache sizes were not reported by the firmware.".into());
        }
        if specs.cpu.cache_line_bytes.is_none() {
            specs.unavailable.push(
                "Cache-line size is not exposed by the standard Windows processor class.".into(),
            );
        }
        specs.unavailable.push(
            "Memory timings (CL, tRCD, tRP, tRAS) and XMP/EXPO profiles need raw SPD reads, which are not available to a standard user."
                .into(),
        );
        specs.unavailable.push(
            "PCIe link width and generation per adapter are not exposed without a vendor library; the bus location is shown instead."
                .into(),
        );
        specs.unavailable.push(
            "Drive temperature and wear counters are not read on Windows; the storage provider refuses them to a standard user."
                .into(),
        );
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
        specs.board_vendor = text(v.get("board_manufacturer"));
        specs.board_model = text(v.get("board_product"));
        specs.board_version = text(v.get("board_version"));
        specs.chassis = text(v.get("chassis"));

        // `Win32_Processor` reports cache sizes in KiB and the rated clock.
        let cache = |key: &str| {
            v.get(key)
                .and_then(Value::as_u64)
                .filter(|kb| *kb > 0)
                .map(|kb| kb.saturating_mul(1024))
        };
        if specs.cpu.l1_data_bytes.is_none() {
            specs.cpu.l1_data_bytes = cache("cpu_l1_data_kb");
        }
        if specs.cpu.l1_instruction_bytes.is_none() {
            specs.cpu.l1_instruction_bytes = cache("cpu_l1_instruction_kb");
        }
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
        specs.cpu.current_mhz = v
            .get("cpu_current_clock")
            .and_then(Value::as_u64)
            .filter(|mhz| *mhz > 0);
        specs.cpu.socket = text(v.get("cpu_socket"));
        specs.cpu.stepping = text(v.get("cpu_stepping"));
        specs.cpu.description = text(v.get("cpu_description"));
        specs.cpu.processor_id = text(v.get("cpu_id"));
        specs.cpu.instructions = text(v.get("cpu_instructions"));
        specs.cpu.virtualization = v.get("cpu_virtualization").and_then(Value::as_bool);

        let registry = v
            .get("gpu_reg")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for gpu in v
            .get("gpus")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let name = text(gpu.get("name"));
            if name.is_empty() {
                continue;
            }
            let matched = registry.iter().find(|entry| {
                let entry_name = text(entry.get("name"));
                !entry_name.is_empty()
                    && (entry_name.eq_ignore_ascii_case(&name)
                        || name.contains(&entry_name)
                        || entry_name.contains(&name))
            });
            // The registry value is the adapter's dedicated memory; the CIM
            // AdapterRAM field saturates on cards with more than 4 GiB.
            let vram = matched
                .and_then(|entry| entry.get("memory"))
                .and_then(Value::as_u64)
                .filter(|bytes| *bytes > 0)
                .or_else(|| {
                    gpu.get("vram")
                        .and_then(Value::as_u64)
                        .filter(|bytes| *bytes > 0)
                });
            let device_id = text(gpu.get("pnp"));
            specs.graphics.push(GraphicsSpec {
                name,
                chipset: text(gpu.get("chipset")),
                vendor: text(gpu.get("vendor")),
                device_id: pci_ids(&device_id),
                vram_bytes: vram,
                api: String::new(),
                bus: text(gpu.get("pci")),
                driver: text(gpu.get("driver")),
                driver_date: cim_date(&text(gpu.get("driver_date"))),
                pci: text(gpu.get("pci")),
                refresh: {
                    let value = text(gpu.get("refresh"));
                    if !value.is_empty() {
                        format!("{value} Hz")
                    } else {
                        Default::default()
                    }
                },
                status: text(gpu.get("status")),
                bios: matched
                    .map(|entry| text(entry.get("bios")))
                    .unwrap_or_default(),
                bios_date: matched
                    .map(|entry| text(entry.get("bios_date")))
                    .unwrap_or_default(),
                current_mode: text(gpu.get("mode")),
            });
        }
        if let Some(primary) = specs.graphics.first() {
            specs.gpu_driver = primary.driver.clone();
            specs.gpu_driver_date = primary.driver_date.clone();
        }
        specs.display = text(v.get("display"));

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
                    // Firmware often reports an all-zero placeholder.
                    serial: if text(module.get("serial"))
                        .chars()
                        .any(|c| c.is_ascii_alphanumeric() && c != '0')
                    {
                        text(module.get("serial"))
                    } else {
                        Default::default()
                    },
                    configured_mhz: module
                        .get("configured")
                        .and_then(Value::as_u64)
                        .filter(|speed| *speed > 0),
                    form_factor: text(module.get("form")),
                    bank: text(module.get("bank")),
                    data_width: module
                        .get("data_width")
                        .and_then(Value::as_u64)
                        .filter(|bits| *bits > 0),
                    total_width: module
                        .get("total_width")
                        .and_then(Value::as_u64)
                        .filter(|bits| *bits > 0),
                    configured_voltage_mv: module
                        .get("voltage")
                        .and_then(Value::as_u64)
                        .filter(|mv| *mv > 0),
                    min_voltage_mv: module
                        .get("min_voltage")
                        .and_then(Value::as_u64)
                        .filter(|mv| *mv > 0),
                    max_voltage_mv: module
                        .get("max_voltage")
                        .and_then(Value::as_u64)
                        .filter(|mv| *mv > 0),
                });
            }
        }
        specs.memory.slots_used = Some(specs.memory.modules.len() as u32);
        specs.memory.slots_total = v
            .get("mem_slots")
            .and_then(Value::as_u64)
            .filter(|slots| *slots > 0)
            .map(|slots| slots as u32);
        specs.memory.max_capacity_bytes = v
            .get("mem_max_capacity_kb")
            .and_then(Value::as_u64)
            .filter(|kb| *kb > 0)
            .map(|kb| kb.saturating_mul(1024));
        // SMBIOS memory-array error correction: 5/6/7 are ECC schemes, 3 is none.
        specs.memory.ecc = match v.get("mem_ecc").and_then(Value::as_u64) {
            Some(5..=7) => Some(true),
            Some(3) => Some(false),
            _ => specs.memory.modules.iter().find_map(|module| {
                match (module.data_width, module.total_width) {
                    (Some(data), Some(total)) if total > data => Some(true),
                    (Some(_), Some(_)) => Some(false),
                    _ => None,
                }
            }),
        };
        // A module whose rating is the JEDEC default may already be correct.
        for module in &mut specs.memory.modules {
            if module.configured_mhz.is_none() {
                module.configured_mhz = module.speed_mhz;
            }
        }

        if let Some(monitors) = v.get("monitors").and_then(Value::as_array) {
            for monitor in monitors {
                let model = text(monitor.get("model"));
                let manufacturer = text(monitor.get("manufacturer"));
                if model.is_empty() && manufacturer.is_empty() {
                    continue;
                }
                specs.monitors.push(MonitorSpec {
                    manufacturer,
                    model,
                    serial: if text(monitor.get("serial"))
                        .chars()
                        .any(|c| c.is_ascii_alphanumeric() && c != '0')
                    {
                        text(monitor.get("serial"))
                    } else {
                        Default::default()
                    },
                    year: monitor
                        .get("year")
                        .and_then(Value::as_u64)
                        .filter(|year| *year > 0)
                        .map(|year| year as u32),
                    week: monitor
                        .get("week")
                        .and_then(Value::as_u64)
                        .filter(|week| (1..=53).contains(week))
                        .map(|week| week as u32),
                });
            }
        }

        if let Some(drivers) = v.get("drivers").and_then(Value::as_array) {
            for driver in drivers {
                let name = text(driver.get("name"));
                if name.is_empty() {
                    continue;
                }
                let class_name = text(driver.get("class"));
                // The driver store repeats identical generic entries per device.
                if specs
                    .drivers
                    .iter()
                    .any(|known| known.name == name && known.class_name == class_name)
                {
                    continue;
                }
                specs.drivers.push(DriverSpec {
                    name,
                    class_name,
                    version: text(driver.get("version")),
                    date: cim_date(&text(driver.get("date"))),
                    provider: text(driver.get("provider")),
                    signed: driver.get("signed").and_then(Value::as_bool),
                });
            }
        }

        if let Some(adapters) = v.get("network").and_then(Value::as_array) {
            for adapter in adapters {
                let name = text(adapter.get("name"));
                let description = text(adapter.get("description"));
                if name.is_empty() && description.is_empty() {
                    continue;
                }
                specs.network_adapters.push(NetworkAdapterSpec {
                    name,
                    description,
                    status: text(adapter.get("status")),
                    speed: text(adapter.get("speed")),
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
            let matched = diskdrives
                .iter()
                .find(|drive| text(drive.get("model")) == model);
            let firmware = matched
                .map(|drive| text(drive.get("firmware")))
                .unwrap_or_default();
            let serial = matched
                .map(|drive| text(drive.get("serial")))
                .unwrap_or_default();
            specs.drives.push(DriveSpec {
                model,
                firmware,
                protocol: text(disk.get("bus")),
                link: text(disk.get("media")),
                trim: None,
                smart,
                serial,
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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn profile_value(text: &str, key: &str) -> Option<String> {
    run_section(text)
        .into_iter()
        .find(|(_, k, v)| k == key && !v.is_empty())
        .map(|(_, _, v)| v)
}

/// Decimal units are used by `system_profiler`; exact byte counts win when present.
/// `system_profiler` localises its values; accept the common English and
/// Chinese spellings instead of treating "Enabled"/"Yes" as the only truth.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn profile_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "enabled" | "on" | "是" | "开启" | "啟用" => Some(true),
        "no" | "false" | "disabled" | "off" | "否" | "关闭" | "關閉" => Some(false),
        _ => None,
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
    let number: f64 = parts.next()?.replace(',', ".").parse().ok()?;
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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_mhz(value: &str) -> Option<u64> {
    value
        .split_whitespace()
        .find_map(|part| part.parse::<u64>().ok())
        .filter(|v| *v > 0)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_memory_profile(text: &str) -> MemoryProfile {
    let mut profile = MemoryProfile::default();
    let mut current: Option<MemoryModule> = None;
    for (indent, key, value) in run_section(text) {
        match key.as_str() {
            "ECC" => profile.ecc = profile_flag(&value),
            "Upgradeable Memory" => profile.upgradeable = profile_flag(&value),
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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_diskutil_info(text: &str) -> Option<VolumeHealth> {
    let mut health = VolumeHealth::default();
    for (_, key, value) in run_section(text) {
        match key.as_str() {
            "Device / Media Name" => health.media = value,
            "Protocol" => health.protocol = value,
            "SMART Status" => health.smart = value,
            "Solid State" => health.solid_state = profile_flag(&value),
            _ => {}
        }
    }
    if health.smart.is_empty() && health.protocol.is_empty() && health.media.is_empty() {
        return None;
    }
    Some(health)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_board_id(text: &str) -> Option<String> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("\"board-id\" = "))?;
    let raw = line.split_once("= ")?.1.trim();
    let value = raw
        .strip_prefix("<\"")
        .and_then(|v| v.strip_suffix("\">"))
        .or_else(|| raw.strip_prefix('"').and_then(|v| v.strip_suffix('"')))?;
    if value.is_empty() || value.len() > 128 {
        None
    } else {
        Some(value.to_owned())
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_wifi_interface(text: &str) -> Option<String> {
    let mut wifi = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(port) = line.strip_prefix("Hardware Port: ") {
            wifi = port.trim().eq_ignore_ascii_case("wi-fi")
                || port.trim().eq_ignore_ascii_case("wifi")
                || port.trim().eq_ignore_ascii_case("wireless");
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

/// `system_profiler -json` has shipped both a top-level `SPBluetoothDataType`
/// array and a nested `Bluetooth` object across macOS versions.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn bluetooth_root(value: &serde_json::Value) -> Option<&serde_json::Value> {
    value
        .get("SPBluetoothDataType")
        .or_else(|| value.get("Bluetooth")?.get("SPBluetoothDataType"))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn first_profile_entry(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Array(items) => items.first(),
        serde_json::Value::Object(_) => Some(value),
        _ => None,
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_bluetooth(text: &str) -> Option<BluetoothSpec> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let root = bluetooth_root(&value)?;
    let entry = first_profile_entry(root)?;
    let controller = entry.get("controller_properties")?;
    let state = match controller.get("controller_state").and_then(|v| v.as_str()) {
        Some("attrib_on") => Some(true),
        Some("attrib_off") => Some(false),
        _ => None,
    };
    let devices = entry
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

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
    #[cfg(target_os = "windows")]
    use serde_json::Value;

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
        assert_eq!(
            parse_board_id("  \"board-id\" = \"Mac-TEST\"\n"),
            Some("Mac-TEST".into())
        );
        assert_eq!(parse_board_id("  \"model\" = <\"Mac-TEST\">\n"), None);
    }

    #[test]
    fn profile_flags_accept_common_localised_values() {
        assert_eq!(profile_flag("Enabled"), Some(true));
        assert_eq!(profile_flag("yes"), Some(true));
        assert_eq!(profile_flag("是"), Some(true));
        assert_eq!(profile_flag("Disabled"), Some(false));
        assert_eq!(profile_flag("关闭"), Some(false));
        assert_eq!(profile_flag("unknown"), None);
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
        let nested = parse_bluetooth(r#"{"Bluetooth":{"SPBluetoothDataType":{"controller_properties":{"controller_state":"attrib_off","controller_transport":"USB"}}}}"#).unwrap();
        assert_eq!(nested.state, Some(false));
        assert_eq!(nested.transport, "USB");
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
    fn pci_identifiers_are_extracted_from_device_paths() {
        assert_eq!(
            pci_ids(r"PCI\VEN_10DE&DEV_2786&SUBSYS_88881043&REV_A1\4&2D8B7C3&0&0008"),
            "10DE:2786"
        );
        assert_eq!(pci_ids("ACPI\\PNP0A08\\0"), "");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_inventory_reads_gpus_slots_monitors_and_drivers() {
        let value: Value = serde_json::from_str(
            r#"{
              "gpus": [{
                "name": "NVIDIA GeForce RTX 4070",
                "vendor": "NVIDIA",
                "chipset": "GeForce RTX 4070",
                "pnp": "PCI\\VEN_10DE&DEV_2786&SUBSYS_88881043&REV_A1\\4&2D8B7C3&0&0008",
                "pci": "PCI bus 1, device 0, function 0",
                "driver": "31.0.15.5222",
                "driver_date": "20240515000000.000000-000",
                "vram": 4294967295,
                "mode": "2560 x 1440 x 32",
                "refresh": "165",
                "status": "OK"
              }],
              "display": "2560 x 1440 x 32 @ 165 Hz",
              "gpu_reg": [{
                "name": "NVIDIA GeForce RTX 4070",
                "bios": "95.04.3C.40.8C",
                "bios_date": "2023-04-01",
                "memory": 12884901888
              }],
              "mem_slots": 4,
              "mem_max_capacity_kb": 67108864,
              "mem_ecc": 3,
              "memory": [{
                "slot": "DIMM 1", "bank": "BANK 0", "size": 17179869184,
                "type": 34, "speed": 6000, "configured": 5200,
                "manufacturer": "G.Skill", "part": "F5-6000", "serial": "1234",
                "form": "DIMM", "data_width": 64, "total_width": 64,
                "voltage": 1350, "min_voltage": 1200, "max_voltage": 1350
              }],
              "monitors": [{
                "manufacturer": "DEL", "model": "DELL U2720Q",
                "serial": "ABC123", "year": 2021, "week": 14
              }],
              "drivers": [{
                "name": "NVIDIA GeForce RTX 4070", "class": "DISPLAY",
                "version": "31.0.15.5222",
                "date": "20240515000000.000000-000",
                "provider": "NVIDIA", "signed": true
              }],
              "network": [{
                "name": "Ethernet", "description": "Intel I225-V",
                "media": "802.3", "status": "Up", "speed": "2.5 Gbps"
              }]
            }"#,
        )
        .unwrap();
        let mut specs = SystemSpecs::default();
        windows::apply(&mut specs, &value);
        assert_eq!(specs.graphics.len(), 1);
        assert_eq!(specs.graphics[0].device_id, "10DE:2786");
        assert_eq!(specs.graphics[0].vram_bytes, Some(12_884_901_888));
        assert_eq!(specs.graphics[0].bios, "95.04.3C.40.8C");
        assert_eq!(specs.graphics[0].bus, "PCI bus 1, device 0, function 0");
        assert_eq!(specs.graphics[0].refresh, "165 Hz");
        assert_eq!(specs.memory.slots_total, Some(4));
        assert_eq!(specs.memory.slots_used, Some(1));
        assert_eq!(specs.memory.modules[0].configured_mhz, Some(5200));
        assert_eq!(specs.memory.modules[0].configured_voltage_mv, Some(1350));
        assert_eq!(specs.memory.ecc, Some(false));
        assert_eq!(specs.monitors[0].model, "DELL U2720Q");
        assert_eq!(specs.monitors[0].week, Some(14));
        assert_eq!(specs.drivers.len(), 1);
        assert_eq!(specs.drivers[0].signed, Some(true));
        assert_eq!(specs.network_adapters[0].speed, "2.5 Gbps");
        assert_eq!(specs.display, "2560 x 1440 x 32 @ 165 Hz");
    }

    #[test]
    fn inventory_cache_roundtrips_and_rejects_other_versions() {
        let mut specs = SystemSpecs::default();
        specs.cpu.name = "Test Processor".into();
        specs.memory.total_bytes = Some(32 * 1024 * 1024 * 1024);
        specs.drives.push(DriveSpec {
            model: "Test SSD".into(),
            smart: "Healthy".into(),
            ..Default::default()
        });
        let unique = format!(
            "hardware-hub-cache-test-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        );
        let path = std::env::temp_dir().join(unique);
        save_cache(&path, &specs).expect("cache write");
        let (loaded, cached_at) = load_cache(&path).expect("cache read");
        assert_eq!(loaded.cpu.name, "Test Processor");
        assert_eq!(loaded.drives[0].smart, "Healthy");
        assert!(cached_at > 0);

        // A cache written by a future schema must be ignored, not misread.
        let text = std::fs::read_to_string(&path).unwrap();
        let bumped = text.replacen("\"version\":1", "\"version\":99", 1);
        assert_ne!(text, bumped);
        std::fs::write(&path, bumped).unwrap();
        assert!(load_cache(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn capacities_use_exact_bytes_when_available() {
        assert_eq!(
            parse_capacity("512.11 GB (512,110,190,592 bytes)"),
            Some(512_110_190_592)
        );
        assert_eq!(parse_capacity("256.0 GB"), Some(256_000_000_000));
        assert_eq!(parse_capacity("256,0 GB"), Some(256_000_000_000));
        assert_eq!(parse_capacity("1536 MB"), Some(1_536_000_000));
        assert_eq!(parse_capacity("unknown"), None);
    }
}
