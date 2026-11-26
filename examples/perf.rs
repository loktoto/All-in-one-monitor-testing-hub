//! Micro-profile of the per-frame read path and high-core / high-sensor scaling.
//!
//! Run with `cargo run --release --example perf`. No hardware is touched and
//! nothing is written to disk. The numbers are indicative for this machine.

use hardware_hub::{
    history::History,
    telemetry::{Process, Snapshot, Temperature},
};
use std::{sync::Arc, time::Instant};

fn main() {
    let mut snapshot = Snapshot {
        ready: true,
        cpu_usage: 42.0,
        ..Default::default()
    };
    // Simulate a heavy workstation: 128 threads, 500 processes, 100 sensors.
    snapshot.cores = (0..128).map(|i| ((i * 7) % 100) as f32).collect();
    snapshot.core_mhz = vec![3600; 128];
    snapshot.processes = (0..500)
        .map(|pid| Process {
            pid,
            name: format!("process-{pid}"),
            cpu_usage: pid as f32 / 10.0,
            memory: 1_000_000,
        })
        .collect();
    snapshot.temperatures = (0..100)
        .map(|i| Temperature {
            key: format!("T{i:03}"),
            label: format!("Sensor {i}"),
            source: "hwmon",
            celsius: Some(40.0 + (i % 45) as f32),
            min: Some(32.0),
            max: Some(78.0),
        })
        .collect();
    let shared = Arc::new(snapshot.clone());

    let iterations = 10_000;
    let started = Instant::now();
    for _ in 0..iterations {
        let cloned = snapshot.clone();
        std::hint::black_box(&cloned);
    }
    let deep = started.elapsed().as_nanos() as f64 / iterations as f64;

    let started = Instant::now();
    for _ in 0..iterations {
        let cloned = Arc::clone(&shared);
        std::hint::black_box(&cloned);
    }
    let shared_ns = started.elapsed().as_nanos() as f64 / iterations as f64;

    let mut history = History::default();
    let mut at = Instant::now();
    for _ in 0..60 {
        let mut sample = snapshot.clone();
        sample.updated_at = Some(at);
        history.record(&sample, at);
        at += std::time::Duration::from_millis(1000);
    }
    let started = Instant::now();
    for _ in 0..iterations {
        let segments = hardware_hub::history::segments(&history, at, |s| s.cpu);
        std::hint::black_box(&segments);
    }
    let graph_ns = started.elapsed().as_nanos() as f64 / iterations as f64;

    println!("Workstation profile: 128 threads · 500 processes · 100 sensors");
    println!("Snapshot deep clone: {deep:.0} ns/read (previous UI path)");
    println!("Arc snapshot read:   {shared_ns:.0} ns/read (current UI path)");
    println!("History graph split: {graph_ns:.0} ns/frame for 60 samples");
    if deep > 0.0 {
        println!("Snapshot read speed-up: {:.1}x", deep / shared_ns.max(1.0));
    }
}
