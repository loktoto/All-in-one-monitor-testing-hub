//! Read-only integration with LibreHardwareMonitor's local data.json feed.

use super::{Sensor, Unit};
use serde::Deserialize;
use std::collections::BTreeMap;

pub(super) const SOURCE: &str = "LHM";
const MAX_FEED_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct Node {
    text: String,
    sensor_id: String,
    #[serde(rename = "Type")]
    sensor_type: String,
    raw_value: serde_json::Value,
    value: String,
    children: Vec<Node>,
}

fn decode(bytes: &[u8]) -> Option<Vec<Sensor>> {
    if bytes.len() as u64 > MAX_FEED_BYTES {
        return None;
    }
    let root: Node = serde_json::from_slice(bytes).ok()?;
    if root.children.is_empty() {
        return None;
    }
    let mut rows = BTreeMap::new();
    let mut pending = root.children;
    while let Some(node) = pending.pop() {
        pending.extend(node.children);
        let id = node.sensor_id.trim();
        let is_temperature = id.starts_with('/')
            && id.len() <= 256
            && !id.chars().any(char::is_control)
            && id.rsplit_once('/').is_some_and(|(path, index)| {
                path.ends_with("/temperature") && index.parse::<u32>().is_ok()
            })
            && (node.sensor_type.is_empty() || node.sensor_type == "Temperature");
        if !is_temperature || node.text.to_ascii_lowercase().contains("distance to tjmax") {
            continue;
        }
        let label: String = node
            .text
            .trim()
            .chars()
            .filter(|ch| !ch.is_control())
            .take(120)
            .collect();
        if label.is_empty() {
            continue;
        }
        // The release feed formats RawValue as a string. Newer builds
        // also emit JSON numbers, and some feeds provide only Value.
        let value = node
            .raw_value
            .as_f64()
            .map(|v| v as f32)
            .or_else(|| {
                node.raw_value
                    .as_str()
                    .and_then(|value| parse_celsius(value, true))
            })
            .or_else(|| parse_celsius(&node.value, false))
            .filter(|v| (-20.0..=150.0).contains(v));
        let key = format!("lhm:{id}");
        let sensor = Sensor {
            key: key.clone(),
            cpu_core: is_cpu_core(id, &label),
            label,
            source: SOURCE,
            value,
            unit: Unit::Celsius,
        };
        // Repeated IDs are ambiguous and must not combine independent readings.
        rows.entry(key)
            .and_modify(|slot| *slot = None)
            .or_insert(Some(sensor));
    }
    let mut rows: Vec<_> = rows.into_values().flatten().collect();
    rows.sort_by(|a, b| channel_order(&a.key).cmp(&channel_order(&b.key)));
    Some(rows)
}

fn parse_celsius(text: &str, plain: bool) -> Option<f32> {
    let text = text.trim();
    let number = |text: &str| text.trim().replace(',', ".").parse::<f32>().ok();
    if let Some(value) = text.strip_suffix("°C") {
        number(value)
    } else if let Some(value) = text.strip_suffix("°F") {
        number(value).map(|v| (v - 32.0) * 5.0 / 9.0)
    } else if plain {
        number(text)
    } else {
        None
    }
}

fn channel_order(key: &str) -> (&str, u32) {
    key.rsplit_once('/').map_or((key, 0), |(device, channel)| {
        (device, channel.parse().unwrap_or(u32::MAX))
    })
}

fn is_cpu_core(id: &str, label: &str) -> bool {
    let cpu = id.starts_with("/intelcpu/") || id.starts_with("/amdcpu/");
    let label = label.strip_prefix("CPU ").unwrap_or(label);
    cpu && (label == "Core"
        || ["Core #", "P-Core #", "E-Core #", "Core "]
            .iter()
            .any(|prefix| {
                label
                    .strip_prefix(prefix)
                    .is_some_and(|tail| tail.parse::<u32>().is_ok())
            }))
}

#[cfg(target_os = "windows")]
pub(crate) struct Feed {
    agent: ureq::Agent,
    last_port: u16,
    retry_at: std::time::Instant,
}

#[cfg(target_os = "windows")]
impl Feed {
    pub(crate) fn new() -> Self {
        use std::time::{Duration, Instant};
        Self {
            agent: ureq::Agent::config_builder()
                .proxy(None)
                .max_redirects(0)
                .timeout_global(Some(Duration::from_millis(350)))
                .build()
                .into(),
            last_port: 0,
            retry_at: Instant::now(),
        }
    }

