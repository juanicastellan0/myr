use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlRiskReason {
    MultiStatement,
    WriteOperation(String),
    DdlOperation(String),
    TransactionControl(String),
    SessionMutation(String),
    UnknownStatement(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlSafetyAssessment {
    pub statement_count: usize,
    pub primary_keyword: Option<String>,
    pub reasons: Vec<SqlRiskReason>,
    pub normalized_sql: String,
}

impl SqlSafetyAssessment {
    #[must_use]
    pub fn is_safe_read_only(&self) -> bool {
        self.reasons.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfirmationToken(String);

impl ConfirmationToken {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDecision {
    Allow {
        assessment: SqlSafetyAssessment,
    },
    RequireConfirmation {
        token: ConfirmationToken,
        assessment: SqlSafetyAssessment,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SafeModeError {
    #[error("confirmation token is invalid or expired")]
    InvalidToken,
    #[error("confirmation token does not match the SQL statement")]
    SqlMismatch,
}

#[derive(Debug, Clone)]
struct PendingConfirmation {
    sql_fingerprint: u64,
}

#[derive(Debug, Default)]
pub struct SafeModeGuard {
    enabled: bool,
    nonce: u64,
    pending_confirmations: HashMap<String, PendingConfirmation>,
}

impl SafeModeGuard {
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.pending_confirmations.clear();
        }
    }

    pub fn evaluate(&mut self, sql: &str) -> GuardDecision {
        let assessment = assess_sql_safety(sql);
        if !self.enabled || assessment.is_safe_read_only() {
            return GuardDecision::Allow { assessment };
        }

        self.nonce = self.nonce.saturating_add(1);
        let fingerprint = fingerprint_sql(&assessment.normalized_sql);
        let token_string = format!("confirm-{}-{fingerprint:016x}", self.nonce);
        self.pending_confirmations.insert(
            token_string.clone(),
            PendingConfirmation {
                sql_fingerprint: fingerprint,
            },
        );

        GuardDecision::RequireConfirmation {
            token: ConfirmationToken(token_string),
            assessment,
        }
    }

    pub fn confirm(&mut self, token: &ConfirmationToken, sql: &str) -> Result<(), SafeModeError> {
        let Some(pending) = self.pending_confirmations.remove(token.as_str()) else {
            return Err(SafeModeError::InvalidToken);
        };

        let assessment = assess_sql_safety(sql);
        let fingerprint = fingerprint_sql(&assessment.normalized_sql);
        if pending.sql_fingerprint != fingerprint {
            return Err(SafeModeError::SqlMismatch);
        }

        Ok(())
    }
}

#[must_use]
pub fn assess_sql_safety(sql: &str) -> SqlSafetyAssessment {
    let statements = split_statements(sql);
    let statement_count = statements.len();
    let mut reasons = Vec::new();

    if statement_count > 1 {
        reasons.push(SqlRiskReason::MultiStatement);
    }

    let primary_keyword = statements
        .first()
        .and_then(|statement| first_keyword(statement));
    for statement in &statements {
        if let Some(keyword) = first_keyword(statement) {
            if is_safe_read_keyword(&keyword) {
                continue;
            }

            if is_write_keyword(&keyword) {
                reasons.push(SqlRiskReason::WriteOperation(keyword));
                continue;
            }

            if is_ddl_keyword(&keyword) {
                reasons.push(SqlRiskReason::DdlOperation(keyword));
                continue;
            }

            if is_transaction_keyword(&keyword) {
                reasons.push(SqlRiskReason::TransactionControl(keyword));
                continue;
            }

            if is_session_mutation_keyword(&keyword) {
                reasons.push(SqlRiskReason::SessionMutation(keyword));
                continue;
            }

            reasons.push(SqlRiskReason::UnknownStatement(keyword));
        }
    }

    let normalized_sql = statements.join("; ");

    SqlSafetyAssessment {
        statement_count,
        primary_keyword,
        reasons,
        normalized_sql,
    }
}

fn split_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();

    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while let Some(ch) = chars.next() {
        if in_line_comment {
            if ch == '\n' {
                in_line_comment = false;
            }
            continue;
        }

        if in_block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }

        if !in_single_quote && !in_double_quote && !in_backtick {
            if ch == '-' && chars.peek() == Some(&'-') {
                chars.next();
                in_line_comment = true;
                continue;
            }

            if ch == '#' {
                in_line_comment = true;
                continue;
            }

            if ch == '/' && chars.peek() == Some(&'*') {
                chars.next();
                in_block_comment = true;
                continue;
            }
        }

        match ch {
            '\'' if !in_double_quote && !in_backtick => {
                in_single_quote = !in_single_quote;
                current.push(ch);
            }
            '"' if !in_single_quote && !in_backtick => {
                in_double_quote = !in_double_quote;
                current.push(ch);
            }
            '`' if !in_single_quote && !in_double_quote => {
                in_backtick = !in_backtick;
                current.push(ch);
            }
            ';' if !in_single_quote && !in_double_quote && !in_backtick => {
                let statement = current.trim();
                if !statement.is_empty() {
                    statements.push(statement.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let trailing = current.trim();
    if !trailing.is_empty() {
        statements.push(trailing.to_string());
    }

    statements
}

fn first_keyword(statement: &str) -> Option<String> {
    statement
        .split_whitespace()
        .next()
        .map(|keyword| keyword.to_ascii_uppercase())
}

fn is_safe_read_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "HELP" | "USE"
    )
}

fn is_write_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "INSERT" | "UPDATE" | "DELETE" | "REPLACE" | "LOAD" | "CALL" | "DO"
    )
}

fn is_ddl_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "CREATE" | "ALTER" | "DROP" | "TRUNCATE" | "RENAME" | "ANALYZE" | "OPTIMIZE" | "REPAIR"
    )
}

fn is_transaction_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "START" | "BEGIN" | "COMMIT" | "ROLLBACK" | "LOCK" | "UNLOCK"
    )
}

