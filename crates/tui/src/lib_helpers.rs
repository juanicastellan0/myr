use super::*;

pub(crate) fn bookmark_base_name(
    profile: Option<&str>,
    database: Option<&str>,
    table: Option<&str>,
) -> String {
    let profile_part = profile
        .map(sanitize_bookmark_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "default".to_string());
    let database_part = database
        .map(sanitize_bookmark_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "db".to_string());
    let table_part = table
        .map(sanitize_bookmark_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "query".to_string());
    format!("{profile_part}:{database_part}.{table_part}")
}

pub(crate) fn sanitize_bookmark_segment(value: &str) -> String {
    let mut normalized = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            normalized.push(ch);
        } else {
            normalized.push('_');
        }
        if normalized.len() >= BOOKMARK_NAME_MAX_CHARS {
            break;
        }
    }
    normalized.trim_matches('_').to_string()
}

pub(crate) fn next_bookmark_name(bookmarks: &[SavedBookmark], base_name: &str) -> String {
    if bookmarks
        .iter()
        .all(|bookmark| bookmark.name.as_str() != base_name)
    {
        return base_name.to_string();
    }

    for suffix in 2..10_000 {
        let candidate = format!("{base_name}-{suffix}");
        if bookmarks
            .iter()
            .all(|bookmark| bookmark.name.as_str() != candidate)
        {
            return candidate;
        }
    }

    format!("{base_name}-{}", unix_timestamp_millis())
}

pub(crate) fn parse_password_source(value: &str) -> Option<PasswordSource> {
    match value.trim().to_ascii_lowercase().as_str() {
        "env" | "env_var" | "envvar" | "environment" | "" => Some(PasswordSource::EnvVar),
        "keyring" | "secure_store" | "secure-store" => Some(PasswordSource::Keyring),
        _ => None,
    }
}

pub(crate) fn parse_tls_mode(value: &str) -> Option<TlsMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "disabled" | "off" | "none" => Some(TlsMode::Disabled),
        "prefer" | "" => Some(TlsMode::Prefer),
        "require" | "required" => Some(TlsMode::Require),
        "verify_identity" | "verify-identity" | "verifyidentity" | "verify" => {
            Some(TlsMode::VerifyIdentity)
        }
        _ => None,
    }
}

pub(crate) fn parse_read_only_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" | "ro" | "read-only" => Some(true),
        "0" | "false" | "no" | "n" | "off" | "rw" | "read-write" | "" => Some(false),
        _ => None,
    }
}

pub(crate) fn compact_sql_for_audit(sql: &str) -> String {
    let compact = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_for_audit(&compact, AUDIT_SQL_MAX_CHARS)
}

