use std::fs;
use std::path::Path;

use serde_json::{json, Map, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("failed to write export file at {path}: {source}")]
    Write {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize JSON export: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn export_rows_to_csv(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<usize, ExportError> {
    let mut content = String::new();
    content.push_str(
        &headers
            .iter()
            .map(|header| csv_escape(header))
            .collect::<Vec<_>>()
            .join(","),
    );
    content.push('\n');

    for row in rows {
        let mut values = Vec::with_capacity(headers.len());
        for column_index in 0..headers.len() {
            let value = row
                .get(column_index)
                .map(std::string::String::as_str)
                .unwrap_or("");
            values.push(csv_escape(value));
        }
        content.push_str(&values.join(","));
        content.push('\n');
    }

    fs::write(path, content).map_err(|source| ExportError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(rows.len())
}

pub fn export_rows_to_json(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<usize, ExportError> {
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let mut object = Map::with_capacity(headers.len());
        for (column_index, header) in headers.iter().enumerate() {
            let value = row
                .get(column_index)
                .map_or(Value::Null, |value| json!(value));
            object.insert(header.clone(), value);
        }
        records.push(Value::Object(object));
    }

    let payload = serde_json::to_string_pretty(&records)?;
    fs::write(path, payload).map_err(|source| ExportError::Write {
        path: path.display().to_string(),
        source,
    })?;
    Ok(rows.len())
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{export_rows_to_csv, export_rows_to_json};

    #[test]
    fn exports_rows_to_csv_with_header_and_escaping() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let path = temp_dir.path().join("result.csv");
        let headers = vec!["id".to_string(), "name".to_string()];
        let rows = vec![
            vec!["1".to_string(), "alpha".to_string()],
            vec!["2".to_string(), "quote \"name\"".to_string()],
        ];

        let written = export_rows_to_csv(&path, &headers, &rows).expect("csv export failed");
        assert_eq!(written, 2);
        let output = fs::read_to_string(path).expect("failed to read csv output");
        assert!(output.contains("id,name"));
        assert!(output.contains("2,\"quote \"\"name\"\"\""));
    }

    #[test]
    fn exports_rows_to_json_objects_by_header() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let path = temp_dir.path().join("result.json");
        let headers = vec!["id".to_string(), "value".to_string()];
        let rows = vec![vec!["10".to_string(), "ok".to_string()]];

        let written = export_rows_to_json(&path, &headers, &rows).expect("json export failed");
        assert_eq!(written, 1);
        let output = fs::read_to_string(path).expect("failed to read json output");
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid json");
        assert_eq!(parsed[0]["id"], "10");
        assert_eq!(parsed[0]["value"], "ok");
    }
}