fn is_session_mutation_keyword(keyword: &str) -> bool {
    matches!(keyword, "SET" | "GRANT" | "REVOKE")
}

fn fingerprint_sql(normalized_sql: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized_sql.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::{assess_sql_safety, GuardDecision, SafeModeError, SafeModeGuard, SqlRiskReason};

    #[test]
    fn select_without_side_effects_is_safe() {
        let assessment = assess_sql_safety("SELECT * FROM users");
        assert!(assessment.is_safe_read_only());
        assert!(assessment.reasons.is_empty());
        assert_eq!(assessment.primary_keyword.as_deref(), Some("SELECT"));
    }

    #[test]
    fn destructive_statement_requires_confirmation_when_safe_mode_enabled() {
        let mut guard = SafeModeGuard::new(true);
        let decision = guard.evaluate("DELETE FROM users");

        match decision {
            GuardDecision::Allow { .. } => panic!("delete should not be auto-allowed"),
            GuardDecision::RequireConfirmation { assessment, .. } => {
                assert!(assessment
                    .reasons
                    .contains(&SqlRiskReason::WriteOperation("DELETE".to_string())));
            }
        }
    }

    #[test]
    fn dangerous_statement_is_allowed_when_safe_mode_disabled() {
        let mut guard = SafeModeGuard::new(false);
        let decision = guard.evaluate("DROP TABLE users");

        assert!(matches!(decision, GuardDecision::Allow { .. }));
    }

    #[test]
    fn multi_statement_sql_is_marked_risky() {
        let assessment = assess_sql_safety("SELECT 1; DELETE FROM users");
        assert!(assessment.reasons.contains(&SqlRiskReason::MultiStatement));
    }

    #[test]
    fn ignores_comments_when_classifying_sql() {
        let assessment = assess_sql_safety(
            r#"
            -- user lookup
            /* safe read */
            SELECT * FROM users;
            "#,
        );
        assert!(assessment.is_safe_read_only());
    }

    #[test]
    fn confirmation_requires_matching_sql_and_token_is_single_use() {
        let mut guard = SafeModeGuard::new(true);
        let decision = guard.evaluate("UPDATE users SET admin = 1");
        let token = match decision {
            GuardDecision::RequireConfirmation { token, .. } => token,
            GuardDecision::Allow { .. } => panic!("update should require confirmation"),
        };

        guard
            .confirm(&token, "UPDATE users SET admin = 1")
            .expect("matching sql should confirm");

        let err = guard
            .confirm(&token, "UPDATE users SET admin = 1")
            .expect_err("token should be single use");
        assert_eq!(err, SafeModeError::InvalidToken);
    }

    #[test]
    fn confirmation_fails_when_sql_does_not_match_token() {
        let mut guard = SafeModeGuard::new(true);
        let decision = guard.evaluate("DELETE FROM users WHERE id = 1");
        let token = match decision {
            GuardDecision::RequireConfirmation { token, .. } => token,
            GuardDecision::Allow { .. } => panic!("delete should require confirmation"),
        };

        let err = guard
            .confirm(&token, "DELETE FROM users WHERE id = 2")
            .expect_err("different statement should fail");
        assert_eq!(err, SafeModeError::SqlMismatch);
    }
}
