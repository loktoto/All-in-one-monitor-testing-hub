//! The optional HWiNFO Gadget export publishes selected readings in the registry.

use super::{Sensor, Unit};
use std::collections::BTreeMap;

fn decode(rows: impl IntoIterator<Item = (String, String, String)>) -> Vec<Sensor> {
    let mut sensors = BTreeMap::new();
    for (device, label, formatted) in rows {
        let device = device.trim();
        let label = label.trim();
        if [device, label]
            .iter()
            .any(|s| s.is_empty() || s.len() > 512 || s.chars().any(char::is_control))
            || label.to_ascii_lowercase().contains("distance to tjmax")
        {
            continue;
        }
        let formatted = formatted.trim();
        let (number, unit, fahrenheit) = if let Some(s) = formatted.strip_suffix("°C") {
            (s, Unit::Celsius, false)
        } else if let Some(s) = formatted.strip_suffix("°F") {
            (s, Unit::Celsius, true)
        } else if let Some(s) = formatted.strip_suffix("RPM") {
            (s, Unit::Rpm, false)
        } else {
            continue;
        };
        let value = number
            .trim()
            .replace(',', ".")
            .parse::<f32>()
            .ok()
            .map(|n| {
                if fahrenheit {
                    (n - 32.0) * 5.0 / 9.0
                } else {
                    n
                }
            })
            .filter(|n| match unit {
                Unit::Celsius => (-20.0..=150.0).contains(n),
                Unit::Rpm => (0.0..=20_000.0).contains(n),
            });
        // Gadget indices can change as the exported selection changes.
        let key = format!("hwinfo:{unit:?}:{}:{device}/{label}", device.len());
        let cpu_core = unit == Unit::Celsius
            && device.starts_with("CPU [#")
            && ["Core ", "P-Core ", "E-Core "].iter().any(|prefix| {
                label.strip_prefix(prefix).is_some_and(|n| {
                    let n = n.trim_start_matches('#');
                    let (id, qualifier) = n.split_once(' ').unwrap_or((n, ""));
                    id.parse::<u32>().is_ok() && matches!(qualifier, "" | "(P-core)" | "(E-core)")
                })
            });
        let sensor = Sensor {
            key: key.clone(),
            label: label.to_owned(),
            source: "HWiNFO",
            value,
            unit,
            cpu_core,
        };
        sensors
            .entry(key)
            .and_modify(|row| *row = None)
            .or_insert(Some(sensor));
    }
    sensors.into_values().flatten().collect()
}

#[cfg(target_os = "windows")]
pub(crate) fn sample(enabled: bool) -> (super::ExternalThermals, Vec<Sensor>) {
    use super::ExternalThermals;
    use winreg::{enums::*, RegKey};

    if !enabled {
        return (ExternalThermals::Disabled, Vec::new());
    }
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let key = RegKey::predef(hive)
            .open_subkey_with_flags(r"SOFTWARE\HWiNFO64\VSB", KEY_READ | KEY_WOW64_64KEY);
        if let Some(rows) = key.ok().and_then(|key| read_key(&key)) {
            return (ExternalThermals::Connected, rows);
        }
    }
    (ExternalThermals::Unavailable, Vec::new())
}

