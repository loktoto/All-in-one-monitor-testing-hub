# Hardware Hub v2 — All-in-One UI/UX & Engineering RFC

> **Status:** Proposed Design & Architecture Blueprint (Awaiting Implementation Approval)  
> **Tracking Epic:** #2  
> **Sub-Issues:** #3 (Window Modes & Nav) · #4 (Liquid Fill & Hardware Scaling) · #5 (Real Stress Lab) · #6 (Cross-Platform Coverage) · #7 (Fast/Smooth Performance) · #8 (Semantic Colors & Polish)

---

## 1. Product Vision

**Hardware Hub v2** combines the best daily workflows of **HWMonitor, CPU-Z, GPU-Z, CrystalDiskInfo, Prime95, FurMark, MemTest, and OCCT** into **one fast, sleek, lightweight, cross-platform Rust desktop app** (Windows, macOS Intel & Apple Silicon, Linux).

- **Simple at a glance:** Opens cleanly like a modern HWMonitor — never covers your entire screen.
- **Deep when needed:** One click expands into CPU-Z / GPU-Z / CrystalDiskInfo inspection or a full Stress Lab cockpit.
- **100% local & honest:** No account, no cloud telemetry, no invasive kernel driver required. Every sensor displays its source (`SMC`, `hwmon`, `NVML`, `DXGI`, `ACPI`, `CIM`), and unavailable platform values are honestly marked rather than fabricated.

---

## 2. Window Modes & Ergonomics (Never Fullscreen by Default — #3)

Nobody wants a hardware monitor hijacking their entire display on launch. Remove `.with_maximized(true)` and introduce **three seamless modes**:

| Mode | Default Size | Purpose |
| :--- | :--- | :--- |
| **Compact HWMonitor Mode** | `~420 × 680 px` | Narrow dockable vertical strip for side-by-side monitoring next to games, IDEs, or browsers. Displays mini glance strip + collapsible sensor tree + click-to-expand inline sparkline. |
| **Expanded Workspace** | `~1120 × 720 px` | Slack-inspired dual-rail layout for Hardware Deep Dive, multi-sensor graphs, Stress Lab cockpit, and Reports. Remembers exact user size & position safely across monitors. |
| **Floating Mini HUD** | `~260 × 72 px` | Always-on-top translucent dark glass pill displaying user-pinned sensors (e.g., CPU %, RAM %, Peak °C + mini sparkline). |

### Slack-Inspired Dual-Rail Navigation
- **Primary Icon Rail (`52 px`):** Fixed to **4 workspaces** so it never overflows or clips on any DPI:
  1. **Live Monitor** (Modern HWMonitor tree + live chart inspector)
  2. **Hardware Dive** (CPU-Z / GPU-Z / CrystalDiskInfo / Board / Displays / Network / Bluetooth / Drivers)
  3. **Stress Lab** (Prime95 / FurMark / MemTest / Storage / Combined)
  4. **Diagnostics & Reports** (Processes, PresentMon Frame Analysis, Run Comparison, Exports)
- **Collapsible Sub-Navigation Tree (`196 px`):** Lists only the active workspace's categories; auto-collapses smoothly below `820 px` width.
- **Global `Ctrl+K` / `Cmd+K` Quick-Jump Palette:** Fuzzy-search any hardware tab, sensor key (`TC0D`, `TZ01`), process, or stress test instantly.
- **`Simple | Pro` Density Toggle:** Clean glanceable defaults for everyday users; high-density HWMonitor tables with raw keys and full Min/Max/Avg columns for enthusiasts.
- **Native Dark Titlebar:** Enable `DWMWA_USE_IMMERSIVE_DARK_MODE` on Windows and unified dark chrome on macOS.

---

## 3. Visual System, Liquid-Fill CPU Tiles & Extreme Hardware Scalability (#4, #8)

### Obsidian Graphite Design System
- **Surfaces:** Canvas `#111215` · Rails/Chrome `#16181D` · Cards `#1C1F26` · Hairline Borders `1px #2A2E39`
- **Strict Semantic Palette (paired with icons for accessibility):**
  - **Electric Blue `#2F81F7`** — Active nav, CPU load, primary actions
  - **Emerald `#30D158`** — `LIVE`, `HEALTHY`, `SMART VERIFIED`, `0 ERRORS`, normal thermals *(fixes current amber HEALTHY bug)*
  - **Violet `#A371F7`** — Memory / RAM telemetry
  - **Amber `#FF9F0A`** — Warming temps, `85°C` advisory limit, `CACHED` badge
  - **Crimson `#FF453A`** — `Stop Check (Esc)`, errors, critical alerts, `Record CSV` dot
