// Temporary probe: compare sysinfo CPU numbers with the OS counters.

use sysinfo::System;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    let mut system = System::new_all();
    for round in 0..6 {
        sleep(Duration::from_secs(1));
        system.refresh_cpu_all();
        let cores: Vec<String> = system
            .cpus()
            .iter()
            .map(|c| format!("{:.0}", c.cpu_usage()))
            .collect();
        println!(
            "round {round}: global {:.1}% | cores {}",
            system.global_cpu_usage(),
            cores.join(" ")
        );
    }
}
