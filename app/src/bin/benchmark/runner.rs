use std::io;
use std::time::Instant;

use myr_adapters::mysql::MysqlDataBackend;
use myr_core::query_runner::{QueryBackend, QueryRowStream};

use crate::io_other;
use crate::model::QueryMetrics;

pub(crate) async fn run_query_benchmark(
    backend: &MysqlDataBackend,
    sql: &str,
) -> io::Result<QueryMetrics> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    let started_at = Instant::now();
    let mut first_row = None;
    let mut rows_streamed = 0_u64;

    while let Some(_row) = stream.next_row().await.map_err(io_other)? {
        rows_streamed += 1;
        if first_row.is_none() {
            first_row = Some(started_at.elapsed());
        }
    }

    Ok(QueryMetrics {
        rows_streamed,
        first_row,
        elapsed: started_at.elapsed(),
    })
}

pub(crate) async fn ensure_seed_data(
    backend: &MysqlDataBackend,
    target_rows: u64,
) -> io::Result<()> {
    execute_sql(
        backend,
        "CREATE TABLE IF NOT EXISTS events (\
         id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,\
         user_id INT NOT NULL,\
         category VARCHAR(32) NOT NULL,\
         payload VARCHAR(128) NOT NULL,\
         created_at DATETIME NOT NULL,\
         KEY idx_created_at (created_at),\
         KEY idx_user_id_id (user_id, id)\
         )",
    )
    .await?;

    let existing_rows = query_scalar_u64(backend, "SELECT COUNT(*) FROM events").await?;
    if existing_rows >= target_rows {
        return Ok(());
    }

    let mut next = existing_rows + 1;
    while next <= target_rows {
        let end = (next + 999).min(target_rows);
        execute_sql(backend, &build_insert_batch_sql(next, end)).await?;
        next = end + 1;
    }

    Ok(())
}

pub(crate) async fn execute_sql(backend: &MysqlDataBackend, sql: &str) -> io::Result<()> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    while stream.next_row().await.map_err(io_other)?.is_some() {}
    Ok(())
}

pub(crate) async fn query_scalar_u64(backend: &MysqlDataBackend, sql: &str) -> io::Result<u64> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    let row = stream
        .next_row()
        .await
        .map_err(io_other)?
        .ok_or_else(|| io_other("query returned no rows"))?;
    let value = row
        .values
        .first()
        .ok_or_else(|| io_other("query returned no columns"))?;
    value
        .parse::<u64>()
        .map_err(|error| io_other(format!("failed to parse scalar value `{value}`: {error}")))
}

pub(crate) fn build_insert_batch_sql(start: u64, end: u64) -> String {
    let mut values = Vec::with_capacity((end - start + 1) as usize);
    for index in start..=end {
        let user_id = (index % 5_000) + 1;
        let category = match index % 5 {
            0 => "search",
            1 => "play",
            2 => "pause",
            3 => "skip",
            _ => "share",
        };
        let payload = format!("payload-{index}");
        let created_offset = index % 86_400;
        values.push(format!(
            "({user_id}, '{category}', '{payload}', NOW() - INTERVAL {created_offset} SECOND)"
        ));
    }

    format!(
        "INSERT INTO events (user_id, category, payload, created_at) VALUES {}",
        values.join(",")
    )
}