pub(crate) fn truncate_for_audit(value: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    for ch in value.chars().take(max_chars) {
        truncated.push(ch);
    }
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

#[cfg(test)]
pub(crate) fn default_audit_trail() -> Option<FileAuditTrail> {
    None
}

#[cfg(not(test))]
pub(crate) fn default_audit_trail() -> Option<FileAuditTrail> {
    FileAuditTrail::load_default().ok()
}

#[cfg(test)]
pub(crate) fn default_bookmark_store() -> Option<FileBookmarksStore> {
    None
}

#[cfg(not(test))]
pub(crate) fn default_bookmark_store() -> Option<FileBookmarksStore> {
    FileBookmarksStore::load_default().ok()
}

pub(crate) fn previous_char_boundary(text: &str, index: usize) -> usize {
    let clamped = index.min(text.len());
    if clamped == 0 {
        return 0;
    }

    text[..clamped]
        .char_indices()
        .last()
        .map(|(position, _)| position)
        .unwrap_or(0)
}

pub(crate) fn next_char_boundary(text: &str, index: usize) -> usize {
    let clamped = index.min(text.len());
    if clamped >= text.len() {
        return text.len();
    }

    let mut iter = text[clamped..].char_indices();
    match iter.nth(1) {
        Some((offset, _)) => clamped + offset,
        None => text.len(),
    }
}

pub(crate) fn run_connect_worker(profile: ConnectionProfile) -> ConnectWorkerOutcome {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return ConnectWorkerOutcome::Failure(format!("failed to create runtime: {error}"));
        }
    };

    runtime.block_on(async move {
        let mut manager = ConnectionManager::new(MysqlConnectionBackend);
        let connect_latency =
            match tokio::time::timeout(CONNECT_TIMEOUT, manager.connect(profile.clone())).await {
                Ok(Ok(latency)) => latency,
                Ok(Err(error)) => return ConnectWorkerOutcome::Failure(error.to_string()),
                Err(_) => {
                    return ConnectWorkerOutcome::Failure(format!(
                        "connect timed out after {:.1?}",
                        CONNECT_TIMEOUT
                    ));
                }
            };

        let mut warnings = Vec::new();
        match tokio::time::timeout(CONNECT_TIMEOUT, manager.disconnect()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warnings.push(format!("disconnect warning: {error}")),
            Err(_) => warnings.push(format!(
                "disconnect timed out after {:.1?}",
                CONNECT_TIMEOUT
            )),
        }

        let data_backend = MysqlDataBackend::from_profile(&profile);
        let mut schema_cache =
            SchemaCacheService::new(data_backend.clone(), Duration::from_secs(10));
        let databases =
            match tokio::time::timeout(CONNECT_TIMEOUT, schema_cache.list_databases()).await {
                Ok(Ok(databases)) => databases,
                Ok(Err(error)) => {
                    warnings.push(format!("schema fetch failed: {error}"));
                    Vec::new()
                }
                Err(_) => {
                    warnings.push(format!(
                        "schema fetch timed out after {:.1?}",
                        CONNECT_TIMEOUT
                    ));
                    Vec::new()
                }
            };

        if let Err(error) = data_backend.disconnect().await {
            warnings.push(format!("schema backend disconnect warning: {error}"));
        }

        ConnectWorkerOutcome::Success {
            profile,
            connect_latency,
            databases,
            warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
        }
    })
}

pub(crate) fn run_query_worker(
    backend: MysqlDataBackend,
    sql: String,
    cancellation: CancellationToken,
) -> QueryWorkerOutcome {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return QueryWorkerOutcome::Failure(format!("failed to create runtime: {error}"));
        }
    };

    let runner = QueryRunner::new(backend);
    let mut results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
    match runtime.block_on(async {
        tokio::time::timeout(
            QUERY_TIMEOUT,
            runner.execute_streaming(&sql, &mut results, &cancellation),
        )
        .await
    }) {
        Ok(Ok(summary)) => QueryWorkerOutcome::Success {
            results,
            rows_streamed: summary.rows_streamed,
            was_cancelled: summary.was_cancelled,
            elapsed: summary.elapsed,
        },
        Ok(Err(error)) => QueryWorkerOutcome::Failure(error.to_string()),
        Err(_) => {
            cancellation.cancel();
            QueryWorkerOutcome::Failure(format!("query timed out after {:.1?}", QUERY_TIMEOUT))
        }
    }
}

pub(crate) fn is_transient_query_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "temporary",
        "connection reset",
        "connection refused",
        "connection closed",
        "broken pipe",
        "server has gone away",
        "lost connection",
        "pool was disconnect",
        "i/o error",
        "io error",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn is_connection_lost_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "pool was disconnect",
        "server has gone away",
        "lost connection",
        "connection reset",
        "connection refused",
        "connection closed",
        "broken pipe",
        "not connected",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn quote_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

pub(crate) fn candidate_key_column(columns: &[String]) -> Option<String> {
    if let Some(column) = columns
        .iter()
        .find(|column| column.eq_ignore_ascii_case("id"))
    {
        return Some(column.clone());
    }

    columns
        .iter()
        .find(|column| column.to_ascii_lowercase().ends_with("_id"))
        .cloned()
}

pub(crate) fn extract_key_bounds(
    results: &ResultsRingBuffer<QueryRow>,
    columns: &[String],
    key_column: &str,
) -> (Option<String>, Option<String>) {
    let Some(key_index) = columns
        .iter()
        .position(|column| column.eq_ignore_ascii_case(key_column))
    else {
        return (None, None);
    };

    let first = results
        .get(0)
        .and_then(|row| row.values.get(key_index))
        .cloned();
    let last = results
        .len()
        .checked_sub(1)
        .and_then(|index| results.get(index))
        .and_then(|row| row.values.get(key_index))
        .cloned();
    (first, last)
}

