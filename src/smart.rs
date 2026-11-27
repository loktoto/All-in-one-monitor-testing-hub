//! Parse SMART reports and optionally query installed smartctl in read-only mode.
use serde_json::Value;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const MAX_REPORT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ROWS: usize = 512;

#[derive(Clone, Debug)]
pub struct Attribute {
    pub id: String,
    pub name: String,
    pub value: String,
    pub worst: String,
    pub threshold: String,
    pub raw: String,
}
#[derive(Clone, Debug)]
pub struct SmartReport {
    pub model: String,
    pub health: Option<bool>,
    pub exit_status: Option<u64>,
    pub messages: Vec<String>,
    pub attributes: Vec<Attribute>,
    pub nvme: Vec<(String, String)>,
    pub details: Vec<(String, String)>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    name: String,
    kind: String,
    protocol: String,
}

impl Device {
    pub fn label(&self) -> String {
        if self.protocol.is_empty() {
            self.name.clone()
        } else {
            format!("{} · {}", self.name, self.protocol)
        }
    }
}

pub fn scan() -> Result<Vec<Device>, String> {
    parse_scan(&command(&["--scan", "--json"])?)
}

pub fn read_device(device: &Device) -> Result<SmartReport, String> {
    parse(&command(&[
        "--all",
        "--json",
        "--device",
        &device.kind,
        "--",
        &device.name,
    ])?)
}

fn parse_scan(bytes: &[u8]) -> Result<Vec<Device>, String> {
    let json: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid device scan: {e}"))?;
    if !json
        .pointer("/smartctl/exit_status")
        .is_some_and(|s| s.as_u64() == Some(0))
    {
        return Err(first_message(&json)
            .unwrap_or("SMART device discovery failed.")
            .to_owned());
    }
    let mut devices = Vec::new();
    for row in json
        .get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(128)
    {
        let (Some(name), Some(kind)) = (
            row.get("name").and_then(Value::as_str),
            row.get("type").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !(name.starts_with("/dev/") || name.starts_with(r"\\.\PhysicalDrive"))
            || name.len() > 512
            || name.chars().any(char::is_control)
            || kind.is_empty()
            || kind.len() > 100
            || !kind
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_,+-".contains(&c))
            || devices
                .iter()
                .any(|d: &Device| d.name == name && d.kind == kind)
        {
            continue;
        }
        devices.push(Device {
            name: name.into(),
            kind: kind.into(),
            protocol: row
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(32)
                .collect(),
        });
    }
    Ok(devices)
}

fn first_message(json: &Value) -> Option<&str> {
    json.pointer("/smartctl/messages/0/string")
        .and_then(Value::as_str)
}

fn executable() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    let binary = if cfg!(windows) {
        "smartctl.exe"
    } else {
        "smartctl"
    };
    let mut folders: Vec<PathBuf> = std::env::split_paths(&paths)
        .filter(|p| p.is_absolute())
        .collect();
    #[cfg(windows)]
    for name in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(folder) = std::env::var_os(name) {
            folders.push(PathBuf::from(folder).join("smartmontools").join("bin"));
        }
    }
    #[cfg(unix)]
    folders.extend(["/usr/sbin", "/usr/local/sbin", "/opt/homebrew/sbin"].map(PathBuf::from));
    folders
        .into_iter()
        .map(|folder| folder.join(binary))
        .find(|path| path.is_absolute() && path.is_file())
}

