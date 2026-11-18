# 2026 mainstream monitor study → implementation gap analysis

Research date: 2026-11-18. Sources are the vendors' own product pages where
available. This is a practical feature comparison for this repository, not a
claim of feature-for-feature equivalence with any commercial product.

## What was researched

| Product | Documented strengths | Official source |
| --- | --- | --- |
| CPU-Z 3.02 | CPU name/codename/process/package/caches, mainboard/chipset/BIOS, memory type/size/timings/SPD including DDR5 and XMP 3.0, per-core clocks, GPU tab with clocks/TDP, sensor voltages, single/multi-core benchmark, Validator V3, text/HTML reports, CLI report/bench flags, F5/F6/F7 screenshot, clipboard and validation-file shortcuts. | <https://www.cpuid.com/softwares/cpu-z.html> |
| HWMonitor / HWMonitor PRO | Voltages, temperatures, fan speeds, power, current, utilisation, clocks; S.M.A.R.T., battery, LPCIO, PSU and AIO sources; per-pin GPU currents, max sensors, CSV logging (1.62), real-time and transparent graph windows (1.64/1.66), themes, sensor tree state. | <https://www.cpuid.com/softwares/hwmonitor.html> |
| GPU-Z | NVIDIA/AMD/Intel adapter, GPU and display information, default/overclock/boost clocks, memory size/type/speed/bus width, PCIe-lane render test, result validation, BIOS backup, no install required. | <https://www.techpowerup.com/gpuz/> |
| HWiNFO | In-depth hardware tree, low-overhead real-time monitoring, extensive reporting/status logging, failure prediction and add-on interfaces; no personal data collection. | <https://www.hwinfo.com/> |

## Comparison with this build, and what changed

| Mainstream capability | This build before | This build now |
| --- | --- | --- |
| CPU/mainboard/memory/drive identity and caches | Specification sheet added on Windows in the uncommitted work: socket, stepping, configured memory speed, form factor, serials, GPU driver/date, display mode, drive serials | Same, plus **exportable CPU-Z-style system report** (text or JSON, optional redaction of serials/paths/device names) from Reports |
| GPU identity, driver and device limits | Adapter name/vendor/device ID and NVML detail string | Added an **Adapter capabilities** section: backend, vendor/device IDs, maximum 2D texture, compute workgroup dimensions, storage-buffer binding and optional feature count |
| Temperature/fan current-min-max table | Present on all platforms | Retained; values are labelled as observed session extremes rather than specifications |
| Real-time multi-sensor graphs | CPU and memory chart only | Added a **selectable temperature history graph** with min/max/average, gap-aware line segments and single-sensor CSV export |
| CSV session logging | Fixed ~1 s cadence | Unchanged bounded writer, plus **user-controlled sampling interval (250–5000 ms)** so cost and density can be balanced |
| Threshold alerts | None | Added **sustained temperature alerts** with warning/critical severity, configurable threshold and hold time, freshness checks, acknowledgement and a status-bar indicator. Advisory only: no fan, clock or test changes |
| Reports, clipboard, validation | Text/JSON test results | System report added; online validation is deliberately not implemented (it would transmit hardware data) |
| Benchmarks (CPU-Z, 3DMark-style) | Bounded correctness checks, explicitly not benchmarks | Still correctness checks. A cross-machine comparable score would need a fixed, calibrated workload and versioned scoring; publishing an uncalibrated ops/sec number would be misleading |
| Overclocking, fan curves, BIOS backup, hardware writes | None | Still none. These need privileged/driver access and have real hardware risk; this app stays read-only except for its owned scratch test file |
| Remote monitoring, alerts by email, account/cloud | None | Still none; local-only by design |
| GPU render/PCIe-lane test | Small verified compute dispatch | Still compute-only; no synthetic graphics load or VRAM stress test |
| Keyboard-driven navigation | None | **Cmd/Ctrl+1…9** page shortcuts and **Esc** to request check cancellation; hint shown in the sidebar |

## Performance profiling and optimisation

Profiled on the Windows development host with a synthetic 500-process snapshot
(`cargo run --release --example perf`; hardware untouched, no disk writes):

| Read path | Before | After |
| --- | --- | --- |
| Snapshot read per rendered frame | 33,294 ns deep clone of processes, disks, names and adapters | 15 ns atomic `Arc` clone |
| Specification sheet read per frame | Full `SystemSpecs` deep clone | `Arc` clone |
| Temperature graph split for 60 samples | n/a (no graph) | 1,895 ns/frame |
| Telemetry cadence | Fixed ~1 s, fixed sleeps stacked onto collection time | Configurable 250–5000 ms; worker sleeps only for the remainder of the period after collection finishes |

The release smoke run (`cargo run --release --example smoke`) passed: telemetry
with 8 logical CPUs and 221 processes, a completed CPU check (3,423 verified
rounds), memory and file checks, and an honestly reported unavailable GPU path.
A `cargo test --locked` run passed 48 library and 4 binary tests. The release
binary was launched on the Thermal page without a runtime failure.

## UI/UX direction

The existing Apple-HIG-inspired tokens (neutral graphite surfaces, SF-style
system fonts, translucent-looking cards, restrained accent colours, generous
spacing) were kept. New controls follow the same language: grouped sections,
short explanatory captions, selectable severity chips, a graph with a compact
range summary, and discoverable keyboard hints. Alerts and sampling were placed
on the Thermal page where their meaning is obvious; exports live on Reports.

## Remaining, deliberate boundaries

CPU-Z-style voltage readings depend on model-specific SMC keys and are not
guessable; motherboard chipset and SPD timing profiles are not exposed by the
read-only providers used here. AMD/Intel GPU sensors need vendor libraries or
kernel drivers. Windows cannot expose per-fan RPM or per-core die temperature
through the standard-user providers used. Online validation, overclocking,
remote monitoring and scheduled stress suites are outside the local, read-only
scope. These gaps are listed in the UI under **Not available** rather than
filled with estimates.
