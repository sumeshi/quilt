use crate::controllers::log::LogController;
use polars::prelude::{col, lit, DataType, Expr};
use regex::escape as regex_escape;
use serde::Deserialize;
use sqlparser::ast::{
    BinaryOperator, Expr as SqlExpr, Query, Select, SetExpr, Statement, UnaryOperator,
    Value as SqlValue,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ZircRule {
    pub title: String,
    pub id: Option<String>,
    pub level: Option<String>,
    pub tags: Option<Vec<String>>,
    pub rule: Vec<String>,
}

pub fn load_rules(path: &Path) -> Result<Vec<ZircRule>, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read Sigma JSON rules {}: {e}", path.display()))?;
    let mut rules: Vec<ZircRule> = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse Sigma JSON rules {}: {e}", path.display()))?;
    rules.retain(|rule| !rule.rule.is_empty());
    Ok(rules)
}

pub fn sql_to_polars_expr(
    sql: &str,
    available_cols: &[String],
    field_map: Option<&HashMap<String, String>>,
) -> Option<Expr> {
    let dialect = GenericDialect {};
    let statements = match Parser::parse_sql(&dialect, sql) {
        Ok(statements) => statements,
        Err(error) => {
            LogController::warn(&format!("Failed to parse Sigma SQL '{sql}': {error}"));
            return None;
        }
    };

    let where_expr = match statements.first() {
        Some(Statement::Query(query)) => extract_where_clause(query),
        Some(_) => {
            LogController::warn("Unsupported Sigma SQL statement. Expected SELECT.");
            None
        }
        None => {
            LogController::warn("Sigma SQL rule produced no statements.");
            None
        }
    }?;

    let expr = where_to_expr(where_expr, available_cols, field_map);
    if expr.is_none() {
        LogController::debug(&format!(
            "Skipping Sigma SQL rule due to unsupported syntax or unresolved fields: {}",
            abbreviate_sql(sql)
        ));
    }
    expr
}

pub fn referenced_fields(sql: &str) -> Vec<String> {
    let dialect = GenericDialect {};
    let statements = match Parser::parse_sql(&dialect, sql) {
        Ok(statements) => statements,
        Err(error) => {
            LogController::warn(&format!(
                "Failed to parse Sigma SQL while collecting fields '{sql}': {error}"
            ));
            return Vec::new();
        }
    };

    let Some(Statement::Query(query)) = statements.first() else {
        return Vec::new();
    };
    let Some(where_expr) = extract_where_clause(query) else {
        return Vec::new();
    };

    let mut fields = BTreeSet::new();
    collect_expr_identifiers(where_expr, &mut fields);
    fields.into_iter().collect()
}

pub fn where_to_expr(
    ast: &SqlExpr,
    available_cols: &[String],
    field_map: Option<&HashMap<String, String>>,
) -> Option<Expr> {
    match ast {
        SqlExpr::Nested(expr) => where_to_expr(expr, available_cols, field_map),
        SqlExpr::UnaryOp {
            op: UnaryOperator::Not,
            expr,
        } => where_to_expr(expr, available_cols, field_map).map(|expr| expr.not()),
        SqlExpr::BinaryOp { left, op, right } => match op {
            BinaryOperator::Eq => {
                let column = resolve_operand_column(left, available_cols, field_map)?;
                let value = sql_expr_to_string_literal(right)?;
                Some(string_col(&column).eq(lit(value)))
            }
            BinaryOperator::And => combine_optional_exprs(
                where_to_expr(left, available_cols, field_map),
                where_to_expr(right, available_cols, field_map),
                BoolOp::And,
            ),
            BinaryOperator::Or => combine_optional_exprs(
                where_to_expr(left, available_cols, field_map),
                where_to_expr(right, available_cols, field_map),
                BoolOp::Or,
            ),
            _ => None,
        },
        SqlExpr::Like {
            negated,
            expr,
            pattern,
            escape_char,
            ..
        } => {
            let column = resolve_operand_column(expr, available_cols, field_map)?;
            let pattern = sql_expr_to_string_literal(pattern)?;
            let regex = like_pattern_to_regex(&pattern, escape_char.as_deref());
            let expr = string_col(&column).str().contains(lit(regex), false);
            Some(if *negated { expr.not() } else { expr })
        }
        _ => None,
    }
}

