use std::collections::HashSet;
use std::future::Future;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use myr_adapters::export::{
    export_rows_to_csv, export_rows_to_csv_with_options, export_rows_to_json,
    export_rows_to_json_with_options, ExportCompression, JsonExportFormat,
};
use myr_adapters::mysql::{MysqlConnectionBackend, MysqlDataBackend};
use myr_core::connection_manager::ConnectionManager;
use myr_core::profiles::{ConnectionProfile, FileProfilesStore};
use myr_core::query_runner::{QueryBackend, QueryRowStream};
use myr_core::schema_cache::SchemaCacheService;

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_USER: &str = "root";
const DEFAULT_PORT: u16 = 3306;
const HEALTH_CHECK_SQL: &str = "SELECT 1 AS health_check";
const SCHEMA_CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Tui,
    Query(QueryCommand),
    Export(ExportCommand),
    Doctor(DoctorCommand),
    Help(HelpTopic),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelpTopic {
    Global,
    Query,
    Export,
    Doctor,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ConnectionArgs {
    profile: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    database: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryCommand {
    connection: ConnectionArgs,
    sql: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportCommand {
    connection: ConnectionArgs,
    sql: String,
    format: ExportFormat,
    output: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Csv,
    CsvGzip,
    Json,
    JsonGzip,
    JsonLines,
    JsonLinesGzip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorCommand {
    connection: ConnectionArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryCapture {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = parse_args()?;
    run_app(command, myr_tui::run)
}

fn run_app(
    command: CliCommand,
    run_tui: impl FnOnce() -> Result<(), myr_tui::TuiError>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = myr_core::domain_name();
    let _ = myr_adapters::adapter_name();

    match command {
        CliCommand::Tui => run_tui()?,
        CliCommand::Query(command) => run_async(run_query_command(command))?,
        CliCommand::Export(command) => run_async(run_export_command(command))?,
        CliCommand::Doctor(command) => run_async(run_doctor_command(command))?,
        CliCommand::Help(topic) => print_help(topic),
    }

    Ok(())
}

fn parse_args() -> io::Result<CliCommand> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from(args: impl IntoIterator<Item = String>) -> io::Result<CliCommand> {
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(CliCommand::Tui);
    };

    match command.as_str() {
        "-h" | "--help" | "help" => Ok(CliCommand::Help(HelpTopic::Global)),
        "query" => parse_query_command(args),
        "export" => parse_export_command(args),
        "doctor" => parse_doctor_command(args),
        _ => Err(io_other(format!(
            "unknown command `{command}`. expected one of `query`, `export`, `doctor`"
        ))),
    }
}

fn parse_query_command(args: impl IntoIterator<Item = String>) -> io::Result<CliCommand> {
    let mut args = args.into_iter();
    let mut connection = ConnectionArgs::default();
    let mut sql = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "-h" | "--help" => return Ok(CliCommand::Help(HelpTopic::Query)),
            "--sql" => sql = Some(next_non_empty_value(&mut args, "--sql")?),
            _ => {
                if !parse_connection_flag(flag.as_str(), &mut args, &mut connection)? {
                    return Err(io_other(format!("unknown argument `{flag}` for `query`")));
                }
            }
        }
    }

    let Some(sql) = sql else {
        return Err(io_other("missing required `--sql` value"));
    };

    Ok(CliCommand::Query(QueryCommand { connection, sql }))
}

fn parse_export_command(args: impl IntoIterator<Item = String>) -> io::Result<CliCommand> {
    let mut args = args.into_iter();
    let mut connection = ConnectionArgs::default();
    let mut sql = None;
    let mut format = None;
    let mut output = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "-h" | "--help" => return Ok(CliCommand::Help(HelpTopic::Export)),
            "--sql" => sql = Some(next_non_empty_value(&mut args, "--sql")?),
            "--format" => {
                let raw = next_non_empty_value(&mut args, "--format")?;
                format = Some(parse_export_format(raw.as_str())?);
            }
            "--output" => {
                output = Some(PathBuf::from(next_non_empty_value(&mut args, "--output")?))
            }
            _ => {
                if !parse_connection_flag(flag.as_str(), &mut args, &mut connection)? {
                    return Err(io_other(format!("unknown argument `{flag}` for `export`")));
                }
            }
        }
    }

    let Some(sql) = sql else {
        return Err(io_other("missing required `--sql` value"));
    };
    let Some(format) = format else {
        return Err(io_other("missing required `--format` value"));
    };
    let Some(output) = output else {
        return Err(io_other("missing required `--output` value"));
    };

    Ok(CliCommand::Export(ExportCommand {
        connection,
        sql,
        format,
        output,
    }))
}

