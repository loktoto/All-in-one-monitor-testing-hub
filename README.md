# Hardware Hub

A local Rust desktop app that monitors a computer and runs small, bounded stability checks. It runs on **macOS, Windows and Linux**. No account, cloud service or automatic stress test is involved. The interface is in English; notes below are also provided in 繁體中文.

## Run on macOS

1. Install the Xcode command line tools (`xcode-select --install`), then the stable Rust toolchain (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`).
2. In this folder run `cargo run --release`.
3. **Overview** shows live readings. Hardware pages group CPU, GPU, Memory, Storage, Thermals / fans, Motherboard, Network / Wi-Fi and Bluetooth information. CPU, GPU, Memory and Storage each include their relevant bounded check and recent results. **Details**, **Processes**, **Frame analysis**, **Stress tests** and **Reports** provide shared views. **Floating monitor** opens an optional always-on-top desktop window.

Graphics discovery uses Metal. If you install a binary produced by CI instead of building locally, macOS may quarantine it; clear that with `xattr -d com.apple.quarantine hardware-hub`. An unsigned local build may also need `System Settings → Privacy & Security → Open Anyway` the first time.

## Run on Windows

1. Install the current stable Rust toolchain with the MSVC target and the Visual Studio C++ build tools.
2. In this folder run `cargo run --release`. Or use the Windows executable artifact from a successful GitHub Actions run.
3. The same pages are available; GPU discovery uses DX12.

Linux is supported for development where an X11/graphics stack is available (the `x11` feature is enabled only for Linux targets).

## What each page shows

**Hardware pages** — CPU, GPU, Memory and Storage collect the available readings and a matching manual check in one place. Motherboard, Network / Wi-Fi and Bluetooth show the data currently exposed on the platform. macOS Bluetooth controller state, transport and chipset, if reported, are read at launch and can be checked again manually; connected-device names are shown only when the OS reports them; Bluetooth-specific stress tests are not available. Network interface counters are available; macOS may identify the Wi-Fi interface from networksetup at startup. An optional TCP connection check targets a user-entered numeric IP and port, makes at most four connections with no application payload and reports successful connect time. It is a reachability diagnostic, not a Wi-Fi throughput or stress test. Link quality is unavailable. macOS may provide a board ID through IORegistry, but motherboard model and dedicated board tests are unavailable. Only one check can run at a time.

**Overview** — CPU model and load with a recent-activity chart, memory used/available, per-core load, a HWMonitor-style sensor table (**current / min / max** for every temperature sensor and fan), graphics adapters, and volume capacity with S.M.A.R.T. state and I/O counters where available. Min/max are the observed extremes since launch and can be cleared with **Reset min/max**.

**Processes** — filter by process name or PID and sort by CPU or memory; per-interface network rates use received/sent byte deltas divided by the measured refresh interval. CPU percentage is relative to one logical core and can exceed 100%. Process and interface availability depend on OS permissions, and virtual interfaces can appear. No process control or GPU attribution is provided.

**Frame analysis** reads a previously saved PresentMon CSV (up to 32 MiB / 500,000 rows) for exactly one process and one swap chain. It uses `DisplayedTime` or `MsBetweenDisplayChange` where available, skips missing or invalid displayed intervals, and reports average displayed FPS, slowest 1% FPS and the 99th percentile frame time. The slowest 1% is the reciprocal of the mean of the longest 1% of valid intervals, rounded up. Optional mean CPU/GPU busy times use only rows that provide values. PresentMon capture is Windows-specific; Hardware Hub does not run it or capture game performance itself. See [PresentMon CSV definitions](https://github.com/GameTechDev/PresentMon/blob/main/README-ConsoleApplication.md#csv-columns).

The optional floating desktop monitor shows CPU and memory, including stale status. It is not an in-game overlay and does not measure FPS. Visibility above full-screen applications depends on the window manager.

**Record session to CSV** starts an optional local log of CPU, memory, temperature and fan readings with sensor keys. It runs in a background writer, reports dropped batches and errors, stops at 1,000,000 rows, and requires a new file. Times are when the app recorded the batch, not sensor-provided timestamps. Stop recording and let it finish before closing; a crash or forced termination may leave a partial file.

The chart keeps at most 60 distinct, recent collection batches; missing or delayed readings break the line. **Export recent history** writes this window as CSV with elapsed time since app launch. Blank fields mean missing values; an existing file is never overwritten. It does not record a persistent full-session log.

**Details** — the specification sheet, collected once at startup from local, read-only sources:

| Area | macOS source | Windows / Linux source |
| --- | --- | --- |
| System (model, OS, kernel, firmware, uptime, battery, board ID) | `sysctl`, `system_profiler SPHardwareDataType`, `pmset` | Windows: CIM (`Win32_ComputerSystem`, `Win32_BIOS`, `Win32_BaseBoard`, `Win32_Battery`). Linux: `sysinfo`, these fields unavailable |
| Processor (vendor, architecture, physical/logical cores, P/E cores, clocks, L1/L2/L3, cache line, SMT) | `sysctl` (`hw.*`, `machdep.cpu.*`) | Windows: `sysinfo` + `Win32_Processor` (rated clock, L2/L3; no L1). Linux: `sysinfo`, caches unavailable |
| Memory (modules with size, type, speed, manufacturer, part number, ECC, upgradability) | `system_profiler SPMemoryDataType` | Windows: `Win32_PhysicalMemory`. Linux: total only |
| Graphics (adapter, backend, driver, vendor/device ID, reported VRAM and API) | `wgpu` + `system_profiler SPDisplaysDataType` | `wgpu` |
| Storage health (protocol, solid state, S.M.A.R.T. state per volume, drive model, firmware, link, TRIM) | `diskutil info`, `system_profiler SPNVMeDataType` | Windows: `Get-PhysicalDisk`, `Get-StorageReliabilityCounter` and `Get-Volume` report model, firmware, bus, OS health, temperature and wear. Linux: unavailable |

Anything the platform does not expose is listed under **Not available** rather than estimated.

**Import S.M.A.R.T. report** in Storage or Details reads a user-selected local `smartctl -a -j` JSON report (maximum 4 MiB). It displays reported ATA attributes, selected NVMe fields, the smartctl exit status and messages. Hardware Hub does not run smartctl, install it or access a drive for this view. The report may be stale or for a different device; its source is not verified. A nonzero smartctl exit status can indicate errors or health warnings. See the [smartctl manual](https://github.com/smartmontools/smartmontools/blob/main/src/smartctl.8.in) for device names, permissions and exit status meanings.

## Measurements and limitations

- `sysinfo` supplies CPU model and aggregate/per-logical-core load, memory, and mounted volume capacity, type and I/O counters. Volume identity is not necessarily a physical drive; a physical disk may have multiple volumes. The OS can report unknown disk type, unavailable counters or omit devices; zero counters are not proof of zero activity. Values reflect the operating system, not a hardware diagnosis.
- `wgpu` enumerates graphics adapters on one platform-native backend (Metal on macOS, DX12 on Windows, Vulkan on Linux). NVIDIA's optional NVML driver library supplies GPU load, temperature, VRAM, graphics clock, power, intended fan-0 speed and driver version on Windows and Linux when available; intended fan speed is not proof that the fan is turning; macOS does not load NVML. Where available, it separately shows raw, driver-reported IORegistry utilization keys without claiming a verified mapping to a wgpu adapter. Keys and meanings vary by hardware and may be absent. A unique NVML name match is only a heuristic, not a verified physical identity; ambiguous identical devices are kept separate and labelled.
- On macOS the sensor table reads the SMC directly (the same source HWMonitor-style tools use), which exposes the per-subsystem keys: CPU proximity/die, GPU, battery, memory, heatsink and so on. The key shown on hover is the raw SMC key. `sysinfo` reports only one comparison temperature, so it is used only when the SMC provider is unavailable. On Windows the table reads the firmware's ACPI thermal zones: the `Thermal Zone Information` performance counters (instances such as `\_TZ.TZ00`, readable without administrator rights) with the elevated-only WMI class `MSAcpi_ThermalZoneTemperature` as a fallback and alternative source. Those zones are coarse firmware values, not per-core die temperature, and Windows exposes no fan speed. On Linux the table shows whatever `sysinfo` reports. Any gap is listed under **Not available**.
- Fan speed is shown only when the firmware exposes `F0Ac`-style keys; many Macs (including some virtualised or restricted installations) expose a fan count with no per-fan key, and then no fan row appears instead of a fabricated 0 RPM. Voltage and power keys also exist in the SMC but their meaning differs between models, so they are not shown rather than guessed. There is **no thermal automatic stop**. Use a compatible external monitoring tool and stop if temperatures or system behavior are unsafe.
- Min/max values are extremes observed while the app is running. They are not vendor specifications, not lifetime peaks, and they reset when the app restarts. The displayed collection age is when the batch was collected, not an individual sensor timestamp. The macOS collector shows only the drive-reported overall health state. Optional imported smartctl JSON can display reported ATA attributes and selected NVMe health fields; it is a saved report, not a live or verified device association.
- The specification sheet is read once at startup by the local OS tools listed above, using read-only commands. It does not require administrator rights, and it never sends anything over the network. Windows uses one `powershell -NoProfile` pass over CIM/WMI, which may take a few seconds; the page shows a spinner until it lands. Re-open the app to re-read it after a hardware or OS change.
- `diskutil`/`system_profiler` S.M.A.R.T. output is the summary the drive firmware exposes. It is not a full attribute table and cannot be obtained for every USB/RAID enclosure; unsupported drives show no health value instead of a guessed one.
- Disk throughput counters are bytes since the last refresh (approximately one second), not a guaranteed precise per-second rate. They are OS-provided and can be unavailable or zero even when activity occurs.

## Manual tests

Only one test runs at a time; choose **Stop test** to request cancellation. Closing the window while a test runs requests a stop and keeps the window open until it finishes. CPU uses at most eight workers and may not load all cores; memory covers only the allocated region. CPU checks repeated integer results against a fixed known answer; memory writes and verifies alternating byte patterns; file I/O writes, flushes and reads back a newly created unique scratch file in the chosen directory; GPU runs a small integer shader and verifies every result on the CPU; this is a light compute check and not a full GPU stress test. These checks can find some errors, but do not prove hardware is healthy or replace comprehensive diagnostics. The GPU test uses the adapter selected in the UI; software adapters are reported unavailable.

Duration is 1–600 seconds; CPU is limited to 1–8 threads; memory is at most 256 MiB and at most one quarter of currently available memory; the file size is 1–256 MiB and total writes have a user-set cap (64 MiB by default, at most 1 GiB per run). Tests may finish before the duration limit. File writes consume free space and cause SSD wear; use a suitable directory. No raw disk or existing user file is opened for writing. The owned scratch file is removed on normal completion, failure or cancellation; a power loss, crash or forced termination can leave it behind. Stop is checked between disk chunks and while polling GPU completion; a blocked disk operation or GPU driver can still delay stopping. Unfinished GPU work is not counted as verified. GPU initialization also counts toward the time limit. A run with zero full verification passes is reported as inconclusive.

Reports contain test type, status, verified work when known, settings, system context, elapsed time and a UTC timestamp. Expand **Evidence and limits** for the settings, system context at the start and scope of each check. They stay in memory until exported as text or JSON; a new path is required and existing files are never overwritten. Reports include the selected directory and hardware/OS details, so review before sharing.

## Development

```sh
cargo fmt --check
cargo test
cargo run --example smoke  # bounded local CPU / 1 MiB RAM / 1 MiB file checks
cargo run
```

CI builds and tests on Windows and macOS and publishes each release executable as a workflow artifact. See the [current capability inventory](docs/capabilities.md), [architecture and roadmap](docs/architecture.md) and the [public reference study](docs/reference-study.md) for evidence, feature gaps and boundaries. Test on actual hardware with representative NVIDIA/AMD/Intel/Apple adapters and storage devices before relying on it. No real hardware validation is implied by a CI build.

## 繁體中文

這是以 Windows 與 macOS 為主要平台的本機監控與手動測試工具。開啟後不會自動執行壓力測試。**Overview** 顯示即時讀值；**Details** 顯示開機時讀取一次的規格表（CPU 廠牌、核心數、快取、記憶體模組、顯示卡驅動、磁碟 S.M.A.R.T. 狀態、電池）；**Stress tests** 提供有上限的手動檢查；**Reports** 匯出本次結果。

macOS 的顯示卡使用 Metal；磁碟健康、記憶體模組與電池資訊來自 `diskutil`、`system_profiler`、`sysctl` 與 `pmset`，全部為唯讀指令且不連網。平台未提供的項目會列在 **Not available**，不會以估計值取代。

感測器表直接讀取 AppleSMC（同 HWMonitor 類工具的做法），因此可取得 CPU、GPU、電池、記憶體、散熱器等 16 個左右溫度感測器，每個都顯示**目前 / 最低 / 最高**，並可隨時按 **Reset min/max** 清除。最低／最高只是程式執行期間觀察到的極值，不是規格值，也不會跨次啟動保留。風扇只在韌體提供 `F0Ac` 類 key 時顯示；本機只提供風扇數量而無單一風扇 key，因此不會顯示 0 RPM。電壓與瓦數雖然 SMC 有 key，但各機型定義不同，故不顯示而不亂填。NVIDIA NVML 僅在 Windows／Linux 載入，且**沒有過熱自動停止**功能。

測試有時間與資源上限。磁碟測試只建立獨立暫存檔並驗證讀回資料，可設定總寫入上限（預設 64 MiB，最多 1 GiB），但仍會造成 SSD 耗損；異常斷電可能留下暫存檔。停止可能需等待目前的磁碟或 GPU 操作完成。結果可從 Reports 匯出，未匯出的結果會在程式結束時消失。

## 參考產品優點研究（繁體中文）

以公開官方資料整理的廣泛代表性研究，並非聲稱已涵蓋所有產品或功能。內容包括證據與待核實項目、44 項去重能力的研究時 MVP 對照（後續更新請看 current capability inventory），以及按安全前提排序的路線圖／介面規格。這些是研究與建議，並不代表功能已實作。

- [產品與來源：監控、測試、基準、儲存及診斷](docs/research/01-product-evidence.zh-Hant.md)
- [去重能力目錄與現有版本落差](docs/research/02-capability-catalog.zh-Hant.md)
- [優先路線圖、可行性及介面規格](docs/research/03-roadmap-and-ux.zh-Hant.md)
- [監控軟件優點彙整：HWiNFO／HWMonitor／CAM／Afterburner／PresentMon／CPU-Z／GPU-Z／CrystalDiskInfo／工作管理員／PowerToys](docs/research/04-monitor-app-advantages.zh-Hant.md)