fn extract_where_clause(query: &Query) -> Option<&SqlExpr> {
    match query.body.as_ref() {
        SetExpr::Select(select) => extract_select_where_clause(select),
        _ => None,
    }
}

fn extract_select_where_clause(select: &Select) -> Option<&SqlExpr> {
    select.selection.as_ref()
}

fn resolve_operand_column(
    expr: &SqlExpr,
    available_cols: &[String],
    field_map: Option<&HashMap<String, String>>,
) -> Option<String> {
    let identifier = extract_identifier(expr)?;
    resolve_column_name(&identifier, available_cols, field_map)
}

fn collect_expr_identifiers(expr: &SqlExpr, fields: &mut BTreeSet<String>) {
    match expr {
        SqlExpr::Nested(expr) => collect_expr_identifiers(expr, fields),
        SqlExpr::UnaryOp { expr, .. } => collect_expr_identifiers(expr, fields),
        SqlExpr::BinaryOp { left, right, .. } => {
            if let Some(identifier) = extract_identifier_quiet(left) {
                fields.insert(identifier);
            }
            if let Some(identifier) = extract_identifier_quiet(right) {
                fields.insert(identifier);
            }
            collect_expr_identifiers(left, fields);
            collect_expr_identifiers(right, fields);
        }
        SqlExpr::Like { expr, pattern, .. } => {
            if let Some(identifier) = extract_identifier_quiet(expr) {
                fields.insert(identifier);
            }
            collect_expr_identifiers(expr, fields);
            collect_expr_identifiers(pattern, fields);
        }
        _ => {}
    }
}

fn extract_identifier(expr: &SqlExpr) -> Option<String> {
    extract_identifier_quiet(expr)
}

fn extract_identifier_quiet(expr: &SqlExpr) -> Option<String> {
    match expr {
        SqlExpr::Identifier(ident) => Some(ident.value.clone()),
        SqlExpr::CompoundIdentifier(parts) => parts.last().map(|ident| ident.value.clone()),
        _ => None,
    }
}

fn resolve_column_name(
    identifier: &str,
    available_cols: &[String],
    field_map: Option<&HashMap<String, String>>,
) -> Option<String> {
    if let Some(mapped) = field_map.and_then(|mapping| mapping.get(identifier)) {
        if available_cols.iter().any(|column| column == mapped) {
            return Some(mapped.clone());
        }
        return None;
    }

    if let Some(column) = available_cols.iter().find(|column| *column == identifier) {
        return Some(column.clone());
    }

    if let Some(column) = available_cols
        .iter()
        .find(|column| column.eq_ignore_ascii_case(identifier))
    {
        return Some(column.clone());
    }

    None
}

fn sql_expr_to_string_literal(expr: &SqlExpr) -> Option<String> {
    match expr {
        SqlExpr::Value(value) => sql_value_to_string(value),
        SqlExpr::Nested(expr) => sql_expr_to_string_literal(expr),
        _ => None,
    }
}

fn sql_value_to_string(value: &SqlValue) -> Option<String> {
    match value {
        SqlValue::SingleQuotedString(value)
        | SqlValue::DoubleQuotedString(value)
        | SqlValue::TripleSingleQuotedString(value)
        | SqlValue::TripleDoubleQuotedString(value)
        | SqlValue::SingleQuotedByteStringLiteral(value)
        | SqlValue::DoubleQuotedByteStringLiteral(value)
        | SqlValue::SingleQuotedRawStringLiteral(value)
        | SqlValue::DoubleQuotedRawStringLiteral(value)
        | SqlValue::NationalStringLiteral(value)
        | SqlValue::EscapedStringLiteral(value)
        | SqlValue::UnicodeStringLiteral(value) => Some(value.clone()),
        SqlValue::Number(value, _) => Some(value.clone()),
        _ => None,
    }
}

fn abbreviate_sql(sql: &str) -> String {
    const MAX_LEN: usize = 160;
    let normalized = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.len() <= MAX_LEN {
        normalized
    } else {
        format!("{}...", &normalized[..MAX_LEN])
    }
}