#[cfg(target_os = "windows")]
fn read_key(key: &winreg::RegKey) -> Option<Vec<Sensor>> {
    use std::time::UNIX_EPOCH;
    use winreg::RegKeyMetadata;

    const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
    const TICKS_PER_SECOND: u64 = 10_000_000;
    let ticks = |info: &RegKeyMetadata| {
        (u64::from(info.last_write_time.dwHighDateTime) << 32)
            | u64::from(info.last_write_time.dwLowDateTime)
    };
    let initial = key.query_info().ok()?;
    let written = ticks(&initial).checked_sub(UNIX_EPOCH_TICKS)?;
    let now = UNIX_EPOCH.elapsed().ok()?.as_nanos() / 100;
    if now.abs_diff(u128::from(written)) > u128::from(30 * TICKS_PER_SECOND)
        || !(1..=16_384).contains(&initial.values)
        || initial.max_value_len > 4096
        || initial.max_value_name_len > 128
    {
        return None;
    }

    let mut values = std::collections::HashMap::new();
    for entry in key.enum_values().take(initial.values as usize) {
        let (name, raw) = entry.ok()?;
        if let Ok(value) = winreg::types::FromRegValue::from_reg_value(&raw) {
            values.insert(name, value);
        }
    }
    if ticks(&key.query_info().ok()?) != ticks(&initial) {
        return None;
    }
    Some(decode(values.iter().filter_map(
        |(name, device): (&String, &String)| {
            let index = name.strip_prefix("Sensor")?;
            index.parse::<u32>().ok()?;
            let label = values.get(&format!("Label{index}"))?;
            let reading = values.get(&format!("Value{index}"))?;
            Some((device.clone(), label.clone(), reading.clone()))
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_core_package_ccd_and_fan_keep_distinct_identities() {
        let rows = decode(
            [
                ("CPU [#0]: Intel", "Core 0 (P-core)", "113 °F"),
                ("CPU [#1]: AMD", "Core 0", "52,5 °C"),
                ("CPU [#1]: AMD", "CPU (Tctl/Tdie)", "58 °C"),
                ("CPU [#1]: AMD", "CPU CCD1 (Tdie)", "55 °C"),
                ("CPU [#1]: AMD", "Core 1 Distance to TjMAX", "55 °C"),
                ("GPU [#0]: GPU", "Core 0", "60 °C"),
                ("Board", "Fan 2", "1200 RPM"),
                ("CPU [#0]: Intel", "Core 1", "2.3 GHz"),
            ]
            .map(|(d, l, v)| (d.to_owned(), l.to_owned(), v.to_owned())),
        );
        assert_eq!(rows.len(), 6);
        let cores: Vec<_> = rows.iter().filter(|s| s.cpu_core).collect();
        assert_eq!(cores.len(), 2);
        assert_ne!(cores[0].key, cores[1].key);
        assert!(cores.iter().any(|s| s.value == Some(45.0)));
        assert!(cores.iter().any(|s| s.value == Some(52.5)));
        assert!(rows
            .iter()
            .any(|s| s.unit == Unit::Rpm && s.value == Some(1200.0)));
    }

    #[test]
    fn ambiguous_labels_and_implausible_values_do_not_generate_measurements() {
        let rows = decode(
            [
                ("Board", "System", "42 °C"),
                ("Board", "System", "43 °C"),
                ("Board", "VRM", "300 °C"),
            ]
            .map(|(d, l, v)| (d.to_owned(), l.to_owned(), v.to_owned())),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "VRM");
        assert_eq!(rows[0].value, None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn reads_a_sparse_registry_export_without_using_the_real_gadget_key() {
        use std::time::UNIX_EPOCH;
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};

        let hive = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(
            "Software\\HardwareHubTest-{}-{}",
            std::process::id(),
            UNIX_EPOCH.elapsed().unwrap().as_nanos()
        );
        let readings = (|| {
            let (key, _) = hive.create_subkey(&path)?;
            key.set_value("Sensor12", &"CPU [#0]: Test chip")?;
            key.set_value("Label12", &"Core 1")?;
            key.set_value("Value12", &"49 °C")?;
            key.set_value("Sensor100", &"Board")?;
            key.set_value("Label100", &"Fan")?;
            key.set_value("Value100", &"1510 RPM")?;
            Ok::<_, std::io::Error>(read_key(&key))
        })();
        hive.delete_subkey(&path).unwrap();
        let readings = readings.unwrap().unwrap();
        assert!(readings.iter().any(|s| s.cpu_core && s.value == Some(49.0)));
        assert!(readings
            .iter()
            .any(|s| s.unit == Unit::Rpm && s.value == Some(1510.0)));
    }
}
