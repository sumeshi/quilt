/// Escape characters that can alter terminal layout while preserving readable Unicode.
pub(super) fn escape_controls(value: &str) -> String {
    escaped_tokens(value).collect()
}

pub(super) fn escaped_tokens(value: &str) -> impl Iterator<Item = String> + '_ {
    value.chars().map(|character| match character {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            c if c.is_control()
                || matches!(c, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}') =>
            {
                c.escape_unicode().to_string()
            }
            c => c.to_string(),
        })
}