fn parse_doctor_command(args: impl IntoIterator<Item = String>) -> io::Result<CliCommand> {
    let mut args = args.into_iter();
    let mut connection = ConnectionArgs::default();

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "-h" | "--help" => return Ok(CliCommand::Help(HelpTopic::Doctor)),
            _ => {
                if !parse_connection_flag(flag.as_str(), &mut args, &mut connection)? {
                    return Err(io_other(format!("unknown argument `{flag}` for `doctor`")));
                }
            }
        }
    }

    Ok(CliCommand::Doctor(DoctorCommand { connection }))
}

fn parse_connection_flag(
    flag: &str,
    args: &mut impl Iterator<Item = String>,
    connection: &mut ConnectionArgs,
) -> io::Result<bool> {
    match flag {
        "--profile" => connection.profile = Some(next_non_empty_value(args, "--profile")?),
        "--host" => connection.host = Some(next_non_empty_value(args, "--host")?),
        "--port" => {
            let raw = next_non_empty_value(args, "--port")?;
            connection.port = Some(
                raw.parse::<u16>()
                    .map_err(|error| io_other(format!("invalid --port value: {error}")))?,
            );
        }
        "--user" => connection.user = Some(next_non_empty_value(args, "--user")?),
        "--database" => connection.database = Some(next_non_empty_value(args, "--database")?),
        _ => return Ok(false),
    }

    Ok(true)
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<String> {
    args.next()
        .ok_or_else(|| io_other(format!("missing value for `{flag}`")))
}

fn next_non_empty_value(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<String> {
    let value = next_value(args, flag)?;
    if value.trim().is_empty() {
        return Err(io_other(format!("`{flag}` value must not be empty")));
    }
    Ok(value)
}

fn parse_export_format(raw: &str) -> io::Result<ExportFormat> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "csv" => Ok(ExportFormat::Csv),
        "csv.gz" | "csv-gz" => Ok(ExportFormat::CsvGzip),
        "json" => Ok(ExportFormat::Json),
        "json.gz" | "json-gz" => Ok(ExportFormat::JsonGzip),
        "jsonl" | "json-lines" | "ndjson" => Ok(ExportFormat::JsonLines),
        "jsonl.gz" | "jsonl-gz" | "ndjson.gz" | "ndjson-gz" => Ok(ExportFormat::JsonLinesGzip),
        _ => Err(io_other(format!(
            "invalid export format `{raw}`. expected one of: csv, csv.gz, json, json.gz, jsonl, jsonl.gz"
        ))),
    }
}

fn print_help(topic: HelpTopic) {
    match topic {
        HelpTopic::Global => print_global_help(),
        HelpTopic::Query => print_query_help(),
        HelpTopic::Export => print_export_help(),
        HelpTopic::Doctor => print_doctor_help(),
    }
}

fn print_global_help() {
    println!(
        "myr-app\n\n\
Usage:\n  myr-app [COMMAND] [OPTIONS]\n\n\
Without COMMAND, starts the interactive TUI.\n\n\
Commands:\n  query   Execute SQL and stream JSON Lines to stdout\n  export  Execute SQL and write rows to a file\n  doctor  Run connection + schema + query smoke checks\n  help    Show this help\n\n\
Run `myr-app <command> --help` for command-specific options."
    );
}

fn print_query_help() {
    println!(
        "myr-app query\n\n\
Usage:\n  myr-app query --sql <query> [connection options]\n\n\
Output:\n  Streams one JSON object per row to stdout.\n\n\
Connection options:\n  --profile <name>     Use a named connection profile from profiles.toml\n  --host <host>        Override host\n  --port <port>        Override port (default fallback: 3306)\n  --user <user>        Override user\n  --database <name>    Override database\n\n\
Environment:\n  MYR_DB_PASSWORD is used for authentication when password source is env_var.\n"
    );
}

fn print_export_help() {
    println!(
        "myr-app export\n\n\
Usage:\n  myr-app export --sql <query> --format <format> --output <path> [connection options]\n\n\
Formats:\n  csv | csv.gz | json | json.gz | jsonl | jsonl.gz\n\n\
Connection options:\n  --profile <name>     Use a named connection profile from profiles.toml\n  --host <host>        Override host\n  --port <port>        Override port (default fallback: 3306)\n  --user <user>        Override user\n  --database <name>    Override database\n\n\
Environment:\n  MYR_DB_PASSWORD is used for authentication when password source is env_var.\n"
    );
}

