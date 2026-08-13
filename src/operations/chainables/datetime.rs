use chrono::NaiveDateTime;
use dtparse::parse as dtparse_parse;
use once_cell::sync::Lazy;
use regex::Regex;

static FUZZY_DATETIME_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        r"(?i)(?:on\s+)?(?:january|february|march|april|may|june|july|august|september|october|november|december)\s+\d{1,2}(?:st|nd|rd|th)?,?\s+\d{4}(?:\s+at\s+)?\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
        r"(?i)(?:on\s+)?(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\s+\d{1,2},?\s+\d{4}\s+\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
        r"\d{4}-\d{1,2}-\d{1,2}\s+\d{1,2}:\d{2}(?::\d{2})?",
        r"\d{1,2}/\d{1,2}/\d{4}\s+\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
        r"(?i)(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday)\s+(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\s+\d{1,2}\s+\d{4}\s+\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("valid datetime pattern"))
    .collect()
});

/// Parse the datetime forms accepted by the existing datetime operations.
/// The returned value is timezone-naive; timezone-aware input is normalized to
/// its local wall-clock representation by the parser, matching existing qsv behavior.
pub fn parse_datetime_auto(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if let Ok((datetime, _)) = dtparse_parse(value) {
        return Some(datetime);
    }

    if let Some(found) = FUZZY_DATETIME_PATTERNS.iter().find_map(|pattern| {
        pattern
            .find(value)
            .and_then(|match_| parse_extracted(match_.as_str()))
    }) {
        return Some(found);
    }

    let formats = [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
        "%m/%d/%Y %H:%M:%S%.f",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y",
        "%Y/%m/%d %H:%M:%S%.f",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d",
        "%d/%b/%Y:%H:%M:%S",
        "%d %b %Y %H:%M:%S",
        "%b %d %Y %H:%M:%S",
        "%d %B %Y %H:%M:%S",
        "%B %d %Y %H:%M:%S",
        "%d-%b-%Y %H:%M:%S",
        "%d %b %Y",
        "%b %d %Y",
        "%a %b %d %H:%M:%S %Y",
        "%a, %d %b %Y %H:%M:%S",
        "%s",
        "%s%.f",
        "%m/%d/%Y %I:%M:%S %p",
        "%Y-%m-%d %I:%M:%S %p",
    ];
    formats
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
}

fn parse_extracted(value: &str) -> Option<NaiveDateTime> {
    let cleaned = value
        .replace(" at ", " ")
        .replace("st,", ",")
        .replace("nd,", ",")
        .replace("rd,", ",")
        .replace("th,", ",")
        .replace("st ", " ")
        .replace("nd ", " ")
        .replace("rd ", " ")
        .replace("th ", " ");
    if let Ok((datetime, _)) = dtparse_parse(&cleaned) {
        return Some(datetime);
    }
    [
        "%B %d, %Y %I:%M:%S %p",
        "%B %d, %Y %I:%M %p",
        "%b %d, %Y %I:%M:%S %p",
        "%b %d, %Y %I:%M %p",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%m/%d/%Y %I:%M:%S %p",
        "%m/%d/%Y %I:%M %p",
        "%A %b %d %Y %I:%M:%S %p",
        "%A %b %d %Y %I:%M %p",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(&cleaned, format).ok())
}
