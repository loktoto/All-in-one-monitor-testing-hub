# Hardware Hub

A local Rust desktop app that monitors a computer and runs small, bounded stability checks. It runs on **macOS, Windows and Linux**. No account, cloud service or automatic stress test is involved. The interface is in English; notes below are also provided in 繁體中文.

## Highlights

- **Production defaults** — the app remembers your last page, sampling interval, worker count, alert threshold and export paths in a per-user JSON file, and restores them on the next launch. A corrupt or missing file falls back to safe defaults instead of blocking startup.
- **Quality gate** — `cargo fmt --check`, `cargo clippy -D warnings` and the full test suite run in CI on Linux and Windows (`.github/workflows/ci.yml`). The codebase builds without warnings.
- **Panic-free hot paths** — no `unwrap`/`expect` on user or OS input; failures surface as messages or "Not available" rows.
- **Self-contained packaging** — the window/taskbar icon is generated at startup, the release profile strips symbols, and no external asset files are required.

## Build and test

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo run --release
```

`cargo build --release` produces a single `hardware-hub` binary (`hardware-hub.exe` on Windows). See [Development](#development) for the smoke and performance examples.

## Run on macOS

1. Install the Xcode command line tools (`xcode-select --install`), then the stable Rust toolchain (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).
2. In this folder run `cargo run --release`.
3. **Overview** shows live readings. Hardware pages group CPU, GPU, Memory, Storage, Thermals / fans, Parts & board, Drivers, Network / Wi-Fi and Bluetooth information. CPU, GPU, Memory and Storage each include their relevant bounded check and recent results. **Details**, **Processes**, **Frame analysis**, **Stress tests** and **Reports** provide shared views. **Floating monitor** opens an optional always-on-top desktop window.

Graphics discovery uses Metal. If you install a binary produced by CI instead of building locally, macOS may quarantine it; clear that with `xattr -d com.apple.quarantine hardware-hub`. An unsigned local build may also need `System Settings → Privacy & Security → Open Anyway` the first time.

## Run on Windows

1. Install the current stable Rust toolchain with the MSVC target and the Visual Studio C++ build tools.
2. In this folder run `cargo run --release`. Or use the Windows executable artifact from a successful GitHub Actions run.
3. The same pages are available; GPU discovery uses DX12.

Linux is supported for development where an X11/graphics stack is available (the `x11` feature is enabled only for Linux targets).

## What each page shows

Hardware Hub opens in Compact mode on a fresh setup. **Expand** opens the wider workspace and **Compact** returns to the narrow monitor. The toolbar exposes the Floating HUD and sensor density options. **Ctrl+K** on Windows/Linux or **⌘K** on macOS searches pages, stress checks, temperature sensors and running processes.

**Hardware pages** — CPU, GPU, Memory and Storage collect the available readings and a matching manual check in one place. Motherboard, Network / Wi-Fi and Bluetooth show the data currently exposed on the platform. macOS Bluetooth controller state, transport and chipset, if reported, are read at launch and can be checked again manually; connected-device names are shown only when the OS reports them; Bluetooth-specific stress tests are not available. Network interface counters are available; macOS may identify the Wi-Fi interface from networksetup at startup. An optional TCP connection check targets a user-entered numeric IP and port, makes at most four connections with no application payload and reports successful connect time. It is a reachability diagnostic, not a Wi-Fi throughput or stress test. Link quality is unavailable. macOS may provide a board ID through IORegistry, but motherboard model and dedicated board tests are unavailable. Only one check can run at a time, except the Combined check which intentionally loads CPU and memory together.

**Overview** — CPU and memory activity, a liquid gauge for each logical processor, a sensor table with current/min/max readings, graphics adapters and volume capacity. Each LP tile shows operating-system usage for a logical processor. On Linux systems where `coretemp` and sysfs can identify a physical core, its threads show the same measured core temperature. Windows ACPI thermal zones appear on **Thermals / fans**. Click **Reset min/max** to clear the session extremes.

**Thermals / fans** — the sensor table plus a selectable temperature-history graph (min / max / average, gap-aware line segments, one-sensor CSV export and all-sensors wide CSV export), an advisory temperature alert with warning/critical severity and a configurable hold time, and a sampling-interval slider (250–5000 ms). Alerts never change fan speed, clocks or tests. **GPU** also shows wgpu adapter capabilities (backend, PCI IDs, texture/workgroup/buffer limits and optional feature count). **Reports** can export a CPU-Z/GPU-Z-style system report as text or JSON, with optional redaction of serial numbers, paths and device names. Cmd/Ctrl+1–9 switch the main pages, Cmd/Ctrl+Shift+1–6 reach Parts & board, Drivers, Network, Bluetooth, Frame analysis and Stress tests, and Esc cancels the current check.

**Processes** — filter by process name or PID and sort by CPU or memory; per-interface network rates use received/sent byte deltas divided by the measured refresh interval. CPU percentage is relative to one logical core and can exceed 100%. Process and interface availability depend on OS permissions, and virtual interfaces can appear. No process control or GPU attribution is provided.

**Frame analysis** reads a previously saved PresentMon CSV (up to 32 MiB / 500,000 rows) for exactly one process and one swap chain. It uses `DisplayedTime` or `MsBetweenDisplayChange` where available, skips missing or invalid displayed intervals, and reports average displayed FPS, slowest 1% FPS and the 99th percentile frame time. The slowest 1% is the reciprocal of the mean of the longest 1% of valid intervals, rounded up. Optional mean CPU/GPU busy times use only rows that provide values. PresentMon capture is Windows-specific; Hardware Hub does not run it or capture game performance itself. See [PresentMon CSV definitions](https://github.com/GameTechDev/PresentMon/blob/main/README-ConsoleApplication.md#csv-columns).

The optional floating desktop monitor shows CPU and memory, including stale status. It is not an in-game overlay and does not measure FPS. Visibility above full-screen applications depends on the window manager.

**Record session to CSV** starts an optional local log of CPU, memory, temperature and fan readings with sensor keys. It runs in a background writer, reports dropped batches and errors, stops at 1,000,000 rows, and requires a new file. Times are when the app recorded the batch, not sensor-provided timestamps. Stop recording and let it finish before closing; a crash or forced termination may leave a partial file.

The chart keeps at most 60 distinct, recent collection batches; missing or delayed readings break the line. **Export recent history** writes this window as CSV with elapsed time since app launch. Blank fields mean missing values; an existing file is never overwritten. It does not record a persistent full-session log.

**Details** — the specification sheet, collected once at startup from local, read-only sources:

| Area | macOS source | Windows / Linux source |
| --- | --- | --- |
| System (model, OS, kernel, firmware, uptime, battery, board ID) | `sysctl`, `system_profiler SPHardwareDataType`, `pmset` | Windows: CIM (`Win32_ComputerSystem`, `Win32_BIOS`, `Win32_BaseBoard`, `Win32_Battery`). Linux: `sysinfo` and readable DMI and power-supply sysfs files |
| Processor (vendor, architecture, socket, stepping, physical/logical cores, P/E cores, clocks, L1/L2/L3, cache line, SMT) | `sysctl` (`hw.*`, `machdep.cpu.*`) | Windows: `sysinfo` + `Win32_Processor` (socket, stepping, rated clock, L2/L3; no L1). Linux: `sysinfo`, `/proc/cpuinfo` and readable sysfs cache and frequency files |
| Memory (modules with slot, size, type, rated and configured speed, form factor, manufacturer, part number, serial, ECC, upgradability) | `system_profiler SPMemoryDataType` | Windows: `Win32_PhysicalMemory`. Linux: total only |
| Graphics (adapter, backend, driver and driver date, vendor/device ID, reported VRAM, API, current display mode) | `wgpu` + `system_profiler SPDisplaysDataType` | `wgpu`; Windows also reads `Win32_VideoController` for the OS driver and display mode |
| Storage health (protocol, solid state, S.M.A.R.T. state per volume, drive model, firmware, serial, link, TRIM) | `diskutil info`, `system_profiler SPNVMeDataType` | Windows: the `MSFT_PhysicalDisk` CIM class, `Win32_DiskDrive` and `Get-Volume` report model, firmware, bus, solid state, OS health and volume state; drive temperature and wear counters are refused to a standard user and are not read. Linux: unavailable |

Anything the platform does not expose is listed under **Not available** rather than estimated.

**Parts & board** extends the hardware view with CPU-Z-style detail: processor family/description, instruction sets, firmware virtualization, processor ID, per-slot memory modules (configured/voltage range and bus widths, ECC derived from width), GPU driver version/date, video BIOS and reported VRAM/PCI address/current display mode, plus attached displays decoded from each monitor's EDID identity block (manufacturer, model, serial, manufacture year/week). Chipset/VRM sensors and GPU clocks still need a vendor driver and are not read.

**Drivers** lists the installed-driver records from the OS driver store with class, version, provider and signature status. It can be filtered by name/provider and by class. It is a read-only inventory of what the OS reports, not a driver update tool and not a verification that a driver is correct or current.

**Import S.M.A.R.T. report** in Storage or Details reads a user-selected local `smartctl -a -j` JSON report (maximum 4 MiB). It displays reported ATA attributes, selected NVMe fields, the smartctl exit status and messages. Hardware Hub does not run smartctl, install it or access a drive for this view. The report may be stale or for a different device; its source is not verified. A nonzero smartctl exit status can indicate errors or health warnings. See the [smartctl manual](https://github.com/smartmontools/smartmontools/blob/main/src/smartctl.8.in) for device names, permissions and exit status meanings.

## Measurements and limitations

- `sysinfo` supplies CPU model and aggregate/per-logical-core load, memory, and mounted volume capacity, type and I/O counters. Volume identity is not necessarily a physical drive; a physical disk may have multiple volumes. The OS can report unknown disk type, unavailable counters or omit devices; zero counters are not proof of zero activity. Values reflect the operating system, not a hardware diagnosis.
- `wgpu` enumerates graphics adapters on one platform-native backend (Metal on macOS, DX12 on Windows, Vulkan on Linux). NVIDIA's optional NVML driver library supplies GPU load, temperature, VRAM, graphics clock, power, intended fan-0 speed and driver version on Windows and Linux when available; intended fan speed is not proof that the fan is turning; macOS does not load NVML. Where available, it separately shows raw, driver-reported IORegistry utilization keys without claiming a verified mapping to a wgpu adapter. Keys and meanings vary by hardware and may be absent. A unique NVML name match is only a heuristic, not a verified physical identity; ambiguous identical devices are kept separate and labelled.
- On macOS the sensor table reads the SMC directly (the same source HWMonitor-style tools use), which exposes the per-subsystem keys: CPU proximity/die, GPU, battery, memory, heatsink and so on. The key shown on hover is the raw SMC key. `sysinfo` reports only one comparison temperature, so it is used only when the SMC provider is unavailable. On Windows the table reads the firmware's ACPI thermal zones: the `Thermal Zone Information` performance counters (instances such as `\_TZ.TZ00`, readable without administrator rights) with the elevated-only WMI class `MSAcpi_ThermalZoneTemperature` as a fallback and alternative source. Those zones are coarse firmware values, not per-core die temperature, and Windows exposes no fan speed. On Linux the table reads temperature and RPM from readable `hwmon` channels, with `sysinfo` as a fallback. Any gap is listed under **Not available**.
- On macOS, fan speeds require readable `F0Ac`-style keys. A fan count can be available without measured RPM. Automatic stop is an opt-in setting for the running software check and depends on the temperature readings this app receives.
- Min/max values are extremes observed while the app is running. They are not vendor specifications, not lifetime peaks, and they reset when the app restarts. The displayed collection age is when the batch was collected, not an individual sensor timestamp. The macOS collector shows only the drive-reported overall health state. Optional imported smartctl JSON can display reported ATA attributes and selected NVMe health fields; it is a saved report, not a live or verified device association.
- The specification sheet is read once at startup by the local OS tools listed above, using read-only commands. It does not require administrator rights, and it never sends anything over the network. Windows uses one `powershell -NoProfile` pass over CIM/WMI, which can take ten seconds or more; the result is cached in the per-user config directory (`inventory.json`, next to `settings.json`) so the next launch shows it immediately while a fresh collection runs in the background. The page marks cached values with a **CACHED** pill and states when they were collected. Delete `inventory.json` to force a cold read; nothing in it leaves the machine.
- `diskutil`/`system_profiler` S.M.A.R.T. output is the summary the drive firmware exposes. It is not a full attribute table and cannot be obtained for every USB/RAID enclosure; unsupported drives show no health value instead of a guessed one.
- Disk throughput counters are bytes since the last refresh (approximately one second), not a guaranteed precise per-second rate. They are OS-provided and can be unavailable or zero even when activity occurs.

## Manual tests

**Stress Lab** offers timed checks from 1 second to 24 hours, continuous runs and pass-count runs. CPU supports integer, floating-point/FMA and blend workloads with up to the logical processors reported by the OS, capped at 256 workers. Memory supports four patterns or cycles through all four, with a 16 GiB allocation ceiling and an available-memory reserve. GPU offers four compute intensities on the selected hardware adapter. **Combined** verifies CPU and memory concurrently. The cockpit shows elapsed time, verified work, throughput, mismatches and observed temperatures. The **Start stability check** control stays visible while scrolling and becomes **Stop Check** during a run.

**Thermal auto-stop** is off by default. When enabled, it requests cancellation after consecutive fresh readings meet the configured threshold for the hold duration. These readings can be missing, coarse or delayed. Auto-stop applies only to the app's workload and does not change hardware settings. Closing the window during a check requests a stop and allows the worker to finish.

File checks write and verify an owned scratch file of up to 1 GiB within an explicit per-run cap, up to 64 GiB. Pass-count runs need a write cap large enough for all requested passes. Writes use free space and cause SSD wear. Normal completion, cancellation and errors clean up the scratch file. Power loss or forced termination can leave it behind. A blocked disk or GPU driver can delay stopping. Incomplete GPU dispatches are excluded from verified counts. Reports keep the first eight mismatch details and the detected mismatch count.

Reports contain test type, status, verified work when known, settings, system context, elapsed time, a UTC timestamp, and any recorded mismatch details. **Save run summary** writes a one-page, shareable text certificate of a single result with no paths or hardware identifiers; it states plainly that the run was a bounded correctness check, not a benchmark score or certification. With two or more results in a session, **Compare runs** places any two side by side with their status, verified units, error count, elapsed time and settings, in the style of a pro report-comparison tool; differing settings or durations make raw unit counts non-comparable and no score is implied. Expand **Evidence and limits** for the settings (including the selected pattern/intensity), system context at the start and scope of each check. They stay in memory until exported as text or JSON; a new path is required and existing files are never overwritten. Reports include the selected directory and hardware/OS details, so review before sharing.

## Development

```sh
cargo fmt --check
cargo test
cargo run --example smoke  # bounded local CPU / 1 MiB RAM / 1 MiB file checks
cargo run --example smoke -- --gpu  # also verify a short workload on a discovered hardware GPU
cargo run --release --example perf  # micro-profile of the UI read path
cargo run
```

CI builds and tests on Windows and macOS and publishes each release executable as a workflow artifact. See the [current capability inventory](docs/capabilities.md), [architecture and roadmap](docs/architecture.md) and the [public reference study](docs/reference-study.md) for evidence, feature gaps and boundaries. Test on actual hardware with representative NVIDIA/AMD/Intel/Apple adapters and storage devices before relying on it. No real hardware validation is implied by a CI build.

## 繁體中文

這是以 Windows 與 macOS 為主要平台的本機監控與手動測試工具。開啟後不會自動執行壓力測試。**Overview** 顯示即時讀值；**Details** 顯示開機時讀取一次的規格表（CPU 廠牌、核心數、快取、記憶體模組、顯示卡驅動、磁碟 S.M.A.R.T. 狀態、電池）；**Stress tests** 提供有上限的手動檢查；**Reports** 匯出本次結果。

macOS 的顯示卡使用 Metal；磁碟健康、記憶體模組與電池資訊來自 `diskutil`、`system_profiler`、`sysctl` 與 `pmset`，全部為唯讀指令且不連網。平台未提供的項目會列在 **Not available**，不會以估計值取代。

感測器表直接讀取 AppleSMC（同 HWMonitor 類工具的做法），因此可取得 CPU、GPU、電池、記憶體、散熱器等 16 個左右溫度感測器，每個都顯示**目前 / 最低 / 最高**，並可隨時按 **Reset min/max** 清除。**Thermals / fans** 頁面另有可切換的溫度歷史圖、單一感測器 CSV 匯出、持續高溫警告（只提示，不會自動停機或改風速），以及 250–5000 ms 取樣間隔。**Reports** 可滙出 CPU-Z／GPU-Z 風格的系統報告（文字或 JSON，可遮蔽序號與路徑）。最低／最高只是程式執行期間觀察到的極值，不是規格值，也不會跨次啟動保留。風扇只在韌體提供 `F0Ac` 類 key 時顯示；本機只提供風扇數量而無單一風扇 key，因此不會顯示 0 RPM。電壓與瓦數雖然 SMC 有 key，但各機型定義不同，故不顯示而不亂填。NVIDIA NVML 僅喺 Windows／Linux 載入。Stress Lab 可手動開啟高溫停止，連續新讀數達到設定嘅溫度同維持時間後會停止測試。

測試有時間與資源上限。磁碟測試只建立獨立暫存檔並驗證讀回資料，可設定總寫入上限（預設 64 MiB，最多 1 GiB），但仍會造成 SSD 耗損；異常斷電可能留下暫存檔。停止可能需等待目前的磁碟或 GPU 操作完成。結果可從 Reports 匯出，未匯出的結果會在程式結束時消失。

## 參考產品優點研究（繁體中文）

以公開官方資料整理的廣泛代表性研究，並非聲稱已涵蓋所有產品或功能。內容包括證據與待核實項目、44 項去重能力的研究時 MVP 對照（後續更新請看 current capability inventory），以及按安全前提排序的路線圖／介面規格。這些是研究與建議，並不代表功能已實作。

- [產品與來源：監控、測試、基準、儲存及診斷](docs/research/01-product-evidence.zh-Hant.md)
- [去重能力目錄與現有版本落差](docs/research/02-capability-catalog.zh-Hant.md)
- [優先路線圖、可行性及介面規格](docs/research/03-roadmap-and-ux.zh-Hant.md)
- [監控軟件優點彙整：HWiNFO／HWMonitor／CAM／Afterburner／PresentMon／CPU-Z／GPU-Z／CrystalDiskInfo／工作管理員／PowerToys](docs/research/04-monitor-app-advantages.zh-Hant.md)
- [2026 mainstream monitor study and implemented gap analysis](docs/research/05-mainstream-2026-gap-analysis.md)
