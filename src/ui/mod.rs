//! Modular UI layer for Hardware Hub v2.
//!
//! Organized by functional purpose:
//! - `theme`: Obsidian Graphite tokens, semantic palette, formatters, dark titlebar
//! - `widgets`: Liquid-fill CPU tiles, multi-DIMM visualizer, HWMonitor range bars,
//!   circular gauges, virtualized tables, and accessible status chips
//! - `shell`: Compact/Expanded window modes, Slack dual-rail nav, Ctrl+K palette,
//!   top toolbar, status bar, and Floating Monitor HUD
//! - `monitor`: Modern HWMonitor live sensor tree + slide-in trend inspector
//! - `hardware`: CPU-Z / GPU-Z / CrystalDiskInfo deep hardware inspection views
//! - `stress_lab`: Real-world Stress Lab (5m–24h / Continuous / Pass-count) cockpit
//! - `diagnostics`: Processes, PresentMon Frame Analysis, Reports & Run Comparison

pub mod diagnostics;
pub mod hardware;
pub mod monitor;
pub mod shell;
pub mod stress_lab;
pub mod theme;
pub mod widgets;

use hardware_hub::stress::Kind;

/// Four top-level Slack-rail workspaces so navigation never overflows.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Workspace {
    Monitor,
    Hardware,
    Stress,
    Diagnostics,
}