- **Dynamic Version:** Always display `concat!("v", env!("CARGO_PKG_VERSION"))` (fixes hardcoded `v0.1.0`).

### Liquid / Water-Fill CPU Core Visualization (Scales 4T → 128T+)
Each logical core tile renders a smooth, critically-damped **liquid water-fill gauge** rising from the bottom proportional to real-time utilization (`0–100%`), colored from cool electric blue to bright cyan/amber under heavy load, with crisp tabular monospace `%` labels:

| Thread Count | Adaptive Layout Strategy |
| :--- | :--- |
| **1–16 threads** | Full liquid-fill tiles in a 4-column grid with core ID, large `%`, and optional P/E badge. |
| **17–32 threads** | Compact medium liquid-fill tiles (8 columns) grouped by **P-Cores / E-Cores** (Intel hybrid / Apple Silicon) or **CCD / NUMA** (AMD). |
| **33–128+ threads** | Auto-switch to **Compact Liquid Micro-Tiles / Heatmap** (with rich hover tooltip: Core ID, Cluster, Load %) + toggleable **Per-CCD Summary** and **Top Busiest Cores** strip; virtualized scroll if expanded. |

### Multi-DIMM / Unified Memory Scalability (1 Slot → 16+ Slots)
- **1–4 DIMM Slots:** Visual side-by-side DIMM stick cards showing Slot locator, Capacity, DDR generation, Speed (MT/s), Vendor, Voltage, Part Number, and ghosted Empty Slots.
- **5–16+ DIMM Slots (Workstation / Server / HEDT):** Seamless switch to a **Channel-Grouped Compact Rack Grid** (Channels A–H) or dense toggleable table with summary header (`Installed / Populated Slots / Channels / ECC`).
- **Apple Silicon / Soldered Unified Memory:** Dedicated **Unified Memory Architecture** card (Capacity, LPDDR type/bandwidth, Active / Wired / Compressed / Swap memory pressure bar) instead of fake DIMM sticks.

### Multi-GPU, Multi-Storage & Modern HWMonitor Sensor Tree
- **Multi-GPU:** Adapter selector pills (`GPU 0`, `GPU 1`, ...) + compact side-by-side summary strip.
- **Multi-Drive:** CrystalDiskInfo-style compact drive list with emerald `SMART VERIFIED` badges, capacity bars, and click-to-inspect S.M.A.R.T. drawer.
- **HWMonitor Sensor Tree:** Collapsible hardware groups with columns: `Sensor + Source Key`, `Current (monospace)`, **Visual Min–Current–Max horizontal thermal range bar** (emerald → amber → red with crisp white current-value dot), `Min`, `Max`, and `Pin to HUD`. Clicking any row opens a slide-in live history graph.

---

## 4. Real-World Stress Lab (Not 1–5 Minute Toys — #5)

Real hardware validation requires sustained runs, full-core saturation, meaningful memory coverage, and an unmistakable emergency stop.

### Real-World Run Modes
- **Timed Presets:** `5m (Quick)` · `30m` · `1h` · `4h` · `8h` · `12h` · `24h` · **Custom `HH:MM:SS`**
- **Continuous Mode:** **Run Until Stopped** (elapsed counter + rolling metrics, zero arbitrary 10-minute cutoff)
- **Pass-Count Mode (MemTest / Storage):** Run `1 / 4 / 10 / Infinite` full verification passes

