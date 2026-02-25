impl TuiApp {
    pub(super) fn export_results(&mut self, format: myr_core::actions_engine::ExportFormat) {
        if !self.has_results || self.results.is_empty() {
            self.status_line = "No results available to export".to_string();
            return;
        }

        let rows = (0..self.results.len())
            .filter_map(|index| self.results.get(index))
            .map(|row| row.values.clone())
            .collect::<Vec<_>>();
        let file_path = export_file_path(match format {
            myr_core::actions_engine::ExportFormat::Csv => "csv",
            myr_core::actions_engine::ExportFormat::Json => "json",
            myr_core::actions_engine::ExportFormat::CsvGzip => "csv.gz",
            myr_core::actions_engine::ExportFormat::JsonGzip => "json.gz",
            myr_core::actions_engine::ExportFormat::JsonLines => "jsonl",
            myr_core::actions_engine::ExportFormat::JsonLinesGzip => "jsonl.gz",
        });

        let result = match format {
            myr_core::actions_engine::ExportFormat::Csv => {
                export_rows_to_csv(&file_path, &self.result_columns, &rows)
            }
            myr_core::actions_engine::ExportFormat::Json => {
                export_rows_to_json(&file_path, &self.result_columns, &rows)
            }
            myr_core::actions_engine::ExportFormat::CsvGzip => export_rows_to_csv_with_options(
                &file_path,
                &self.result_columns,
                &rows,
                ExportCompression::Gzip,
            ),
            myr_core::actions_engine::ExportFormat::JsonGzip => export_rows_to_json_with_options(
                &file_path,
                &self.result_columns,
                &rows,
                JsonExportFormat::Array,
                ExportCompression::Gzip,
            ),
            myr_core::actions_engine::ExportFormat::JsonLines => export_rows_to_json_with_options(
                &file_path,
                &self.result_columns,
                &rows,
                JsonExportFormat::JsonLines,
                ExportCompression::None,
            ),
            myr_core::actions_engine::ExportFormat::JsonLinesGzip => {
                export_rows_to_json_with_options(
                    &file_path,
                    &self.result_columns,
                    &rows,
                    JsonExportFormat::JsonLines,
                    ExportCompression::Gzip,
                )
            }
        };

        match result {
            Ok(row_count) => {
                self.status_line = format!("Exported {row_count} rows to {}", file_path.display());
            }
            Err(error) => {
                self.status_line = format!("Export failed: {error}");
            }
        }
    }

}