fn print_doctor_help() {
    println!(
        "myr-app doctor\n\n\
Usage:\n  myr-app doctor [connection options]\n\n\
Checks:\n  connection ping, schema listing, and `SELECT 1` query smoke.\n\n\
Connection options:\n  --profile <name>     Use a named connection profile from profiles.toml\n  --host <host>        Override host\n  --port <port>        Override port (default fallback: 3306)\n  --user <user>        Override user\n  --database <name>    Override database\n\n\
Environment:\n  MYR_DB_PASSWORD is used for authentication when password source is env_var.\n"
    );
}

fn run_async(task: impl Future<Output = io::Result<()>>) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io_other)?;
    runtime.block_on(task)
}

async fn run_query_command(command: QueryCommand) -> io::Result<()> {
    let profile = resolve_connection_profile(&command.connection)?;
    eprintln!("query.profile={}", profile.name);

    let backend = MysqlDataBackend::from_profile(&profile);
    let query_result = stream_query_as_json_lines(&backend, &command.sql).await;
    if let Err(error) = backend.disconnect().await {
        eprintln!("query.disconnect_warning={error}");
    }

    query_result
}

async fn stream_query_as_json_lines(backend: &MysqlDataBackend, sql: &str) -> io::Result<()> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    let mut columns = normalize_column_names(stream.column_names().unwrap_or(&[]), 0);
    let mut rows_streamed = 0_u64;

    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();
    while let Some(row) = stream.next_row().await.map_err(io_other)? {
        if columns.is_empty() {
            columns =
                normalize_column_names(stream.column_names().unwrap_or(&[]), row.values.len());
        } else if columns.len() < row.values.len() {
            columns = normalize_column_names(&columns, row.values.len());
        }

        let object = row_as_json_object(&columns, &row.values);
        serde_json::to_writer(&mut stdout_lock, &serde_json::Value::Object(object))
            .map_err(io_other)?;
        stdout_lock.write_all(b"\n")?;
        rows_streamed = rows_streamed.saturating_add(1);
    }

    eprintln!("query.rows_streamed={rows_streamed}");
    if columns.is_empty() {
        eprintln!("query.columns=none");
    }

    Ok(())
}

async fn run_export_command(command: ExportCommand) -> io::Result<()> {
    let profile = resolve_connection_profile(&command.connection)?;
    let backend = MysqlDataBackend::from_profile(&profile);

    let capture_result = collect_query_rows(&backend, &command.sql).await;
    if let Err(error) = backend.disconnect().await {
        eprintln!("export.disconnect_warning={error}");
    }
    let capture = capture_result?;

    if capture.columns.is_empty() {
        return Err(io_other(
            "export requires a query that returns result-set columns",
        ));
    }

    let written = match command.format {
        ExportFormat::Csv => export_rows_to_csv(&command.output, &capture.columns, &capture.rows),
        ExportFormat::CsvGzip => export_rows_to_csv_with_options(
            &command.output,
            &capture.columns,
            &capture.rows,
            ExportCompression::Gzip,
        ),
        ExportFormat::Json => export_rows_to_json(&command.output, &capture.columns, &capture.rows),
        ExportFormat::JsonGzip => export_rows_to_json_with_options(
            &command.output,
            &capture.columns,
            &capture.rows,
            JsonExportFormat::Array,
            ExportCompression::Gzip,
        ),
        ExportFormat::JsonLines => export_rows_to_json_with_options(
            &command.output,
            &capture.columns,
            &capture.rows,
            JsonExportFormat::JsonLines,
            ExportCompression::None,
        ),
        ExportFormat::JsonLinesGzip => export_rows_to_json_with_options(
            &command.output,
            &capture.columns,
            &capture.rows,
            JsonExportFormat::JsonLines,
            ExportCompression::Gzip,
        ),
    }
    .map_err(io_other)?;

    println!("export.path={}", command.output.display());
    println!("export.rows_written={written}");
    println!("export.columns={}", capture.columns.len());
    Ok(())
}

