//! Read a user-selected smartctl JSON report. Never access a device here.
use serde_json::Value;
use std::{fs::File, io::Read, path::Path};

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
    pub truncated: bool,
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
}
