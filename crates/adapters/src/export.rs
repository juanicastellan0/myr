use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use flate2::Compression as GzipCompression;
use flate2::write::GzEncoder;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportCompression {
    None,
    Gzip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonExportFormat {
    Array,
    JsonLines,
}

pub fn export_rows_to_csv(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<usize, ExportError> {
    export_rows_to_csv_with_options(path, headers, rows, ExportCompression::None)
}

pub fn export_rows_to_csv_with_options(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
    compression: ExportCompression,
) -> Result<usize, ExportError> {
    let mut writer = OutputWriter::create(path, compression)?;

    writer
        .write_all(
            headers
                .iter()
                .map(|header| csv_escape(header))
                .collect::<Vec<_>>()
                .join(",")
                .as_bytes(),
        )
        .map_err(|source| ExportError::Write {
            path: path.display().to_string(),
            source,
        })?;
    writer.write_all(b"\n").map_err(|source| ExportError::Write {
        path: path.display().to_string(),
        source,
    })?;

    for row in rows {
        let mut values = Vec::with_capacity(headers.len());
        for column_index in 0..headers.len() {
            let value = row
                .get(column_index)
                .map(std::string::String::as_str)
                .unwrap_or("");
            values.push(csv_escape(value));
        }

        writer
            .write_all(values.join(",").as_bytes())
            .map_err(|source| ExportError::Write {
                path: path.display().to_string(),
                source,
            })?;
        writer.write_all(b"\n").map_err(|source| ExportError::Write {
            path: path.display().to_string(),
            source,
        })?;
    }

    writer.finish(path)?;
    Ok(rows.len())
}

pub fn export_rows_to_json(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<usize, ExportError> {
    export_rows_to_json_with_options(
        path,
        headers,
        rows,
        JsonExportFormat::Array,
        ExportCompression::None,
    )
}

pub fn export_rows_to_json_with_options(
    path: &Path,
    headers: &[String],
    rows: &[Vec<String>],
    format: JsonExportFormat,
    compression: ExportCompression,
) -> Result<usize, ExportError> {
    let mut writer = OutputWriter::create(path, compression)?;

    match format {
        JsonExportFormat::Array => {
            writer.write_all(b"[").map_err(|source| ExportError::Write {
                path: path.display().to_string(),
                source,
            })?;
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    writer.write_all(b",").map_err(|source| ExportError::Write {
                        path: path.display().to_string(),
                        source,
                    })?;
                }
                let object = row_as_json_object(headers, row);
                serde_json::to_writer(&mut writer, &Value::Object(object))?;
            }
            writer.write_all(b"]\n").map_err(|source| ExportError::Write {
                path: path.display().to_string(),
                source,
            })?;
        }
        JsonExportFormat::JsonLines => {
            for row in rows {
                let object = row_as_json_object(headers, row);
                serde_json::to_writer(&mut writer, &Value::Object(object))?;
                writer.write_all(b"\n").map_err(|source| ExportError::Write {
                    path: path.display().to_string(),
                    source,
                })?;
            }
        }
    }

    writer.finish(path)?;
    Ok(rows.len())
}

fn row_as_json_object(headers: &[String], row: &[String]) -> Map<String, Value> {
    let mut object = Map::with_capacity(headers.len());
    for (column_index, header) in headers.iter().enumerate() {
        let value = row.get(column_index).map_or(Value::Null, |value| json!(value));
        object.insert(header.clone(), value);
    }
    object
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

enum OutputWriter {
    Plain(BufWriter<File>),
    Gzip(GzEncoder<BufWriter<File>>),
}

impl OutputWriter {
    fn create(path: &Path, compression: ExportCompression) -> Result<Self, ExportError> {
        let file = File::create(path).map_err(|source| ExportError::Write {
            path: path.display().to_string(),
            source,
        })?;
        let writer = BufWriter::new(file);
        Ok(match compression {
            ExportCompression::None => Self::Plain(writer),
            ExportCompression::Gzip => {
                Self::Gzip(GzEncoder::new(writer, GzipCompression::default()))
            }
        })
    }

    fn finish(self, path: &Path) -> Result<(), ExportError> {
        match self {
            Self::Plain(mut writer) => writer.flush().map_err(|source| ExportError::Write {
                path: path.display().to_string(),
                source,
            }),
            Self::Gzip(writer) => {
                let mut inner = writer.finish().map_err(|source| ExportError::Write {
                    path: path.display().to_string(),
                    source,
                })?;
                inner.flush().map_err(|source| ExportError::Write {
                    path: path.display().to_string(),
                    source,
                })
            }
        }
    }
}

impl Write for OutputWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(writer) => writer.write(buf),
            Self::Gzip(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(writer) => writer.flush(),
            Self::Gzip(writer) => writer.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Read;

    use flate2::read::GzDecoder;
    use tempfile::TempDir;

    use super::{
        export_rows_to_csv, export_rows_to_csv_with_options, export_rows_to_json,
        export_rows_to_json_with_options, ExportCompression, JsonExportFormat,
    };

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

    #[test]
    fn exports_rows_to_gzip_csv() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let path = temp_dir.path().join("result.csv.gz");
        let headers = vec!["id".to_string(), "name".to_string()];
        let rows = vec![vec!["1".to_string(), "alpha".to_string()]];

        let written = export_rows_to_csv_with_options(&path, &headers, &rows, ExportCompression::Gzip)
            .expect("gzip csv export failed");
        assert_eq!(written, 1);

        let file = fs::File::open(path).expect("open gzip file");
        let mut decoder = GzDecoder::new(file);
        let mut output = String::new();
        decoder
            .read_to_string(&mut output)
            .expect("decode gzip output");
        assert!(output.contains("id,name"));
        assert!(output.contains("1,alpha"));
    }

    #[test]
    fn exports_rows_to_json_lines_with_gzip() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let path = temp_dir.path().join("result.jsonl.gz");
        let headers = vec!["id".to_string(), "value".to_string()];
        let rows = vec![
            vec!["10".to_string(), "ok".to_string()],
            vec!["11".to_string(), "next".to_string()],
        ];

        let written = export_rows_to_json_with_options(
            &path,
            &headers,
            &rows,
            JsonExportFormat::JsonLines,
            ExportCompression::Gzip,
        )
        .expect("jsonl gzip export failed");
        assert_eq!(written, 2);

        let file = fs::File::open(path).expect("open gzip file");
        let mut decoder = GzDecoder::new(file);
        let mut output = String::new();
        decoder
            .read_to_string(&mut output)
            .expect("decode gzip output");
        let lines = output.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value =
            serde_json::from_str(lines[0]).expect("first line should be json");
        let second: serde_json::Value =
            serde_json::from_str(lines[1]).expect("second line should be json");
        assert_eq!(first["id"], "10");
        assert_eq!(second["value"], "next");
    }
}
