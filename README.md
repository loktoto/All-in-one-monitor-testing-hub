# Hardware Hub

A local, Windows-first Rust desktop MVP for monitoring a computer and manually running small, bounded stability checks. No account, cloud service, or automatic stress test is involved. The interface is in English; notes below are also provided in 繁體中文.

## Run on Windows

1. Install the current stable Rust toolchain with the MSVC target and the Visual Studio C++ build tools.
2. In this folder run `cargo run --release`. Or use the Windows executable artifact from a successful GitHub Actions run.
3. Open **Overview** for live readings, **Stress tests** to configure and start a test, and **Reports** to export session results. Exports use `create_new` and never overwrite an existing file.

Requires a working graphics driver for the window. GPU compute requires a compatible wgpu hardware adapter and driver. Linux development is supported where a compatible X11/graphics stack is available, but Windows is the primary target.

## Measurements and limitations

- `sysinfo` supplies CPU model and aggregate/per-logical-core load, memory, and mounted volume capacity, type and I/O counters. Volume identity is not necessarily a physical drive; a physical disk may have multiple volumes. The OS can report unknown disk type, unavailable counters or omit devices; zero counters are not proof of zero activity. Values reflect the operating system, not a hardware diagnosis.
- `wgpu` enumerates graphics adapters. NVIDIA's optional NVML driver library supplies GPU load, temperature and VRAM when available. Other GPUs have adapter inventory only; AMD/Intel load, temperature and VRAM are not available. More than one graphics backend can expose the same adapter.
- CPU, memory and storage temperature, fan speeds, power, SMART data and health are **not available**. There is **no thermal automatic stop**. Use a compatible external monitoring tool and stop if temperatures or system behavior are unsafe. Readings can be unavailable due to drivers, OS, device, permissions or virtualization.
- Disk throughput counters are bytes since the last refresh (approximately one second), not a guaranteed precise per-second rate. They are OS-provided and can be unavailable or zero even when activity occurs.

## Manual tests

Only one test runs at a time; choose **Stop test** to request cancellation. Closing the window while a test runs requests a stop and keeps the window open until it finishes. CPU uses at most eight workers and may not load all cores; memory covers only the allocated region. CPU checks repeated integer results against a run-time reference; memory writes and verifies alternating byte patterns; file I/O writes, flushes and reads back a newly created unique scratch file in the chosen directory; GPU runs a small integer shader and verifies every result on the CPU; this is a light compute check and not a full GPU stress test. These checks can find some errors, but do not prove hardware is healthy or replace comprehensive diagnostics. The GPU test selects the default high-performance hardware adapter, not a user-selected GPU.

Duration is 1–600 seconds; CPU is limited to 1–8 threads; memory is at most 256 MiB and at most one quarter of currently available memory; the file size is 1–256 MiB and total writes are capped at 1 GiB per run. Tests may finish before the duration limit. File writes consume free space and cause SSD wear; use a suitable directory. No raw disk or existing user file is opened for writing. The owned scratch file is removed on normal completion, failure or cancellation; a power loss, crash or forced termination can leave it behind. Stop is checked between disk chunks and while polling GPU completion; a blocked disk operation or GPU driver can still delay stopping. Unfinished GPU work is not counted as verified. GPU initialization also counts toward the time limit. A run with zero full verification passes is reported as inconclusive.

Reports contain test type, status, details, elapsed time and Unix timestamp. They are local plain text and are retained in memory only until exported.

## Development

```sh
cargo fmt --check
cargo test
cargo run
```

Windows CI builds the MSVC release executable and publishes it as a workflow artifact. It also runs the tests on Windows. See [architecture and roadmap](docs/architecture.md) for boundaries and reference notes. Test on actual Windows hardware with representative NVIDIA/AMD/Intel adapters and storage devices before relying on it. No real Windows hardware validation is implied by a CI build.

## 繁體中文

這是以 Windows 為優先的本機監控與手動測試工具。開啟後不會自動執行壓力測試。可查看 CPU、記憶體、磁碟區與 GPU 清單；只有可用的 NVIDIA NVML 驅動能提供 GPU 負載、溫度與顯示記憶體資訊。CPU、記憶體及磁碟溫度、SMART 與風扇轉速尚未支援，也沒有過熱自動停止功能。請使用相容的外部工具監看溫度。

測試有時間與資源上限。磁碟測試只建立獨立暫存檔並驗證讀回資料，最多寫入 1 GiB，但仍會造成 SSD 耗損；異常斷電可能留下暫存檔。停止可能需等待目前的磁碟或 GPU 操作完成。結果可從 Reports 匯出，未匯出的結果會在程式結束時消失。
