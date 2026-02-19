use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SqlGenerationError {
    #[error("database name cannot be empty")]
    EmptyDatabaseName,
    #[error("table name cannot be empty")]
    EmptyTableName,
    #[error("column name cannot be empty")]
    EmptyColumnName,
    #[error("count estimate requires an explicit database name")]
    MissingDatabaseForEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlTarget<'a> {
    pub database: Option<&'a str>,
    pub table: &'a str,
}

impl<'a> SqlTarget<'a> {
    pub fn new(database: Option<&'a str>, table: &'a str) -> Result<Self, SqlGenerationError> {
        if table.trim().is_empty() {
            return Err(SqlGenerationError::EmptyTableName);
        }
        if let Some(database_name) = database {
            if database_name.trim().is_empty() {
                return Err(SqlGenerationError::EmptyDatabaseName);
            }
        }
        Ok(Self { database, table })
    }
}

#[must_use]
pub fn quote_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn qualified_table_sql(target: &SqlTarget<'_>) -> String {
    match target.database {
        Some(database) => format!(
            "{}.{}",
            quote_identifier(database),
            quote_identifier(target.table)
        ),
        None => quote_identifier(target.table),
    }
}

pub fn preview_select_sql(target: &SqlTarget<'_>, limit: usize) -> String {
    format!(
        "SELECT * FROM {} LIMIT {}",
        qualified_table_sql(target),
        limit
    )
}

pub fn describe_table_sql(target: &SqlTarget<'_>) -> String {
    format!("DESCRIBE {}", qualified_table_sql(target))
}

pub fn show_create_table_sql(target: &SqlTarget<'_>) -> String {
    format!("SHOW CREATE TABLE {}", qualified_table_sql(target))
}

pub fn show_index_sql(target: &SqlTarget<'_>) -> String {
    format!("SHOW INDEX FROM {}", qualified_table_sql(target))
}

pub fn select_column_preview_sql(
    target: &SqlTarget<'_>,
    column: &str,
    limit: usize,
) -> Result<String, SqlGenerationError> {
    if column.trim().is_empty() {
        return Err(SqlGenerationError::EmptyColumnName);
    }

    Ok(format!(
        "SELECT {} FROM {} LIMIT {}",
        quote_identifier(column),
        qualified_table_sql(target),
        limit
    ))
}

pub fn count_estimate_sql(target: &SqlTarget<'_>) -> Result<String, SqlGenerationError> {
    let database = target
        .database
        .ok_or(SqlGenerationError::MissingDatabaseForEstimate)?;

    Ok(format!(
        "SELECT TABLE_ROWS AS estimated_rows FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {}",
        quote_sql_string(database),
        quote_sql_string(target.table)
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        count_estimate_sql, describe_table_sql, preview_select_sql, quote_identifier,
        select_column_preview_sql, show_create_table_sql, show_index_sql, SqlGenerationError,
        SqlTarget,
    };

    #[test]
    fn quotes_identifiers_with_backticks() {
        assert_eq!(quote_identifier("users"), "`users`");
        assert_eq!(quote_identifier("odd`name"), "`odd``name`");
    }

    #[test]
    fn generates_preview_describe_and_show_statements() {
        let target = SqlTarget::new(Some("app"), "users").expect("valid target");

        assert_eq!(
            preview_select_sql(&target, 200),
            "SELECT * FROM `app`.`users` LIMIT 200"
        );
        assert_eq!(describe_table_sql(&target), "DESCRIBE `app`.`users`");
        assert_eq!(
            show_create_table_sql(&target),
            "SHOW CREATE TABLE `app`.`users`"
        );
        assert_eq!(show_index_sql(&target), "SHOW INDEX FROM `app`.`users`");
    }

    #[test]
    fn generates_column_preview_with_safe_identifier() {
        let target = SqlTarget::new(Some("app"), "users").expect("valid target");

        let sql = select_column_preview_sql(&target, "email", 50).expect("sql generation");
        assert_eq!(sql, "SELECT `email` FROM `app`.`users` LIMIT 50");
    }

    #[test]
    fn generates_count_estimate_query_against_information_schema() {
        let target = SqlTarget::new(Some("app"), "users").expect("valid target");
        let sql = count_estimate_sql(&target).expect("count estimate query");
        assert_eq!(
            sql,
            "SELECT TABLE_ROWS AS estimated_rows FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = 'app' AND TABLE_NAME = 'users'"
        );
    }

    #[test]
    fn count_estimate_requires_database() {
        let target = SqlTarget::new(None, "users").expect("valid target");
        let err = count_estimate_sql(&target).expect_err("should require database");
        assert_eq!(err, SqlGenerationError::MissingDatabaseForEstimate);
    }
}