async fn collect_query_rows(backend: &MysqlDataBackend, sql: &str) -> io::Result<QueryCapture> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    let mut columns = normalize_column_names(stream.column_names().unwrap_or(&[]), 0);
    let mut rows = Vec::new();

    while let Some(row) = stream.next_row().await.map_err(io_other)? {
        if columns.is_empty() {
            columns =
                normalize_column_names(stream.column_names().unwrap_or(&[]), row.values.len());
        } else if columns.len() < row.values.len() {
            columns = normalize_column_names(&columns, row.values.len());
        }
        rows.push(row.values);
    }

    Ok(QueryCapture { columns, rows })
}

async fn run_doctor_command(command: DoctorCommand) -> io::Result<()> {
    let profile = resolve_connection_profile(&command.connection)?;
    println!("doctor.profile={}", profile.name);

    let mut manager = ConnectionManager::new(MysqlConnectionBackend);
    let connect_latency = match manager.connect(profile.clone()).await {
        Ok(latency) => {
            println!(
                "doctor.connection=ok latency_ms={:.3}",
                latency.as_secs_f64() * 1_000.0
            );
            latency
        }
        Err(error) => {
            println!("doctor.connection=failed error={error}");
            return Err(io_other(error));
        }
    };

    let backend = MysqlDataBackend::from_profile(&profile);
    let mut schema_cache = SchemaCacheService::new(backend.clone(), SCHEMA_CACHE_TTL);
    let schema_result = schema_cache.list_databases().await;
    match &schema_result {
        Ok(databases) => println!("doctor.schema=ok databases={}", databases.len()),
        Err(error) => println!("doctor.schema=failed error={error}"),
    }

    let query_result = run_query_smoke_check(&backend).await;
    match &query_result {
        Ok(rows) => println!("doctor.query_smoke=ok rows={rows}"),
        Err(error) => println!("doctor.query_smoke=failed error={error}"),
    }

    if let Err(error) = manager.disconnect().await {
        eprintln!("doctor.disconnect_warning={error}");
    }
    if let Err(error) = backend.disconnect().await {
        eprintln!("doctor.backend_disconnect_warning={error}");
    }

    if schema_result.is_err() || query_result.is_err() {
        return Err(io_other("doctor checks failed"));
    }

    println!(
        "doctor.status=ok connect_latency_ms={:.3}",
        connect_latency.as_secs_f64() * 1_000.0
    );
    Ok(())
}

async fn run_query_smoke_check(backend: &MysqlDataBackend) -> io::Result<u64> {
    let mut stream = backend
        .start_query(HEALTH_CHECK_SQL)
        .await
        .map_err(io_other)?;
    let mut rows = 0_u64;
    while stream.next_row().await.map_err(io_other)?.is_some() {
        rows = rows.saturating_add(1);
    }

    if rows == 0 {
        return Err(io_other("health check query returned zero rows"));
    }

    Ok(rows)
}

fn resolve_connection_profile(args: &ConnectionArgs) -> io::Result<ConnectionProfile> {
    let store = FileProfilesStore::load_default().map_err(io_other)?;
    resolve_connection_profile_from_profiles(args, store.profiles())
}

fn resolve_connection_profile_from_profiles(
    args: &ConnectionArgs,
    profiles: &[ConnectionProfile],
) -> io::Result<ConnectionProfile> {
    let mut profile = if let Some(profile_name) = args.profile.as_deref() {
        profiles
            .iter()
            .find(|profile| profile.name == profile_name)
            .cloned()
            .ok_or_else(|| io_other(format!("connection profile `{profile_name}` was not found")))?
    } else if let Some(profile) = auto_selected_profile(profiles) {
        profile
    } else if args.host.is_some() || args.user.is_some() || args.port.is_some() {
        ConnectionProfile::new("cli", DEFAULT_HOST, DEFAULT_USER)
    } else {
        return Err(io_other(
            "no connection profile available; use --profile or pass --host/--user",
        ));
    };

    if let Some(host) = &args.host {
        profile.host = host.clone();
    }
    if let Some(port) = args.port {
        profile.port = port;
    } else if profile.port == 0 {
        profile.port = DEFAULT_PORT;
    }
    if let Some(user) = &args.user {
        profile.user = user.clone();
    }
    if let Some(database) = &args.database {
        profile.database = Some(database.clone());
    }

    if profile.host.trim().is_empty() {
        return Err(io_other("connection host must not be empty"));
    }
    if profile.user.trim().is_empty() {
        return Err(io_other("connection user must not be empty"));
    }

    Ok(profile)
}

