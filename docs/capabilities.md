# Current capabilities and verification boundaries

This is an inventory of the current implementation, not a claim to implement every capability in the [44-item research catalog](research/02-capability-catalog.zh-Hant.md).

| Area | Available in this build | Limits / unavailable |
| --- | --- | --- |
| CPU | OS-reported model, overall and per-logical-core usage; recent history and range; manual integer check against a fixed known answer. | Up to eight worker threads; one small computation cannot certify all instructions or cores. No independent physical-core mapping. |
| Memory | OS-reported total, used and available; history; allocated-memory alternating-pattern readback. | At most 256 MiB and one quarter of available memory; no full-RAM, ECC, SPD, or boot-time test. |
| GPU | One preferred wgpu backend is enumerated; select its retained adapter for a small verified compute check. Optional NVML provides NVIDIA load, temperature and VRAM if supported. | Software adapters are unavailable for testing; no full GPU or VRAM stress test. AMD/Intel load, VRAM and temperature are not supplied by this build. A unique name match with NVML is heuristic; ambiguous matches stay separately labelled. Hardware hot-plug needs restart. |
| Temperatures | `sysinfo` component readings when the operating system exposes them. | Labels/device association may be ambiguous; no per-sensor timestamp or automatic shutdown. Collection age only shows when the batch finished, not when hardware sampled. No fan or power sensors. |
| Storage | Mounted-volume capacity, kind and OS counters when exposed; bounded owned-file write/readback verification, configurable 1–1024 MiB total write cap, default 64 MiB. | A volume is not necessarily a physical drive. No SMART, physical-drive mapping, raw disk test, or reproducible storage benchmark. Readback can come from cache; writes cause wear. |
| Workflow | One manual test at a time, start/stop, progress, typed result status, and session reports in text or JSON with settings and system context. | No automatic thermal stop, persistent session history, scheduling or combined load. Disk/driver operations can delay cancellation. Reports may contain paths and hardware details. |

GPU selection and actual NVML/temperature support depend on the host. The current Linux cloud validation has no physical GPU and no reported system temperature sensors; it cannot establish Windows hardware support. The cross-target check and Windows CI build are compilation/test evidence, not real Windows hardware execution.

## Linux cloud functional checks

The following checks were performed against the Linux cloud environment with a software Xorg display. They demonstrate the listed flows on that environment, not all hardware combinations.

| Flow | Observed result |
| --- | --- |
| Overview | Real CPU, memory and volume data appeared; one llvmpipe adapter was shown; the unavailable system-temperature state was explicit. History reset cleared the chart. |
| CPU | Started, showed progress, stopped, then restarted and completed after a one-second run. The headless smoke run also stopped and restarted it. |
| Memory | A 64 MiB GUI run completed; the headless smoke used 1 MiB. Both verified completed pattern passes. |
| File I/O | A GUI run wrote and verified 1 MiB under a 1 MiB total cap. Its owned scratch file was removed and a pre-existing sentinel remained unchanged; an invalid directory produced Failed. |
| GPU | The GUI selected the software adapter and reported Unavailable. No physical GPU compute, NVML device, or multi-GPU machine was available. |
| Reports | GUI exported text and JSON; a second export to the same JSON path was rejected and its content hash remained unchanged. Automated tests also verify relevant fields and no-overwrite behavior. |

Run `cargo test --locked` for the unit suite, or `cargo run --locked --example smoke` for a bounded headless run. The smoke run allocates 1 MiB for memory, writes at most 1 MiB to its own temporary directory, performs short CPU checks, and verifies the unavailable GPU path without launching a GPU workload.