### Full Hardware Workload Scaling
- **CPU (Prime95-style):** Lift the hardcoded 8-worker cap → dynamically support `1..=logical_cores` (presets: `All Logical`, `Physical Only`, custom). Profiles: *Integer Verify*, *Floating-Point / FMA*, *Blend (CPU + Cache + RAM)*. Live per-worker status dot matrix.
- **Memory (MemTest-style):** Safe percentage-based allocation (`25% / 50% / 75%` of available RAM with strict OS reserve guard to avoid OOM/freeze), cycling all 4 deterministic byte patterns (`55/AA`, `Checkerboard`, `Walking Zero`, `Pseudo-Random`) with exact `Address / Expected / Actual` mismatch capture.
- **GPU (FurMark / Compute-style):** Adapter selector, intensity tiers (`Light / Standard / Heavy / Extreme`), optional VRAM verification sweep, and watchdog/hang detection.
- **Storage I/O:** Sustained verified scratch-file passes with explicit SSD-wear notice and user-controlled write cap.
- **Combined Load (OCCT-style):** Concurrent `CPU + RAM` and opt-in `CPU + RAM + GPU` stability verification.

### Unclipped 2-Column Supervision Cockpit
- Top horizontal 5-mode pill bar (`CPU` | `GPU` | `Memory` | `Storage` | `Combined`).
- Left Config Panel (`36%`): Parameters, duration/pass pills, opt-in **Advisory Thermal Auto-Stop (`°C` + hold time)**, and an **in-place primary button** that flips immediately to crimson **`Stop Check (Esc)`** while running.
- Right Live Cockpit (`64%`): Circular progress ring (or elapsed ring in Continuous mode), bold emerald **`✓ 0 ERRORS · VERIFIED`** badge (turns crimson immediately on first error), tabular KPIs (Verified Work, Throughput/s, Peak Temp + Headroom), and a completely unclipped live dual-line load/temp chart with dashed advisory limit line.

---

## 5. Maximum Cross-Platform Hardware Coverage (#6)

| Platform | Planned / Expanded Read-Only Data Sources |
| :--- | :--- |
| **Windows** | Full CIM/WMI + CPUID instruction flags, `Win32_PhysicalMemory` + `Win32_PhysicalMemoryArray` (empty slots), `wgpu` DX12 + NVIDIA `NVML` + **DXGI/PDH** (AMD/Intel GPU load & VRAM), `MSFT_PhysicalDisk` + `MSFT_StorageReliabilityCounter`, ACPI thermal zones, plus opt-in read-only consumer for LibreHardwareMonitor/HWiNFO shared memory if already running. |
| **macOS (Intel & Apple Silicon)** | Native `arm64` Apple Silicon SMC sensor key tables (M1–M5 temps & fans), `hw.perflevel*` P/E clusters, read-only `IOReport` telemetry where permitted without `sudo`, `vm_stat` unified memory pressure, `diskutil` / `SPNVMeDataType`, battery cycles/health/watts, EDID displays, Thunderbolt/USB/Wi-Fi/Bluetooth. |
| **Linux (First-Class Parity)** | Zero-root **`/sys/class/hwmon`** direct reader (`coretemp`, `k10temp`, `zenpower`, `amdgpu`, `nvme`, fans, power, voltages), `/sys/devices/system/cpu/cpu*/cache` & live `scaling_cur_freq`, `/sys/class/dmi/id`, `/sys/bus/pci/devices` (PCIe link speed/width), `/sys/class/power_supply`, `/sys/class/drm/*/edid`. |

---

## 6. Fast, Smooth, Responsive Performance Budget (#7)

- **Instant Launch:** `< 100 ms` to first interactive frame (render immediately from cached `inventory.json` + fast initial snapshot; slow OS probes run async).
- **Featherweight Footprint:** `< 0.5%` idle CPU, `< 50 MiB` RAM (zero Electron/WinUI memory creep).
- **On-Demand Repainting:** Repaint only on telemetry tick, user input, or active ~200ms easing transitions — never spin needlessly at 60fps when idle.
- **Tiered Polling:** Fast tier (CPU/RAM/temps at user cadence), medium tier (disks/net/processes), slow/on-demand tier (inventory/BT/battery); pause when minimized unless logging or stress testing.
- **Zero Jitter & Virtualized Rendering:** Strict tabular monospace numbers; custom `egui::Painter` liquid-fill tiles with zero per-frame heap allocations; `ScrollArea::show_rows` virtualization for 500+ processes, 300+ drivers, and 128-thread views so frame build time stays `< 2 ms`.
- **Code Modularization:** Split monolithic `src/main.rs` cleanly into `src/ui/{theme,shell,monitor,hardware,stress,diagnostics,widgets}.rs`.
