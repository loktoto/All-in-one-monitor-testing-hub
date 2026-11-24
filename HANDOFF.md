# Handoff — Hardware Hub (macOS port, UI/UX rework, CPU-Z/HWMonitor-class inventory)

Last updated: 2026-11-17. Everything below was verified on the machine described in
[Environment](#environment). Nothing has been committed or pushed: all work is in the
working tree on branch `feature/hardware-monitor-mvp`.

## 1. 摘要（繁體中文）

這個 repo 原本係一個 Windows-first 的 Rust/egui 桌面硬體監控 + 手動測試工具。今次工作把
它變成 **macOS 亦可原生運行**，重寫了整個介面，並把主流監控工具（CPU-Z、HWMonitor、
CrystalDiskInfo、GPU-Z）最有價值的資訊搬進來：

- **macOS 移植**：GPU 改用 Metal 列舉（原本 hardcode Vulkan，在 macOS 一定空）、NVML 只在
  Windows/Linux 載入、X11 feature 只給 Linux、APFS 合成磁碟區過濾。
- **UI/UX 重寫**：設計 token 化的深色主題、側欄導覽、`Overview / Details / Stress tests /
  Reports` 四頁、LIVE/STALE 狀態、Metric 卡片、感測器表、狀態列、彩色狀態標籤。
- **Details 頁（規格表）**：CPU 廠牌/架構/實體與邏輯核心/Apple P・E 核心/時脈/L1-L3/快取行、
  記憶體模組（容量/類型/速度/廠牌/型號/ECC）、GPU 驅動與 vendor/device ID、VRAM、
  每個磁碟區的 S.M.A.R.T. 狀態、NVMe 型號/韌體/link/TRIM、機型/OS/Kernel/韌體/開機時間/電池。
- **HWMonitor 式感測器表**：直接讀 AppleSMC，實測本機 **16 個溫度感測器**，每個都有
  **Current / Min / Max**，可 `Reset min/max`；風扇有 key 才顯示；CPU/RAM 亦有 min/max。

未完成／未驗證的部分在 [§8](#8-known-limitations-honest-gaps) 與 [§9](#9-next-steps-prioritised) 清楚列出。

## 2. Requirements and how each was met

| # | Requirement (as asked) | Delivered | Where |
| --- | --- | --- | --- |
| R1 | "help me migrate it to mac too" | Builds and runs natively on macOS; Metal backend; NVML gated; X11 only on Linux; macOS CI (Intel + Apple silicon); README macOS instructions | `Cargo.toml`, `src/telemetry.rs`, `.github/workflows/macos.yml`, `README.md` |
| R2 | "enhance the ui/ux experience" | Full interface rework: design tokens, sidebar, four pages, card layout, status pills, sensor table, progress panels, coloured result chips, UTC timestamps, keyboard-free but consistent navigation | `src/main.rs` (rewritten, 2107 lines) |
| R3 | "gather all advantage in mainstream monitor apps, cpuz, hwmonitor, crystaldiskinfo, gpuz, etc" | Research-driven feature matrix + `specs` module reading identity/caches/modules/driver/drive health; sensor provider; documented gaps | `src/specs.rs`, `src/sensors.rs`, `docs/capabilities.md`, `README.md` |
| R4 | "you can use rae and craft anything repo to help you with the work" | Used their published *study → build → check* and *evidence + limitations* discipline as the working method. Neither tool was executed, no proprietary binary inspected; lineage is recorded in `docs/reference-study.md` | `docs/reference-study.md` |
| R5 | "get more information as possible like hwmonitor, like min temp, max temp, current temp" | Direct AppleSMC provider: 16 temperature sensors on this Mac, current/min/max per sensor, fan rows when firmware exposes them, session min/max for CPU and memory, reset button, plausibility filtering | `src/sensors.rs`, `src/telemetry.rs`, `src/main.rs` |
| R6 | "return in cantonese" | Replies delivered in Cantonese; repo docs stay English with a 繁體中文 summary section | this file, `README.md` |
| R7 | "hand off everything in details including my requirement" | This document | `HANDOFF.md` |

## 3. Environment

- Host: MacBook Pro `MacBookPro15,2`, Intel Core i7-8565U (4 physical / 8 logical), 16 GB DDR4-2400
  (2 × 8 GB Micron), WDC PC SN520 512 GB NVMe, Intel UHD Graphics 620, battery reporting via `pmset`.
- OS: macOS 15.8.3 (kernel Darwin 24.6.0).
- Toolchain installed during this work: `rustc 1.100.0`, `cargo 1.100.0` via rustup (minimal profile).
  No other system packages were installed.
- Repo started empty in the workspace; cloned from `https://github.com/loktoto/All-in-one-monitor-testing-hub`.
  `main` only had "Initialize repository"; all real code lives on `feature/hardware-monitor-mvp`,
  which is the checked-out branch.

## 4. Repository map (current working tree)

```
Cargo.toml                     deps; eframe features split per target (x11 → Linux only)
src/lib.rs                     modules: report, sensors, specs, stress, telemetry
src/main.rs                    egui app: theme, 4 pages, widgets, formatters
src/telemetry.rs               live sampling thread: CPU/RAM/disks/GPU/sensors + min/max ranges
src/sensors.rs                 AppleSMC provider (IOKit FFI) + label mapping + decoders
src/specs.rs                   one-shot system inventory (sysctl/system_profiler/diskutil/pmset)
src/stress.rs                  bounded manual checks (CPU/GPU/memory/file I/O) + result states
src/report.rs                  text/JSON export with create_new (never overwrites)
examples/smoke.rs              headless end-to-end run
docs/architecture.md           architecture notes (updated: specs + backend selection)
docs/capabilities.md           capability inventory (updated: Details + Sensors rows)
docs/reference-study.md        public-documentation study (unchanged method reference)
docs/research/*.zh-Hant.md     earlier product research (unchanged)
.github/workflows/windows.yml  existing Windows CI (unchanged)
.github/workflows/macos.yml    NEW: macOS CI for macos-13 (Intel) and macos-14 (Apple silicon)
README.md                      rewritten: macOS + Windows instructions, per-platform source table
HANDOFF.md                     NEW: this document
```

Sizes for orientation: `main.rs` 2107, `stress.rs` 774, `specs.rs` 716, `telemetry.rs` 515,
`sensors.rs` 433, `examples/smoke.rs` 141, `report.rs` 91.

Diff against `d018f9c`: 8 files changed, +2206/−281, plus three new untracked files
(`sensors.rs`, `specs.rs`, `macos.yml`).

## 5. How the macOS port works

1. **Graphics backend** — `preferred_backends()` returns `METAL` on macOS, `DX12` on Windows,
   `VULKAN` elsewhere. Before this change macOS asked for Vulkan, which always enumerated zero
   adapters, so the GPU section and the GPU check were permanently empty.
2. **NVML** — `init_nvml()` is compiled to `None` outside Windows/Linux. `nvml-wrapper` still
   compiles on macOS but there is no NVIDIA driver library to load.
3. **X11** — `eframe`'s `x11` feature moved into a `[target.'cfg(target_os = "linux")'.dependencies]`
   block. macOS uses CGL, Windows uses its native path; nothing links X11 there.
4. **Volumes** — `/System/Volumes/*` synthetic APFS mounts are filtered from the live list.
5. **Drive health / identity** — `diskutil info <mount>` per visible mount gives protocol,
   S.M.A.R.T. state and solid-state flag; `system_profiler SPNVMeDataType` gives model, firmware,
   link width/speed, TRIM and drive capacity.
6. **Inventory** — `sysctl` (`hw.*`, `machdep.cpu.*`, `kern.*`), `system_profiler` memory/graphics/
   NVMe, `pmset -g batt`. All read-only, no admin rights, no network. Runs once on a background
   thread at startup so the first frame is not blocked; the Details page shows a spinner meanwhile.
7. **Sensors** — `sysinfo` exposes exactly one component on this Mac, so `src/sensors.rs` opens
   `AppleSMC` through IOKit (`IOServiceMatching` → `IOServiceOpen` → `IOConnectCallStructMethod`),
   reads `#KEY`, enumerates every key once, and keeps the `T*` (`sp78`/`flt`) and `F<n>Ac`
   (`fpe2`) ones. Each poll reads only those keys. Guards:
   - `#KEY` count must be 50–5000, otherwise the provider is disabled and `sysinfo` is used.
   - `SMCKeyData_t` is 80 bytes and a unit test asserts it (`AppleSMC expects an 80-byte key struct`).
   - Per-reading plausibility: temperatures −20…150 °C, fans 0…20000 RPM; outliers are dropped.
   - Enumeration capped at 2000 keys; no admin rights, no kernel extension, no new crates.

## 6. UI / UX details

- **Design tokens** — one palette in `main.rs` (`BG`, `PANEL`, `CARD`, `BORDER`, `TEXT`, `MUTED`,
  `ACCENT`, `GREEN`, `AMBER`, `RED`, `VIOLET`); `install_theme` sets spacing, rounding, widget
  strokes and dark visuals once.
- **Navigation** — 224 px sidebar, numbered pills (`01`–`04`), accent bar on the active item,
  hover state, footer showing `wgpu · <backend> · N adapter(s)`.
- **Header** — page title + purpose line, right-aligned `LIVE` / `STALE` / `WAITING` pill driven by
  snapshot age (>3 s ⇒ stale).
- **Overview** — three metric cards (CPU with load bar, memory with used/total + bar, graphics count),
  dual-line history chart (CPU + memory, legend, 0–100 clamp, drag/zoom disabled), per-core meters in
  a responsive grid, **HWMonitor-style sensor table**, graphics adapters, storage rows with
  `SMART VERIFIED` / `SSD` chips and usage bars, and an explicit disclaimer card.
- **Details** — six cards: System, Processor, Memory (per module), Graphics (wgpu adapter + OS
  profile), Storage health, plus a "Not available" card and a sources footer.
- **Stress tests** — 2×2 selectable check tiles, contextual controls per kind, prominent Start/Stop,
  live run panel with elapsed, progress bar, live CPU/memory and last result.
- **Reports** — export card (Text/JSON, path, never overwrites) and one card per result with a
  colour-coded status chip, `YYYY-MM-DD HH:MM:SS UTC` timestamp and configuration summary.
- **Status bar** — single place for app messages, with a colour cue for failures and the host OS name
  on the right.
- **Formatting helpers** — `bytes()` (binary), `decimal_bytes()` (vendor-style GB), `gib()`,
  `clock_text()`, `uptime_text()`, `timestamp_text()` + `civil_from_days()`, `truncate()`,
  `smart_color()`, `load_color()`, `temperature_color()`.

## 7. Data honesty rules (carried through all changes)

- A missing value is shown as `—` / "unavailable" / listed under **Not available**; never guessed,
  never 0 by default.
- Every sensor keeps its provider key (hover to see `TC0D`, `TC0P`, …) so an unfamiliar row is
  traceable and never silently renamed.
- Min/max are explicitly labelled as observed extremes since launch, not specifications, and are
  cleared by **Reset min/max** (a flag the telemetry thread consumes).
- Test results keep the existing five-state model (`Completed / Stopped / Failed / Inconclusive /
  Unavailable`); unchanged from before this work.
- Reports still use `create_new` and never overwrite; text/JSON export unchanged.

## 8. Known limitations (honest gaps)

1. **S.M.A.R.T. attribute table** — only the drive's overall health word is available via
   `diskutil`/`system_profiler`. Raw attributes (05/C5/C6, NVMe Percentage Used, Available Spare,
   Power-On Hours) need either `smartctl` or direct NVMe/IOKit ioctls; not implemented.
2. **Fans on this machine** — the SMC exposes `FAC0` (fan count) but no `F0Ac` key, so no fan row
   appears. The code handles `F<n>Ac` and will show RPM where firmware provides it. No 0 RPM is
   fabricated.
3. **Voltage / power draw** — keys such as `VC0C`, `PC0C`, `PCPC` exist (`sp3c`, `sp96`, `flt`) but
   their meaning differs per model and many are limits/percentages rather than watts. Deliberately
   not shown rather than risk wrong numbers.
4. **Windows/Linux depth** — `specs::portable` only fills OS, CPU name/vendor/cores/clock, memory
   total. Caches, memory modules, drive health, battery are reported as unavailable. Closing this
   needs WMI (`Win32_PhysicalMemory`, `MSStorageDriver_FailurePredictStatus`) and Linux
   `/sys`+`smartctl` work, with real hardware to validate on.
5. **GPU depth** — driver strings and vendor/device IDs come from wgpu; VRAM and API come from
   `system_profiler` on macOS only. No GPU temperature/clock/voltage without vendor APIs (NVML is
   NVIDIA-only and absent here).
6. **Per-core real clock** — not exposed by macOS `sysctl`; CPU-Z's "Core Speed" has no equivalent.
7. **Inventory is one-shot** — collected at startup; hot-plugged devices or OS updates need an app
   restart (stated in the UI footer and README).
8. **No Apple Silicon hardware validation** — code paths for `hw.perflevel*` (P/E cores) are written
   and CI builds on `macos-14`, but the only real run was on Intel. SMC key sets differ.
9. **No visual verification by me** — macOS Screen Recording permission is not granted to this
   terminal, so `screencapture -l <window>` returns "could not create image from window". The UI was
   compiled, launched without panics, and its data paths tested headlessly, but the rendered layout
   was reviewed by the user, not by me.
10. **Not committed / not pushed** — see §11.

## 9. Next steps (prioritised)

1. **Windows/Linux parity for the inventory** (biggest gap vs CPU-Z/CrystalDiskInfo): WMI queries for
   memory modules and `MSStorageDriver_FailurePredictStatus`, Linux `smartctl`-optional provider.
2. **S.M.A.R.T. attributes on macOS** via NVMe `IOHIDEventSystem`/`IOKit` SMART read or an optional
   `smartctl` probe, with the same "unknown stays unknown" rule.
3. **Voltage/power section** — opt-in, per-key labelled, clearly marked model-dependent.
4. **Persist min/max** — optional CSV/JSON history so extremes survive restarts; would also give
   sparkline history for temperatures.
5. **Apple Silicon validation** — run on an M-series Mac, check `hw.perflevel*`, SMC labels, Metal
   adapter reporting.
6. **GPU test isolation** — run the GPU check in a child process so a hung driver cannot block
   cancellation (already listed as future work in `docs/architecture.md`).
7. **Commit the work** — see §11 for the recommended commit split.

## 10. Runbook

```sh
# toolchain (already installed on this machine)
source "$HOME/.cargo/env"

# verify
cargo fmt --check
cargo test --locked            # 26 lib + 3 UI tests
cargo run --locked --example smoke   # headless: sensors, ranges, 5 checks, JSON export, cleanup
cargo build --release --locked

# run the GUI
cargo run                      # or: ./target/debug/hardware-hub
```

CI: `.github/workflows/windows.yml` (MSVC build + tests + artifact) and the new
`.github/workflows/macos.yml` (`macos-13` Intel and `macos-14` Apple silicon; fmt, test, release
build, artifact). Neither workflow has been executed on GitHub from this machine.

## 11. Repo state and what to do next

- Branch: `feature/hardware-monitor-mvp`, based on `d018f9c`.
- Modified: `Cargo.toml`, `README.md`, `docs/architecture.md`, `docs/capabilities.md`,
  `examples/smoke.rs`, `src/lib.rs`, `src/main.rs`, `src/telemetry.rs`.
- New: `src/sensors.rs`, `src/specs.rs`, `.github/workflows/macos.yml`, `HANDOFF.md`.
- **No commit and no push were made** — this was not requested. If you want one, a sensible split is:
  1. macOS port (`Cargo.toml`, `telemetry.rs` backend/NVML/volume changes, `macos.yml`, README run section)
  2. UI/UX rework (`main.rs`)
  3. Inventory + sensors (`specs.rs`, `sensors.rs`, `lib.rs`, docs, smoke example)
- `.freebuff/` is tooling state, not part of the project; leave it out of commits.

## 12. Verification log (commands actually run)

| Check | Result |
| --- | --- |
| `cargo check --locked --all-targets` | passes (macOS, x86_64) |
| `cargo fmt --check` | clean |
| `cargo test --locked` | **26 lib + 3 UI tests pass**, 0 failed; only remaining warning is upstream `block v0.1.6` future-incompat notice |
| `cargo run --example smoke` | `16 temperature sensors, 0 fans`; `CPU range Some(30.1)-Some(30.1)%`; CPU/Memory/Disk Completed, GPU Unavailable as designed; JSON export + overwrite refusal + scratch cleanup OK |
| SMC unit test | provider opens, 16 sensors, all values inside the plausibility range; `SMCKeyData_t` asserted 80 bytes |
| Inventory unit test | real collector run: `macOS 15.8.3 · Intel Core i7-8565U · 4 physical / 8 logical · 2 memory modules · 1 drive · 1 volume · battery 100% · charged · AC Power` |
| GUI launch (debug) | window "Hardware Hub" opened, log contains no panic/error lines |
| Screenshot of the GUI | **not possible**: macOS Screen Recording permission missing for this terminal |

## 13. Key decisions and why

- **Read the SMC directly instead of adding a dependency**: `sysinfo` gives one sensor on macOS;
  the request was HWMonitor-like coverage. IOKit FFI keeps the dependency set unchanged (no new
  crates, no `libc`), and the provider degrades to `sysinfo` if validation fails.
- **Keep `glow` (OpenGL) for windowing rather than switching eframe to wgpu**: OpenGL works on macOS
  via CGL and switching renderers was a larger, riskier change than the port required.
- **One-shot inventory on a background thread**: `system_profiler` calls take ~0.5–2 s each; doing
  them per frame or synchronously at startup would stall the UI.
- **No voltage/power/attribute tables**: undocumented or model-dependent keys would produce
  confidently wrong numbers, which contradicts the project's stated measurement contract
  (`docs/research/03-roadmap-and-ux.zh-Hant.md` §測量與安全資料契約).
