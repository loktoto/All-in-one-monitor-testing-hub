# Architecture and follow-up

The egui event loop only renders snapshots and queues manual tests. A telemetry thread refreshes `sysinfo`, discovers wgpu adapters and queries optional NVML. A separate worker runs one test with an atomic cancellation flag and records a result. Memory allocations, CPU worker count, test duration, temporary file size and cumulative file writes are bounded. GPU work uses small buffers and reads back each completed dispatch. No kernel driver or elevated privilege is installed. The UI and test workers do not send telemetry over the network.

Known gaps: CPU reference is computed during the same run, so systematic faults may go undetected. Memory patterns cover only the allocated process memory and may be optimized/cached by the OS or hardware. File verification can be served by caches even after `sync_all`, and is not a raw-media test. GPU dispatch is a basic compute check, not a graphics/VRAM/thermal qualification; driver hangs can delay cancellation. NVML is dynamically discovered and absent on non-NVIDIA systems. Telemetry is best effort, and multiple API backends can enumerate the same adapter.

Future work: real Windows hardware validation, physical device-to-volume mapping, SMART and temperature providers, AMD/Intel sensor providers, clearer device selection, independent known-answer CPU vectors, richer test progress and persistent reports. Any thermal automation must explicitly handle missing/stale sensor values. A GPU worker should eventually be isolated in a process to recover from stuck drivers.

## References and licensing

[Craft Anything](https://github.com/storytold/craft-anything) describes a Rust clean-room application-building engine (MIT OR Apache-2.0). [REA](https://github.com/morluto/rea) is a reverse-engineering MCP toolkit (MIT). Their documentation was reviewed for relevance; neither is bundled, invoked, or copied. Dependencies are resolved in `Cargo.lock`; consult their upstream licenses and transitive notices before redistribution. Principal libraries: eframe/egui (MIT OR Apache-2.0), wgpu (MIT OR Apache-2.0), sysinfo (MIT), nvml-wrapper (MIT), and pollster (MIT OR Apache-2.0).
