//! Helpers for constructing low-cardinality OpenTelemetry span names for
//! database operations.

/// Constructs a low-cardinality span name from an arbitrary SQL statement,
/// following the OpenTelemetry database span name conventions.
///
/// The full text of a SQL statement has very high cardinality (it embeds
/// literals, parameter values, etc.) and so makes a poor span name. Instead the
/// conventions recommend a `{operation} {target}` summary, e.g. `SELECT users`,
/// falling back to just `{operation}` when no single target can be determined.
///
/// See the OpenTelemetry guidance on [database span names] and [generating a
/// query summary]. This implementation is a lightweight, dependency-free
/// approximation modeled on the [opentelemetry-go-instrumentation SQL probe].
///
/// If no operation can be extracted (e.g. the statement is empty) this returns
/// `"SQL"` so that the span always has a stable, low-cardinality name.
///
/// [database span names]: https://opentelemetry.io/docs/specs/semconv/db/database-spans/#name
/// [generating a query summary]: https://opentelemetry.io/docs/specs/semconv/db/database-spans/#generating-a-summary-of-the-query
/// [opentelemetry-go-instrumentation SQL probe]: https://github.com/open-telemetry/opentelemetry-go-instrumentation/blob/main/internal/pkg/instrumentation/bpf/database/sql/probe.go
pub fn sql_span_name(statement: &str) -> String {
    let mut tokens = SqlTokens::new(statement);

    let Some(operation) = tokens.next() else {
        return "SQL".to_string();
    };
    // SQL keywords are case-insensitive; normalize the operation to reduce
    // cardinality (e.g. `select` and `SELECT` should map to the same name).
    let operation = operation.to_ascii_uppercase();

    let target = match operation.as_str() {
        "SELECT" | "DELETE" => tokens.target_after("FROM"),
        "INSERT" | "REPLACE" => tokens.target_after("INTO"),
        "UPDATE" => tokens.next_target(),
        // For other statements (DDL, transaction control, etc.) the operation
        // alone is a sufficiently descriptive, low-cardinality summary.
        _ => None,
    };

    match target {
        Some(target) => format!("{operation} {target}"),
        None => operation,
    }
}

/// A minimal tokenizer over a SQL statement.
///
/// It yields whitespace-delimited "words", treating `(`, `)`, `,` and `;` as
/// their own single-character tokens so that constructs like `FROM(SELECT ...`
/// and `FROM a, b` are split correctly.
struct SqlTokens<'a> {
    rest: &'a str,
}

impl<'a> SqlTokens<'a> {
    fn new(statement: &'a str) -> Self {
        Self { rest: statement }
    }

    /// Returns the next token following the first occurrence of `keyword`
    /// (matched case-insensitively), cleaned up for use as a span target.
    ///
    /// Returns `None` if the keyword is not found, or if the entity following it
    /// is an anonymous/derived table (e.g. a subquery in parentheses), in which
    /// case the OpenTelemetry conventions recommend omitting the target.
    fn target_after(&mut self, keyword: &str) -> Option<String> {
        while let Some(token) = self.next() {
            if token.eq_ignore_ascii_case(keyword) {
                return self.next_target();
            }
        }
        None
    }

    /// Returns the next token cleaned up for use as a span target, or `None` if
    /// there is no suitable target (e.g. a subquery or an empty token).
    fn next_target(&mut self) -> Option<String> {
        let token = self.next()?;
        // A `(` indicates an anonymous/derived table; omit the target.
        if token == "(" {
            return None;
        }
        let cleaned = token.trim_matches(|c| matches!(c, '"' | '`' | '\'' | '[' | ']'));
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned.to_string())
        }
    }
}

impl<'a> Iterator for SqlTokens<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let rest = std::mem::take(&mut self.rest).trim_start();
        if rest.is_empty() {
            return None;
        }
        // Single ascii char tokens
        let token_end = if rest.starts_with(['(', ')', ',', ';']) {
            Some(1)
        } else {
            rest.find(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';'))
        }
        .unwrap_or(rest.len());

        let (token, rest) = rest.split_at(token_end);
        self.rest = rest;
        Some(token)
    }
}

#[cfg(test)]
mod tests {
    use super::sql_span_name;

    #[test]
    fn select_with_table() {
        assert_eq!(
            sql_span_name("SELECT * FROM users WHERE id = ?"),
            "SELECT users"
        );
    }

    #[test]
    fn select_is_case_insensitive() {
        assert_eq!(sql_span_name("select id from Users"), "SELECT Users");
    }

    #[test]
    fn select_multiline() {
        let query = "SELECT *\n  FROM   wuser_table\n WHERE username = ?";
        assert_eq!(sql_span_name(query), "SELECT wuser_table");
    }

    #[test]
    fn select_multiple_tables_uses_first() {
        assert_eq!(
            sql_span_name("SELECT * FROM songs, artists"),
            "SELECT songs"
        );
    }

    #[test]
    fn select_anonymous_table_has_no_target() {
        assert_eq!(
            sql_span_name("SELECT * FROM (SELECT * FROM orders) t"),
            "SELECT"
        );
    }

    #[test]
    fn insert_into_table() {
        assert_eq!(
            sql_span_name("INSERT INTO shipping_details (order_id) VALUES (?)"),
            "INSERT shipping_details"
        );
    }

    #[test]
    fn update_table() {
        assert_eq!(
            sql_span_name("UPDATE users SET name = ? WHERE id = ?"),
            "UPDATE users"
        );
    }

    #[test]
    fn delete_from_table() {
        assert_eq!(
            sql_span_name("DELETE FROM users WHERE id = ?"),
            "DELETE users"
        );
    }

    #[test]
    fn quoted_identifiers_are_cleaned() {
        assert_eq!(sql_span_name("SELECT * FROM `my_table`"), "SELECT my_table");
        assert_eq!(
            sql_span_name("SELECT * FROM \"my_table\""),
            "SELECT my_table"
        );
    }

    #[test]
    fn schema_qualified_target_preserved() {
        assert_eq!(
            sql_span_name("SELECT * FROM public.users"),
            "SELECT public.users"
        );
    }

    #[test]
    fn ddl_uses_operation_only() {
        assert_eq!(
            sql_span_name("CREATE TABLE users (id INTEGER PRIMARY KEY)"),
            "CREATE"
        );
    }

    #[test]
    fn empty_statement_falls_back() {
        assert_eq!(sql_span_name("   "), "SQL");
    }
}