fn like_pattern_to_regex(pattern: &str, escape_char: Option<&str>) -> String {
    let escape_char = escape_char
        .and_then(|value| value.chars().next())
        .unwrap_or('\\');
    let mut regex = String::from("(?i)^");
    let mut escaped = false;

    for ch in pattern.chars() {
        if escaped {
            regex.push_str(&regex_escape(&ch.to_string()));
            escaped = false;
            continue;
        }

        if ch == escape_char {
            escaped = true;
            continue;
        }

        match ch {
            '%' => regex.push_str(".*"),
            '_' => regex.push('.'),
            other => regex.push_str(&regex_escape(&other.to_string())),
        }
    }

    if escaped {
        regex.push_str(&regex_escape(&escape_char.to_string()));
    }

    regex.push('$');
    regex
}

fn string_col(column_name: &str) -> Expr {
    col(column_name).cast(DataType::String)
}

enum BoolOp {
    And,
    Or,
}

fn combine_optional_exprs(left: Option<Expr>, right: Option<Expr>, op: BoolOp) -> Option<Expr> {
    match op {
        BoolOp::And => match (left, right) {
            (Some(left), Some(right)) => Some(left.and(right)),
            _ => None,
        },
        BoolOp::Or => match (left, right) {
            (Some(left), Some(right)) => Some(left.or(right)),
            (Some(expr), None) | (None, Some(expr)) => Some(expr),
            (None, None) => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polars::prelude::{DataFrame, IntoLazy, NamedFrom, Series};

    fn filter_values(df: DataFrame, expr: Expr, column: &str) -> Vec<String> {
        let filtered = df
            .lazy()
            .filter(expr)
            .collect()
            .expect("filter should succeed");
        filtered
            .column(column)
            .expect("column should exist")
            .str()
            .expect("column should be string")
            .into_no_null_iter()
            .map(|value| value.to_string())
            .collect()
    }

    #[test]
    fn test_like_to_regex_contains() {
        let df = DataFrame::new(vec![Series::new(
            "CommandLine".into(),
            ["abc foo xyz", "bar"],
        )
        .into()])
        .expect("dataframe");
        let available_cols = vec!["CommandLine".to_string()];
        let expr = sql_to_polars_expr(
            "SELECT * FROM logs WHERE CommandLine LIKE '%foo%' ESCAPE '\\'",
            &available_cols,
            None,
        )
        .expect("expression");

        let values = filter_values(df, expr, "CommandLine");
        assert_eq!(values, vec!["abc foo xyz"]);
    }

    #[test]
    fn test_like_escape_backslash() {
        let df = DataFrame::new(vec![Series::new(
            "TargetObject".into(),
            [r"C:\windows\system32", r"C:\temp\system32"],
        )
        .into()])
        .expect("dataframe");
        let available_cols = vec!["TargetObject".to_string()];
        let expr = sql_to_polars_expr(
            r"SELECT * FROM logs WHERE TargetObject LIKE '%\\windows\\%' ESCAPE '\'",
            &available_cols,
            None,
        )
        .expect("expression");

        let values = filter_values(df, expr, "TargetObject");
        assert_eq!(values, vec![r"C:\windows\system32"]);
    }

    #[test]
    fn test_not_expr() {
        let df = DataFrame::new(vec![Series::new("value".into(), ["x", "y"]).into()])
            .expect("dataframe");
        let available_cols = vec!["value".to_string()];
        let expr = sql_to_polars_expr(
            "SELECT * FROM logs WHERE NOT (value = 'x')",
            &available_cols,
            None,
        )
        .expect("expression");

        let values = filter_values(df, expr, "value");
        assert_eq!(values, vec!["y"]);
    }

    #[test]
    fn test_or_expr() {
        let df = DataFrame::new(vec![Series::new("value".into(), ["a", "b", "c"]).into()])
            .expect("dataframe");
        let available_cols = vec!["value".to_string()];
        let expr = sql_to_polars_expr(
            "SELECT * FROM logs WHERE value = 'a' OR value = 'b'",
            &available_cols,
            None,
        )
        .expect("expression");

        let values = filter_values(df, expr, "value");
        assert_eq!(values, vec!["a", "b"]);
    }
}