fn command(args: &[&str]) -> Result<Vec<u8>, String> {
    let program = executable().ok_or("Install smartmontools to scan and read drives directly.")?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start smartctl: {e}"))?;
    let stdout = child.stdout.take().expect("requested a pipe");
    std::thread::scope(|scope| {
        let reader = scope.spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(MAX_REPORT_BYTES + 1).read_to_end(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes)
        });
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if started.elapsed() < Duration::from_secs(10) => {
                    std::thread::sleep(Duration::from_millis(40))
                }
                outcome => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(outcome.err().map_or_else(
                        || "SMART request timed out.".into(),
                        |e| format!("Cannot wait for smartctl: {e}"),
                    ));
                }
            }
        }
        let bytes = reader
            .join()
            .map_err(|_| "SMART output reader stopped.")?
            .map_err(|e| format!("Cannot read smartctl output: {e}"))?;
        if bytes.len() as u64 > MAX_REPORT_BYTES {
            return Err("SMART output exceeds the 4 MiB limit.".into());
        }
        // smartctl uses nonzero exit bits for useful SMART warnings, alongside JSON.
        Ok(bytes)
    })
}
fn field(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(v)) if !v.is_empty() => v.clone(),
        Some(Value::Number(v)) => v.to_string(),
        Some(Value::Bool(v)) => v.to_string(),
        _ => "—".into(),
    }
}
pub fn read(path: &Path) -> Result<SmartReport, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("Cannot inspect report: {e}"))?;
    if !metadata.is_file() {
        return Err("Choose a regular report file.".into());
    }
    let file = File::open(path).map_err(|e| format!("Cannot open report: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read report: {e}"))?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err("Report exceeds the 4 MiB limit.".into());
    }
    parse(&bytes)
}
fn parse(bytes: &[u8]) -> Result<SmartReport, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("Invalid JSON: {e}"))?;
    if !v.get("json_format_version").is_some_and(Value::is_array)
        || !v.get("smartctl").is_some_and(Value::is_object)
    {
        return Err("Not a recognized smartctl JSON report.".into());
    }
    let all_rows = v
        .pointer("/ata_smart_attributes/table")
        .and_then(Value::as_array);
    let attributes = all_rows
        .into_iter()
        .flatten()
        .take(MAX_ROWS)
        .map(|r| Attribute {
            id: field(r.get("id")),
            name: field(r.get("name")),
            value: field(r.get("value")),
            worst: field(r.get("worst")),
            threshold: field(r.get("thresh")),
            raw: field(r.pointer("/raw/string").or_else(|| r.pointer("/raw/value"))),
        })
        .collect();
    let mut nvme = Vec::new();
    for (key, label) in [
        ("critical_warning", "Critical warning (raw)"),
        ("temperature", "Temperature (°C)"),
        ("available_spare", "Available spare (%)"),
        ("available_spare_threshold", "Spare threshold (%)"),
        ("percentage_used", "Endurance used (%)"),
        ("data_units_read", "Data units read"),
        ("data_units_written", "Data units written"),
        ("power_cycles", "Power cycles"),
        ("power_on_hours", "Power-on hours"),
        ("unsafe_shutdowns", "Unsafe shutdowns"),
        ("media_errors", "Media errors"),
        ("num_err_log_entries", "Error log entries"),
    ] {
        if let Some(value) = v
            .get("nvme_smart_health_information_log")
            .and_then(|n| n.get(key))
        {
            nvme.push((label.into(), field(Some(value))));
        }
    }
    let messages = v
        .pointer("/smartctl/messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(20)
        .filter_map(|m| m.get("string").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    Ok(SmartReport {
        model: field(
            v.get("model_name")
                .or_else(|| v.get("device").and_then(|d| d.get("name"))),
        ),
        health: v.pointer("/smart_status/passed").and_then(Value::as_bool),
        exit_status: v.pointer("/smartctl/exit_status").and_then(Value::as_u64),
        messages,
        attributes,
        nvme,
        details: [
            ("Interface", "/device/protocol"),
            ("Firmware", "/firmware_version"),
            ("Capacity (bytes)", "/user_capacity/bytes"),
            ("Temperature (°C)", "/temperature/current"),
            ("Power-on hours", "/power_on_time/hours"),
            ("Power cycles", "/power_cycle_count"),
            ("Rotation (RPM)", "/rotation_rate"),
            ("Current transport speed", "/interface_speed/current/string"),
            ("Supported transport speed", "/interface_speed/max/string"),
        ]
        .into_iter()
        .filter_map(|(label, path)| {
            v.pointer(path)
                .map(|value| (label.into(), field(Some(value))))
        })
        .collect(),
        truncated: all_rows.is_some_and(|v| v.len() > MAX_ROWS),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_ata_attributes_and_unknown_health() {
        let r = parse(br#"{"json_format_version":[1,0],"smartctl":{"exit_status":8},"ata_smart_attributes":{"table":[{"id":5,"name":"Reallocated","value":100,"worst":99,"thresh":10,"raw":{"string":"1"}}]}}"#).unwrap();
        assert_eq!(r.health, None);
        assert_eq!(r.exit_status, Some(8));
        assert_eq!(r.attributes[0].raw, "1");
        assert_eq!(r.attributes[0].threshold, "10");
    }
    #[test]
    fn preserves_nvme_fields_and_refuses_unrelated_json() {
        let r = parse(br#"{"json_format_version":[1,0],"smartctl":{},"smart_status":{"passed":true},"nvme_smart_health_information_log":{"percentage_used":8,"media_errors":0}}"#).unwrap();
        assert_eq!(r.health, Some(true));
        assert!(r
            .nvme
            .iter()
            .any(|(name, value)| name == "Endurance used (%)" && value == "8"));
        assert!(parse(br#"{"smart_status":{"passed":true}}"#).is_err());
    }

    #[test]
    fn scan_keeps_device_type_and_ignores_duplicates_and_command_like_names() {
        let devices = parse_scan(
            br#"{"smartctl":{"exit_status":0},"devices":[
            {"name":"/dev/nvme0","type":"nvme","protocol":"NVMe"},
            {"name":"/dev/nvme0","type":"nvme","protocol":"NVMe"},
            {"name":"/dev/sda","type":"sat,12","protocol":"ATA"},
            {"name":"--test=short","type":"ata"},
            {"name":"/dev/sdz","type":"invalid type"}
        ]}"#,
        )
        .unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].label(), "/dev/nvme0 · NVMe");
        assert_eq!(devices[1].kind, "sat,12");
        assert_eq!(
            parse_scan(
                br#"{"smartctl":{"exit_status":2,"messages":[{"string":"Access denied"}]}}"#
            )
            .unwrap_err(),
            "Access denied"
        );
    }

    #[test]
    fn drive_details_distinguish_zero_from_missing_and_preserve_failure() {
        let report = parse(
            br#"{"json_format_version":[1,0],"smartctl":{"exit_status":8},
          "smart_status":{"passed":false},"temperature":{"current":33},"power_cycle_count":0,
          "firmware_version":"A1"}"#,
        )
        .unwrap();
        assert_eq!(report.health, Some(false));
        assert_eq!(report.exit_status, Some(8));
        assert!(report
            .details
            .contains(&("Power cycles".into(), "0".into())));
        assert!(report
            .details
            .contains(&("Temperature (°C)".into(), "33".into())));
        assert!(!report
            .details
            .iter()
            .any(|(label, _)| label == "Power-on hours"));
    }

    #[test]
    #[ignore = "requires smartmontools and read access to a physical drive"]
    fn reads_a_live_drive_report() {
        let devices = scan().unwrap();
        let device = devices.first().expect("smartctl discovered a drive");
        let report = read_device(device).unwrap();
        println!(
            "{} · health {:?} · {} details · {} SMART attributes",
            device.label(),
            report.health,
            report.details.len() + report.nvme.len(),
            report.attributes.len()
        );
        assert!(
            report.health.is_some() || !report.nvme.is_empty() || !report.attributes.is_empty(),
            "{:?}",
            report.messages
        );
    }
}
