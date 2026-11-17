// Temporary probe: print the sensor readings this process can see.

fn main() {
    let Some(mut reader) = hardware_hub::sensors::open() else {
        println!("provider: none");
        return;
    };
    println!("provider: open");
    for round in 0..3 {
        let readings = reader.readings();
        println!(
            "round {round}: {} temperatures, {} fans",
            readings.temps.len(),
            readings.fans.len()
        );
        for sensor in &readings.temps {
            println!(
                "  {} | {} | {:?} °C",
                sensor.key, sensor.label, sensor.value
            );
        }
        for sensor in &readings.fans {
            println!(
                "  {} | {} | {:?} RPM",
                sensor.key, sensor.label, sensor.value
            );
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