fn auto_selected_profile(profiles: &[ConnectionProfile]) -> Option<ConnectionProfile> {
    profiles
        .iter()
        .find(|profile| profile.is_default)
        .cloned()
        .or_else(|| {
            profiles
                .iter()
                .find(|profile| profile.quick_reconnect)
                .cloned()
        })
        .or_else(|| (profiles.len() == 1).then(|| profiles[0].clone()))
}

fn normalize_column_names(source: &[String], fallback_len: usize) -> Vec<String> {
    let target_len = source.len().max(fallback_len);
    let mut names = Vec::with_capacity(target_len);
    let mut used = HashSet::new();

    for index in 0..target_len {
        let base = source
            .get(index)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("col_{}", index + 1));

        let mut candidate = base.clone();
        let mut suffix = 2_u64;
        while used.contains(&candidate) {
            candidate = format!("{base}_{suffix}");
            suffix = suffix.saturating_add(1);
        }
        used.insert(candidate.clone());
        names.push(candidate);
    }

    names
}

fn row_as_json_object(
    headers: &[String],
    row: &[String],
) -> serde_json::Map<String, serde_json::Value> {
    let mut object = serde_json::Map::with_capacity(headers.len());
    for (index, header) in headers.iter().enumerate() {
        let value = row.get(index).cloned().unwrap_or_default();
        object.insert(header.clone(), serde_json::Value::String(value));
    }
    object
}