impl Workspace {
    pub const ALL: [Self; 4] = [
        Self::Monitor,
        Self::Hardware,
        Self::Stress,
        Self::Diagnostics,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Monitor => "Live Monitor",
            Self::Hardware => "Hardware Dive",
            Self::Stress => "Stress Lab",
            Self::Diagnostics => "Diagnostics",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Monitor => "Monitor",
            Self::Hardware => "Hardware",
            Self::Stress => "Stress",
            Self::Diagnostics => "Reports",
        }
    }

    pub fn default_page(self) -> Page {
        match self {
            Self::Monitor => Page::Overview,
            Self::Hardware => Page::Cpu,
            Self::Stress => Page::Tests,
            Self::Diagnostics => Page::Processes,
        }
    }

    pub fn pages(self) -> &'static [Page] {
        match self {
            Self::Monitor => &[Page::Overview, Page::Thermal],
            Self::Hardware => &[
                Page::Details,
                Page::Cpu,
                Page::Gpu,
                Page::Memory,
                Page::Storage,
                Page::Motherboard,
                Page::Network,
                Page::Bluetooth,
                Page::Drivers,
            ],
            Self::Stress => &[Page::Tests],
            Self::Diagnostics => &[Page::Processes, Page::Frames, Page::Reports],
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Page {
    Overview,
    Cpu,
    Gpu,
    Memory,
    Storage,
    Thermal,
    Motherboard,
    Drivers,
    Network,
    Bluetooth,
    Details,
    Processes,
    Frames,
    Tests,
    Reports,
}

impl Page {
    pub const ALL: [Self; 15] = [
        Self::Overview,
        Self::Thermal,
        Self::Details,
        Self::Cpu,
        Self::Gpu,
        Self::Memory,
        Self::Storage,
        Self::Motherboard,
        Self::Network,
        Self::Bluetooth,
        Self::Drivers,
        Self::Tests,
        Self::Processes,
        Self::Frames,
        Self::Reports,
    ];

    pub fn workspace(self) -> Workspace {
        match self {
            Self::Overview | Self::Thermal => Workspace::Monitor,
            Self::Details
            | Self::Cpu
            | Self::Gpu
            | Self::Memory
            | Self::Storage
            | Self::Motherboard
            | Self::Drivers
            | Self::Network
            | Self::Bluetooth => Workspace::Hardware,
            Self::Tests => Workspace::Stress,
            Self::Processes | Self::Frames | Self::Reports => Workspace::Diagnostics,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Cpu => "CPU",
            Self::Gpu => "GPU",
            Self::Memory => "Memory",
            Self::Storage => "Storage",
            Self::Thermal => "Thermals / fans",
            Self::Motherboard => "Parts & board",
            Self::Drivers => "Drivers",
            Self::Network => "Network / Wi-Fi",
            Self::Bluetooth => "Bluetooth",
            Self::Details => "Details",
            Self::Processes => "Processes",
            Self::Frames => "Frame analysis",
            Self::Tests => "Stress tests",
            Self::Reports => "Reports",
        }
    }

    pub fn nav_label(self) -> &'static str {
        match self {
            Self::Overview => "All Sensors",
            Self::Thermal => "Thermals & Alerts",
            Self::Details => "System Summary",
            Self::Cpu => "Processor (CPU)",
            Self::Gpu => "Graphics (GPU)",
            Self::Memory => "Memory (DIMMs)",
            Self::Storage => "Storage (SMART)",
            Self::Motherboard => "Board & Displays",
            Self::Drivers => "OS Drivers",
            Self::Network => "Network & Wi-Fi",
            Self::Bluetooth => "Bluetooth",
            Self::Processes => "Processes",
            Self::Frames => "Frame Analysis",
            Self::Tests => "Stress Cockpit",
            Self::Reports => "Reports & Compare",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Overview => "Live Monitor",
            Self::Tests => "Stress Lab",
            _ => self.label(),
        }
    }

    pub fn group(self) -> &'static str {
        match self.workspace() {
            Workspace::Monitor => "LIVE MONITOR",
            Workspace::Hardware => "HARDWARE DIVE",
            Workspace::Stress => "STRESS LAB",
            Workspace::Diagnostics => "DIAGNOSTICS & REPORTS",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            Self::Overview => "1",
            Self::Cpu => "2",
            Self::Gpu => "3",
            Self::Memory => "4",
            Self::Storage => "5",
            Self::Thermal => "6",
            Self::Details => "7",
            Self::Processes => "8",
            Self::Reports => "9",
            Self::Motherboard => "Shift+1",
            Self::Drivers => "Shift+2",
            Self::Network => "Shift+3",
            Self::Bluetooth => "Shift+4",
            Self::Frames => "Shift+5",
            Self::Tests => "Shift+6",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Overview => "HWMonitor-style live telemetry, liquid core load and click-to-plot sensor tree.",
            Self::Cpu => "CPU-Z style processor identity, liquid core topology, caches and stability check.",
            Self::Gpu => "GPU-Z style adapters, driver versions, wgpu device limits and compute check.",
            Self::Memory => "Live RAM usage, scalable DIMM slot visualizer and MemTest-style pattern check.",
            Self::Storage => "CrystalDiskInfo-style drive health, volumes, I/O counters and SMART import.",
            Self::Thermal => "Thermal sensors, multi-series history graph, CSV export and advisory alerts.",
            Self::Motherboard => "Mainboard, BIOS/UEFI firmware, EDID displays and physical network adapters.",
            Self::Drivers => "Virtualized read-only inventory of installed hardware drivers.",
            Self::Network => "Per-interface OS traffic counters and optional TCP reachability diagnostic.",
            Self::Bluetooth => "Read-only Bluetooth controller state and connected device inventory.",
            Self::Details => "Complete specification sheet collected from local read-only OS sources.",
            Self::Processes => "Virtualized live process list sorted by CPU or memory usage.",
            Self::Frames => "Offline PresentMon CSV frame-pacing analyzer (Avg FPS, 1% Low, P99).",
            Self::Tests => "Timed (5m–24h), Continuous, or Pass-Count stability verification with live thermal supervision.",
            Self::Reports => "Export system inventory, run summaries, and compare session checks side by side.",
        }
    }

    pub fn from_label(label: &str) -> Option<Page> {
        let key = label.trim().to_ascii_lowercase();
        Some(match key.as_str() {
            "overview" | "home" | "monitor" | "all sensors" => Page::Overview,
            "cpu" | "processor" | "processor (cpu)" => Page::Cpu,
            "gpu" | "graphics" | "graphics (gpu)" => Page::Gpu,
            "memory" | "ram" | "memory (dimms)" => Page::Memory,
            "storage" | "disk" | "volumes" | "storage (smart)" => Page::Storage,
            "thermal" | "thermals" | "thermals / fans" | "fans" | "thermals & alerts" => {
                Page::Thermal
            }
            "motherboard" | "board" | "parts" | "parts & board" | "board & displays" => {
                Page::Motherboard
            }
            "drivers" | "os drivers" => Page::Drivers,
            "network" | "network / wi-fi" | "wifi" | "wi-fi" | "network & wi-fi" => Page::Network,
            "bluetooth" => Page::Bluetooth,
            "details" | "system summary" => Page::Details,
            "processes" => Page::Processes,
            "frames" | "frame analysis" => Page::Frames,
            "tests" | "stress tests" | "stress" | "stress lab" | "stress cockpit" => Page::Tests,
            "reports" | "reports & compare" => Page::Reports,
            _ => return None,
        })
    }

    pub fn keywords(self) -> &'static str {
        match self {
            Self::Overview => "hwmonitor live sensors dashboard home overview",
            Self::Cpu => "cpuz processor cores threads cache avx clock frequency",
            Self::Gpu => "gpuz graphics video vram dx12 metal vulkan nvml",
            Self::Memory => "ram dimm slots ddr4 ddr5 ecc unified memory",
            Self::Storage => "crystaldiskinfo ssd nvme hdd smart disk volume",
            Self::Thermal => "temperature fans acpi smc hwmon alert csv",
            Self::Motherboard => "bios uefi board edid monitor display chassis",
            Self::Drivers => "driver store signed version provider",
            Self::Network => "wifi ethernet tcp ping latency bandwidth",
            Self::Bluetooth => "bt wireless controller devices",
            Self::Details => "specs summary system inventory hardware",
            Self::Processes => "task manager pid cpu memory usage",
            Self::Frames => "presentmon fps frametime p99 1% low",
            Self::Tests => "prime95 furmark memtest occt stress stability burnin",
            Self::Reports => "export json text certificate compare runs",
        }
    }
}

pub const STRESS_KINDS: [(Kind, &str, &str); 5] = [
    (Kind::Cpu, "CPU Compute", "Prime95-style ALU / FMA / Blend"),
    (Kind::Gpu, "GPU Check", "FurMark-style shader verify"),
    (Kind::Memory, "Memory", "MemTest-style byte patterns"),
    (Kind::Disk, "File I/O", "Owned scratch file readback"),
    (Kind::Combined, "Combined", "OCCT-style CPU + RAM load"),
];

pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Cpu => "CPU compute",
        Kind::Gpu => "GPU check",
        Kind::Memory => "Memory patterns",
        Kind::Disk => "File I/O",
        Kind::Combined => "Combined CPU + memory",
    }
}