pub(crate) fn export_file_path(extension: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    std::env::temp_dir().join(format!("myr-export-{timestamp}.{extension}"))
}

pub(crate) fn block_on_result<T, E, F>(future: F) -> Result<T, String>
where
    E: std::fmt::Display,
    F: std::future::Future<Output = Result<T, E>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to create runtime: {error}"))?;

    runtime.block_on(future).map_err(|error| error.to_string())
}

pub(crate) fn demo_column_schemas() -> Vec<ColumnSchema> {
    vec![
        ColumnSchema {
            name: "id".to_string(),
            data_type: "bigint unsigned".to_string(),
            nullable: false,
            default_value: None,
        },
        ColumnSchema {
            name: "email".to_string(),
            data_type: "varchar(255)".to_string(),
            nullable: false,
            default_value: None,
        },
        ColumnSchema {
            name: "created_at".to_string(),
            data_type: "timestamp".to_string(),
            nullable: false,
            default_value: Some("CURRENT_TIMESTAMP".to_string()),
        },
        ColumnSchema {
            name: "updated_at".to_string(),
            data_type: "timestamp".to_string(),
            nullable: false,
            default_value: Some("CURRENT_TIMESTAMP".to_string()),
        },
    ]
}

pub(crate) fn map_key_event(key: KeyEvent) -> Option<Msg> {
    if key.modifiers == KeyModifiers::CONTROL {
        return match key.code {
            KeyCode::Char('p') => Some(Msg::TogglePalette),
            KeyCode::Char('u') => Some(Msg::ClearInput),
            KeyCode::Char('c') => Some(Msg::CancelQuery),
            KeyCode::Char('j') | KeyCode::Enter => Some(Msg::InsertNewline),
            _ => None,
        };
    }

    if key.modifiers == KeyModifiers::ALT {
        return match key.code {
            KeyCode::Char('k') => Some(Msg::Navigate(DirectionKey::Up)),
            KeyCode::Char('j') => Some(Msg::Navigate(DirectionKey::Down)),
            KeyCode::Char('h') => Some(Msg::Navigate(DirectionKey::Left)),
            KeyCode::Char('l') => Some(Msg::Navigate(DirectionKey::Right)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('?') => Some(Msg::ToggleHelp),
        KeyCode::Esc => Some(Msg::TogglePalette),
        KeyCode::Tab => Some(Msg::NextPane),
        KeyCode::F(5) => Some(Msg::Connect),
        KeyCode::F(6) => Some(Msg::GoConnectionWizard),
        KeyCode::F(10) => Some(Msg::Quit),
        KeyCode::F(2) => Some(Msg::TogglePerfOverlay),
        KeyCode::F(3) => Some(Msg::ToggleSafeMode),
        KeyCode::F(4) => Some(Msg::ToggleSchemaColumnView),
        KeyCode::Enter => Some(Msg::Submit),
        KeyCode::Backspace => Some(Msg::Backspace),
        KeyCode::Up => Some(Msg::Navigate(DirectionKey::Up)),
        KeyCode::Down => Some(Msg::Navigate(DirectionKey::Down)),
        KeyCode::Left => Some(Msg::Navigate(DirectionKey::Left)),
        KeyCode::Right => Some(Msg::Navigate(DirectionKey::Right)),
        KeyCode::Char('1') => Some(Msg::InvokeActionSlot(0)),
        KeyCode::Char('2') => Some(Msg::InvokeActionSlot(1)),
        KeyCode::Char('3') => Some(Msg::InvokeActionSlot(2)),
        KeyCode::Char('4') => Some(Msg::InvokeActionSlot(3)),
        KeyCode::Char('5') => Some(Msg::InvokeActionSlot(4)),
        KeyCode::Char('6') => Some(Msg::InvokeActionSlot(5)),
        KeyCode::Char('7') => Some(Msg::InvokeActionSlot(6)),
        KeyCode::Char(ch) => Some(Msg::InputChar(ch)),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn suggest_limit_in_editor(query: &str) -> Option<String> {
    myr_core::actions_engine::suggest_preview_limit(query, 200)
}