fn io_other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{
        auto_selected_profile, normalize_column_names, parse_args_from, parse_export_format,
        resolve_connection_profile_from_profiles, CliCommand, ConnectionArgs, DoctorCommand,
        ExportCommand, ExportFormat, HelpTopic, QueryCommand,
    };
    use myr_core::profiles::ConnectionProfile;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn profile(name: &str) -> ConnectionProfile {
        ConnectionProfile::new(name, "127.0.0.1", "root")
    }

    #[test]
    fn run_app_returns_ok_when_tui_runner_succeeds() {
        let result = super::run_app(CliCommand::Tui, || Ok(()));
        assert!(result.is_ok());
    }

    #[test]
    fn run_app_propagates_tui_errors() {
        let result = super::run_app(CliCommand::Tui, || {
            Err(myr_tui::TuiError::Io(io::Error::other("boom")))
        });
        assert!(result.is_err());
    }

    #[test]
    fn parse_args_defaults_to_tui_mode() {
        let command = parse_args_from(Vec::<String>::new()).expect("parse should succeed");
        assert_eq!(command, CliCommand::Tui);
    }

    #[test]
    fn parse_args_detects_global_help() {
        let command = parse_args_from(args(&["--help"])).expect("parse should succeed");
        assert_eq!(command, CliCommand::Help(HelpTopic::Global));
    }

    #[test]
    fn parse_query_command_with_connection_flags() {
        let command = parse_args_from(args(&[
            "query",
            "--sql",
            "SELECT 1",
            "--profile",
            "local",
            "--host",
            "db.local",
            "--port",
            "3307",
            "--user",
            "script_user",
            "--database",
            "analytics",
        ]))
        .expect("parse should succeed");

        assert_eq!(
            command,
            CliCommand::Query(QueryCommand {
                connection: ConnectionArgs {
                    profile: Some("local".to_string()),
                    host: Some("db.local".to_string()),
                    port: Some(3307),
                    user: Some("script_user".to_string()),
                    database: Some("analytics".to_string()),
                },
                sql: "SELECT 1".to_string(),
            })
        );
    }

    #[test]
    fn parse_export_command_requires_core_flags() {
        let command = parse_args_from(args(&[
            "export",
            "--sql",
            "SELECT id FROM users",
            "--format",
            "jsonl.gz",
            "--output",
            "target/export.jsonl.gz",
        ]))
        .expect("parse should succeed");

        assert_eq!(
            command,
            CliCommand::Export(ExportCommand {
                connection: ConnectionArgs::default(),
                sql: "SELECT id FROM users".to_string(),
                format: ExportFormat::JsonLinesGzip,
                output: "target/export.jsonl.gz".into(),
            })
        );
    }

    #[test]
    fn parse_doctor_command_accepts_connection_overrides() {
        let command = parse_args_from(args(&["doctor", "--host", "127.0.0.1", "--user", "root"]))
            .expect("parse should succeed");

        assert_eq!(
            command,
            CliCommand::Doctor(DoctorCommand {
                connection: ConnectionArgs {
                    profile: None,
                    host: Some("127.0.0.1".to_string()),
                    port: None,
                    user: Some("root".to_string()),
                    database: None,
                },
            })
        );
    }

    #[test]
    fn parse_args_rejects_unknown_commands() {
        let err = parse_args_from(args(&["unknown"])).expect_err("unknown command should fail");
        assert!(err.to_string().contains("unknown command"));
    }

    #[test]
    fn parse_query_requires_sql() {
        let err = parse_args_from(args(&["query"])).expect_err("missing sql should fail");
        assert!(err.to_string().contains("missing required `--sql` value"));
    }

    #[test]
    fn parse_export_rejects_unknown_formats() {
        let err = parse_args_from(args(&[
            "export", "--sql", "SELECT 1", "--format", "yaml", "--output", "out.yaml",
        ]))
        .expect_err("invalid format should fail");
        assert!(err.to_string().contains("invalid export format"));
    }

    #[test]
    fn export_format_aliases_are_supported() {
        assert_eq!(
            parse_export_format("csv.gz").expect("csv.gz should parse"),
            ExportFormat::CsvGzip
        );
        assert_eq!(
            parse_export_format("ndjson").expect("ndjson should parse"),
            ExportFormat::JsonLines
        );
        assert_eq!(
            parse_export_format("jsonl-gz").expect("jsonl-gz should parse"),
            ExportFormat::JsonLinesGzip
        );
    }

    #[test]
    fn auto_selected_profile_prefers_default_then_quick_reconnect() {
        let mut default = profile("default");
        default.is_default = true;
        let mut quick = profile("quick");
        quick.quick_reconnect = true;
        let selected =
            auto_selected_profile(&[quick.clone(), default.clone()]).expect("should select one");
        assert_eq!(selected.name, "default");

        let selected = auto_selected_profile(&[quick.clone()]).expect("should select quick");
        assert_eq!(selected.name, "quick");
    }

    #[test]
    fn resolve_connection_profile_uses_named_profile_and_overrides() {
        let mut named = profile("prod");
        named.host = "db.prod".to_string();
        named.port = 4406;
        named.user = "app".to_string();
        named.database = Some("warehouse".to_string());

        let resolved = resolve_connection_profile_from_profiles(
            &ConnectionArgs {
                profile: Some("prod".to_string()),
                host: Some("db.override".to_string()),
                port: Some(3308),
                user: Some("batch".to_string()),
                database: Some("analytics".to_string()),
            },
            &[named],
        )
        .expect("profile should resolve");

        assert_eq!(resolved.host, "db.override");
        assert_eq!(resolved.port, 3308);
        assert_eq!(resolved.user, "batch");
        assert_eq!(resolved.database.as_deref(), Some("analytics"));
    }

    #[test]
    fn resolve_connection_profile_falls_back_to_inline_profile_for_host_user_overrides() {
        let resolved = resolve_connection_profile_from_profiles(
            &ConnectionArgs {
                profile: None,
                host: Some("127.0.0.1".to_string()),
                port: Some(3307),
                user: Some("root".to_string()),
                database: Some("myr_bench".to_string()),
            },
            &[],
        )
        .expect("inline profile should resolve");

        assert_eq!(resolved.name, "cli");
        assert_eq!(resolved.host, "127.0.0.1");
        assert_eq!(resolved.port, 3307);
        assert_eq!(resolved.user, "root");
        assert_eq!(resolved.database.as_deref(), Some("myr_bench"));
    }

    #[test]
    fn resolve_connection_profile_requires_profile_or_connection_identifiers() {
        let err = resolve_connection_profile_from_profiles(&ConnectionArgs::default(), &[])
            .expect_err("resolution should fail");
        assert!(err.to_string().contains("no connection profile available"));
    }

    #[test]
    fn resolve_connection_profile_rejects_unknown_named_profile() {
        let err = resolve_connection_profile_from_profiles(
            &ConnectionArgs {
                profile: Some("missing".to_string()),
                ..ConnectionArgs::default()
            },
            &[profile("local")],
        )
        .expect_err("resolution should fail");
        assert!(err.to_string().contains("was not found"));
    }

    #[test]
    fn normalize_column_names_fills_blanks_and_deduplicates() {
        let normalized =
            normalize_column_names(&["".to_string(), "id".to_string(), "id".to_string()], 4);
        assert_eq!(
            normalized,
            vec![
                "col_1".to_string(),
                "id".to_string(),
                "id_2".to_string(),
                "col_4".to_string()
            ]
        );
    }
}