    pub(crate) fn sample(&mut self, port: u16) -> (super::ExternalThermals, Vec<Sensor>) {
        use super::ExternalThermals;
        use std::time::{Duration, Instant};
        let changed = self.last_port != port;
        self.last_port = port;
        if port == 0 {
            return (ExternalThermals::Disabled, Vec::new());
        }
        if changed || Instant::now() >= self.retry_at {
            if let Some(rows) = self.read(port) {
                self.retry_at = Instant::now();
                return (ExternalThermals::Connected, rows);
            }
            self.retry_at = Instant::now() + Duration::from_secs(3);
        }
        (ExternalThermals::Unavailable, Vec::new())
    }

    fn read(&self, port: u16) -> Option<Vec<Sensor>> {
        let url = format!("http://127.0.0.1:{port}/data.json");
        let mut response = self.agent.get(&url).call().ok()?;
        if response.status().as_u16() != 200 {
            return None;
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_FEED_BYTES)
            .read_to_vec()
            .ok()?;
        decode(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{"Text":"Hardware","Children":[{"Text":"Computer","Children":[
        {"Text":"CPU Core #1","Type":"Temperature","SensorId":"/intelcpu/0/temperature/2","RawValue":"53,25 °C"},
        {"Text":"Core #1","Type":"Temperature","SensorId":"/intelcpu/1/temperature/2","RawValue":59.0},
        {"Text":"Core #2 Distance to TjMax","Type":"Temperature","SensorId":"/intelcpu/0/temperature/3","RawValue":47.0},
        {"Text":"Core Average","Type":"Temperature","SensorId":"/intelcpu/0/temperature/1","RawValue":55.0},
        {"Text":"CPU Package","Type":"Temperature","SensorId":"/intelcpu/0/temperature/0","RawValue":750.0},
        {"Text":"E-Core #4","SensorId":"/intelcpu/0/temperature/5","Value":"50,5 °C"},
        {"Text":"Core #0","SensorId":"/amdcpu/0/temperature/9","RawValue":null,"Value":"-"},
        {"Text":"Board","Type":"Fan","SensorId":"/lpc/it8688e/fan/0","RawValue":900.0}
    ]}]}"#;

    #[test]
    fn decodes_distinct_cores_and_preserves_unavailable_values() {
        let rows = decode(EXAMPLE.as_bytes()).unwrap();
        assert_eq!(rows.len(), 6);
        let core_values: Vec<_> = rows
            .iter()
            .filter(|row| row.cpu_core)
            .map(|row| row.value)
            .collect();
        assert_eq!(core_values, [None, Some(53.25), Some(50.5), Some(59.0)]);
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "CPU Package")
                .unwrap()
                .value,
            None
        );
    }

    #[test]
    fn rejects_ambiguous_identifiers_and_unrelated_json() {
        assert!(decode(b"{}").is_none());
        assert!(decode(b"{").is_none());
        let rows = decode(
            br#"{"Children":[
            {"Text":"Core #1","SensorId":"/intelcpu/0/temperature/2","RawValue":46},
            {"Text":"Core #2","SensorId":"/intelcpu/0/temperature/2","RawValue":56}
        ]}"#,
        )
        .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn parses_formatted_release_values() {
        assert_eq!(parse_celsius("42.5 °C", true), Some(42.5));
        assert_eq!(parse_celsius("104.0 °F", false), Some(40.0));
        assert_eq!(parse_celsius("42,5", true), Some(42.5));
        assert_eq!(parse_celsius("42,5", false), None);
        assert_eq!(parse_celsius("-", true), None);
        assert!(is_cpu_core("/intelcpu/0/temperature/0", "CPU Core"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn requests_the_local_feed_and_rejects_redirects() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::Duration;

        for redirect in [false, true] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut client, peer) = listener.accept().unwrap();
                assert!(peer.ip().is_loopback());
                client
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 2048 {
                    let mut byte = [0];
                    client.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                assert!(request.starts_with(b"GET /data.json HTTP/1.1\r\n"));
                if redirect {
                    client.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\n\r\n").unwrap();
                } else {
                    write!(
                        client,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                        EXAMPLE.len()
                    )
                    .unwrap();
                    client.write_all(EXAMPLE.as_bytes()).unwrap();
                }
            });
            let (status, readings) = Feed::new().sample(port);
            server.join().unwrap();
            assert_eq!(
                status,
                if redirect {
                    super::super::ExternalThermals::Unavailable
                } else {
                    super::super::ExternalThermals::Connected
                }
            );
            assert_eq!(readings.len(), if redirect { 0 } else { 6 });
        }
    }
}
