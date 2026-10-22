use crate::controllers::command_model::{
    parse_typed_commands, registry, render_command_help, CommandCategory, TypedCommand,
};
use crate::QuiltError;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Completion {
    pub display: String,
    pub replacement: String,
    pub range: Range<usize>,
}

pub(super) struct SelectTarget {
    pub range: Range<usize>,
    pub columns: Vec<String>,
}

#[derive(Clone, Debug)]
struct TokenSpan {
    value: String,
    start: usize,
    end: usize,
    // Each decoded byte maps to the raw source range that produced it.
    source: Vec<Range<usize>>,
    has_shell_quotes: bool,
}

fn token_spans(input: &str) -> Vec<TokenSpan> {
    lex(input).0
}

fn lex(input: &str) -> (Vec<TokenSpan>, bool) {
    let mut out = Vec::new();
    let mut value = String::new();
    let mut source = Vec::new();
    let (mut quote, mut started) = (None, false);
    let mut has_shell_quotes = false;
    let mut start = 0;
    let mut chars = input.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(q)
                if c == '\\' && matches!(chars.peek(), Some((_, n)) if *n == q || *n == '\\') =>
            {
                if let Some((j, next)) = chars.next() {
                    value.push(next);
                    source.extend(std::iter::repeat_n(i..j + next.len_utf8(), next.len_utf8()));
                }
            }
            Some(_) => {
                value.push(c);
                source.extend(std::iter::repeat_n(i..i + c.len_utf8(), c.len_utf8()));
            }
            None if c == '\'' || c == '"' => {
                if !started {
                    start = i;
                }
                quote = Some(c);
                started = true;
                has_shell_quotes = true;
            }
            None if c.is_whitespace() => {
                if started {
                    out.push(TokenSpan {
                        value: std::mem::take(&mut value),
                        start,
                        end: i,
                        source: std::mem::take(&mut source),
                        has_shell_quotes,
                    });
                    started = false;
                    has_shell_quotes = false;
                }
            }
            None => {
                if !started {
                    start = i;
                }
                value.push(c);
                source.extend(std::iter::repeat_n(i..i + c.len_utf8(), c.len_utf8()));
                started = true;
            }
        }
    }
    let incomplete_quote = quote.is_some();
    if started {
        out.push(TokenSpan {
            value,
            start,
            end: input.len(),
            source,
            has_shell_quotes,
        });
    }
    (out, incomplete_quote)
}

fn pipeline_separators(tokens: &[TokenSpan]) -> Vec<usize> {
    let mut separators = Vec::new();
    let mut command: Option<&str> = None;
    let mut positional_only = false;
    let mut positional_count = 0usize;
    let mut skip_next = false;
    for (i, token) in tokens.iter().enumerate() {
        let word = token.value.as_str();
        if skip_next {
            skip_next = false;
            continue;
        }
        let spec = command.and_then(|name| registry().iter().find(|s| s.name == name));
        let after_max = positional_only
            && word == "-"
            && spec.is_some_and(|s| s.max_args.is_some_and(|n| positional_count >= n));
        if word == "-" && (!positional_only || after_max) {
            separators.push(i);
            command = None;
            positional_only = false;
            positional_count = 0;
            continue;
        }
        if command.is_none() {
            command = Some(word);
            continue;
        }
        if word == "--" && !positional_only {
            positional_only = true;
            continue;
        }
        if !positional_only {
            if let Some(spec) = spec {
                let flag = word
                    .strip_prefix("--")
                    .or_else(|| word.strip_prefix('-').filter(|s| !s.is_empty()));
                if let Some(flag) = flag {
                    let (key, inline) = if let Some((k, _)) = flag.split_once('=') {
                        (k, true)
                    } else if !word.starts_with("--") && flag.len() > 1 {
                        let short_len = flag.chars().next().map(char::len_utf8).unwrap_or(0);
                        let short = if short_len < flag.len() {
                            &flag[..short_len]
                        } else {
                            flag
                        };
                        (
                            if spec
                                .options
                                .iter()
                                .any(|o| o.short == Some(short) && o.takes_value)
                            {
                                short
                            } else {
                                flag
                            },
                            spec.options
                                .iter()
                                .any(|o| o.short == Some(short) && o.takes_value),
                        )
                    } else {
                        (flag, false)
                    };
                    if !inline
                        && spec
                            .options
                            .iter()
                            .any(|o| (o.name == key || o.short == Some(key)) && o.takes_value)
                    {
                        skip_next = true;
                        continue;
                    }
                    continue;
                }
            }
        }
        positional_count += 1;
    }
    separators
}

fn active_select(tokens: &[TokenSpan], cursor: usize) -> Option<(usize, usize)> {
    let separators = pipeline_separators(tokens);
    let mut first = 0;
    let mut last = tokens.len();
    for sep in separators {
        if tokens[sep].start < cursor {
            first = sep + 1;
        } else {
            last = sep;
            break;
        }
    }
    let command = tokens[first..last].first()?;
    (command.value == "select").then_some((first, last))
}

fn csv_columns_lenient(value: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if quoted && bytes.get(i + 1) == Some(&b'"') {
                i += 2;
                continue;
            }
            quoted = !quoted;
        } else if bytes[i] == b',' && !quoted {
            fields.push(clean_csv_field(&value[start..i]));
            start = i + 1;
        }
        i += 1;
    }
    if start < value.len() {
        fields.push(clean_csv_field(&value[start..]));
    }
    fields
}

fn clean_csv_field(field: &str) -> String {
    let field = field.trim();
    if field.starts_with('"') && field.ends_with('"') && field.len() >= 2 {
        field[1..field.len() - 1].replace("\"\"", "\"")
    } else {
        field.to_owned()
    }
}

pub(super) fn select_target(input: &str, cursor: usize) -> Option<SelectTarget> {
    if cursor > input.len() || !input.is_char_boundary(cursor) {
        return None;
    }
    if input.trim().is_empty() {
        return Some(SelectTarget {
            range: cursor..cursor,
            columns: Vec::new(),
        });
    }
    let tokens = token_spans(input);
    let (first, last) = active_select(&tokens, cursor)?;
    let start = tokens[first].start;
    let segment_end = if last < tokens.len() {
        tokens[last].start
    } else {
        input.len()
    };
    let end = start + input[start..segment_end].trim_end().len();
    let args = &tokens[first + 1..last];
    let joined = args
        .iter()
        .filter(|t| t.value != "--")
        .map(|t| t.value.as_str())
        .collect::<Vec<_>>()
        .join(",");
    Some(SelectTarget {
        range: start..end.max(start),
        columns: csv_columns_lenient(&joined),
    })
}

pub(super) fn format_select(columns: &[String]) -> String {
    let csv = columns
        .iter()
        .map(|name| csv_field(name))
        .collect::<Vec<_>>()
        .join(",");
    if csv
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\''))
    {
        format!("select {}", quote_argument(&csv))
    } else {
        format!("select {csv}")
    }
}

fn csv_field(name: &str) -> String {
    if name.starts_with('-') || name.contains(',') || name.contains('"') || name.trim() != name {
        format!("\"{}\"", name.replace('"', "\"\""))
    } else {
        name.to_owned()
    }
}

pub(super) fn parse_commands(input: &str) -> Result<Vec<TypedCommand>, QuiltError> {
    let argv = tokenize(input)?;
    if argv.is_empty() {
        return Ok(Vec::new());
    }
    parse_typed_commands(&argv)
}

pub(super) fn preview_prefix(input: &str) -> Vec<TypedCommand> {
    let tokens = token_spans(input);
    let Some(separator) = pipeline_separators(&tokens).last().copied() else {
        return Vec::new();
    };
    let argv = tokens[..separator]
        .iter()
        .map(|t| t.value.clone())
        .collect::<Vec<_>>();
    parse_typed_commands(&argv).unwrap_or_default()
}

pub(super) fn words(input: &str) -> Result<Vec<String>, QuiltError> {
    tokenize(input)
}

fn tokenize(input: &str) -> Result<Vec<String>, QuiltError> {
    let (tokens, incomplete_quote) = lex(input);
    if incomplete_quote {
        return Err(QuiltError::usage("Error: Incomplete quoted argument"));
    }
    Ok(tokens.into_iter().map(|token| token.value).collect())
}

pub(super) fn quote_argument(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(super) fn complete(
    input: &str,
    cursor: usize,
    columns: &[(String, String)],
) -> Vec<Completion> {
    if cursor > input.len() || !input.is_char_boundary(cursor) {
        return vec![];
    }
    let spans = token_spans(input);
    if let Some((first, last)) = active_select(&spans, cursor) {
        if let Some(completion) = complete_select_field(cursor, columns, &spans[first + 1..last]) {
            return completion;
        }
    }
    let left = &input[..cursor];
    let start = left
        .rfind(char::is_whitespace)
        .map_or(0, |i| i + left[i..].chars().next().unwrap().len_utf8());
    let prefix = &left[start..];
    let tokens = tokenize(left)
        .or_else(|_| tokenize(&format!("{left}\"")))
        .or_else(|_| tokenize(&format!("{left}'")))
        .unwrap_or_default();
    let trailing_space = left.chars().last().is_some_and(char::is_whitespace);
    let current_word_complete = trailing_space;
    let mut segment_start = tokens.iter().rposition(|t| t == "-").map_or(0, |i| i + 1);
    if !current_word_complete {
        segment_start = segment_start.min(tokens.len());
    }
    let segment = &tokens[segment_start..];
    let has_partial = !trailing_space && !tokens.is_empty();
    let at_command = segment.is_empty() || (has_partial && segment.len() == 1);
    let mut candidates: Vec<(String, String)> = Vec::new();
    if at_command {
        candidates.extend(
            registry()
                .iter()
                .filter(|s| {
                    matches!(
                        s.category,
                        CommandCategory::Initializer | CommandCategory::Chainable
                    )
                })
                .map(|s| (s.name.to_owned(), s.name.to_owned())),
        );
    } else {
        let command_tokens = if trailing_space {
            segment
        } else {
            &segment[..segment.len().saturating_sub(1)]
        };
        let command = command_tokens
            .first()
            .and_then(|t| registry().iter().find(|s| s.name == t.as_str()));
        if let Some(spec) = command {
            candidates.extend(
                spec.options
                    .iter()
                    .map(|o| (format!("--{}", o.name), format!("--{}", o.name))),
            );
            let prior = segment.last().map(String::as_str).unwrap_or("");
            let arg_index = segment.len().saturating_sub(2);
            let want_columns = trailing_space || (!prior.starts_with('-') && !prior.is_empty());
            if want_columns && !looks_like_path_slot(spec.name, arg_index) {
                candidates.extend(
                    columns
                        .iter()
                        .map(|(name, _)| (name.clone(), quote_column(name, spec.name))),
                );
            }
        }
    }
    let match_prefix = prefix.trim_start_matches(['"', '\'']);
    candidates
        .into_iter()
        .filter(|(display, replacement)| {
            replacement.starts_with(prefix) || display.starts_with(match_prefix)
        })
        .map(|(display, replacement)| Completion {
            display,
            replacement,
            range: start..cursor,
        })
        .collect()
}

fn complete_select_field(
    cursor: usize,
    columns: &[(String, String)],
    args: &[TokenSpan],
) -> Option<Vec<Completion>> {
    let token = args.iter().find(|t| t.start <= cursor && cursor <= t.end)?;
    let decoded_cursor = token
        .source
        .iter()
        .take_while(|source| source.start < cursor)
        .count();
    let bytes = token.value.as_bytes();
    let mut quoted = false;
    let mut field_start = 0;
    let mut field_end = bytes.len();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'"' {
            if quoted && bytes.get(at + 1) == Some(&b'"') {
                at += 2;
                continue;
            }
            quoted = !quoted;
        } else if bytes[at] == b',' && !quoted {
            if decoded_cursor <= at {
                field_end = at;
                break;
            }
            field_start = at + 1;
        }
        at += 1;
    }
    let field_end_decoded = field_end;
    let field_value = token.value.get(field_start..field_end_decoded)?;
    let leading = field_value.len() - field_value.trim_start().len();
    let trailing = field_value.len() - field_value.trim_end().len();
    let lo = field_start + leading;
    let hi = field_end_decoded.saturating_sub(trailing);
    let raw_start = token
        .source
        .get(lo)
        .map(|source| source.start)
        .unwrap_or_else(|| if lo == 0 { token.start } else { token.end });
    let raw_end = if hi > lo {
        token
            .source
            .get(hi - 1)
            .map_or(raw_start, |source| source.end)
    } else {
        raw_start
    };
    let prefix_end = decoded_cursor.clamp(field_start, field_end_decoded);
    let typed_prefix = token.value.get(field_start..prefix_end)?.trim();
    let prefix = if typed_prefix.starts_with('"') {
        typed_prefix.trim_start_matches('"')
    } else {
        typed_prefix
    };
    Some(
        columns
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, _)| {
                let (replacement, range) = if token.has_shell_quotes {
                    let field = csv_field(name);
                    let value = format!(
                        "{}{}{}",
                        &token.value[..field_start],
                        field,
                        &token.value[field_end_decoded..]
                    );
                    (quote_argument(&value), token.start..token.end)
                } else {
                    (quote_column(name, "select"), raw_start..raw_end)
                };
                Completion {
                    display: name.clone(),
                    replacement,
                    range,
                }
            })
            .collect(),
    )
}

fn looks_like_path_slot(command: &str, arg_index: usize) -> bool {
    matches!(command, "load" | "dump" | "dumpcache" | "partition" | "run") && arg_index == 0
}

fn quote_column(name: &str, command: &str) -> String {
    if command == "select" {
        let csv = csv_field(name);
        if csv != name || name.chars().any(|c| c.is_whitespace() || c == '\'') {
            quote_argument(&csv)
        } else {
            name.to_owned()
        }
    } else if name
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, ',' | '.' | ':' | '"' | '\''))
    {
        quote_argument(name)
    } else {
        name.to_owned()
    }
}

pub(super) fn help(input: &str, cursor: usize) -> String {
    let trimmed = input.get(..cursor.min(input.len())).unwrap_or(input).trim();
    if let Some(rest) = trimmed.strip_prefix(':') {
        let name = rest.split_whitespace().next().unwrap_or("");
        return match name {
            "write" => ":write PATH  Write the current pipeline as YAML".into(),
            "cache" => {
                ":cache [PATH]  Save a session checkpoint, or persistent Parquet cache".into()
            }
            "clear-cache" => ":clear-cache  Clear cached pipeline data".into(),
            "undo" => ":undo  Undo the last pipeline edit".into(),
            "redo" => ":redo  Redo the last undone edit".into(),
            "edit" => ":edit [STEP]  Edit a pipeline step (1-based)".into(),
            "quit" => ":quit  Leave the TUI".into(),
            _ => {
                ":write PATH, :cache [PATH], :clear-cache, :undo, :redo, :edit [STEP], :quit".into()
            }
        };
    }
    let tokens = tokenize(trimmed).unwrap_or_default();
    let start = tokens.iter().rposition(|t| t == "-").map_or(0, |i| i + 1);
    let command = tokens
        .get(start)
        .filter(|t| registry().iter().any(|s| s.name == t.as_str()))
        .cloned();
    command
        .as_deref()
        .and_then(render_command_help)
        .unwrap_or_else(crate::controllers::command_model::render_registry_help)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_arguments_and_paths() {
        let args =
            tokenize(r#"load "C:\Users\me\x.csv" - filter "name == 'a b' and `a,b` != \"x\"""#)
                .unwrap();
        assert_eq!(args[1], r"C:\Users\me\x.csv");
        assert!(args[4].contains("name == 'a b'"));
        assert_eq!(tokenize("select ''").unwrap()[1], "");
        assert!(tokenize("filter \"é").is_err());
        for value in [
            r"\\server\share\log.csv",
            r"literal\\backslashes",
            "both 'single' and \"double\" quotes",
        ] {
            assert_eq!(tokenize(&quote_argument(value)).unwrap(), [value]);
        }
    }

    #[test]
    fn completion_handles_unicode_and_quoted_column_names() {
        let cols = vec![
            ("名字".to_string(), "String".to_string()),
            ("a,b c".to_string(), "String".to_string()),
        ];
        let input = "select 名";
        let found = complete(input, input.len(), &cols);
        assert_eq!(found[0].range, 7..10);
        assert_eq!(found[0].replacement, "名字");
        let found = complete("select \"a", 9, &cols);
        let comma = found.iter().find(|c| c.display == "a,b c").unwrap();
        assert_eq!(comma.replacement, "\"\\\"a,b c\\\"\"");
        assert!(parse_commands(&format!("select {}", comma.replacement)).is_ok());
    }

    #[test]
    fn column_completion_round_trips_special_names_by_command() {
        let names = ["a,b'c", "a,\"b", "only\"quote", "a\\b\\c"];
        let columns = names
            .iter()
            .map(|s| (s.to_string(), "String".to_string()))
            .collect::<Vec<_>>();
        for (name, _) in &columns {
            let result = complete("select ", 7, &columns)
                .into_iter()
                .find(|c| &c.display == name)
                .unwrap();
            let commands = parse_commands(&format!("select {}", result.replacement)).unwrap();
            assert!(
                matches!(&commands[0], TypedCommand::Select(args) if args.columns == [name.clone()])
            );
        }

        let name = "a,b'c";
        let columns = [(name.to_string(), "String".to_string())];
        let result = complete("cast ", 5, &columns)
            .into_iter()
            .find(|c| c.display == name)
            .unwrap();
        let commands = parse_commands(&format!("cast {} string", result.replacement)).unwrap();
        assert!(matches!(&commands[0], TypedCommand::Cast(args) if args.column == name));
    }

    #[test]
    fn completion_tracks_partial_and_chained_commands() {
        let found = complete("sel", 3, &[]);
        assert!(found.iter().any(|c| c.display == "select"));
        assert!(
            !complete("load C:\\data.csv", 16, &[("x".into(), "String".into())])
                .iter()
                .any(|c| c.display == "x")
        );
        let columns = [("x".to_string(), "String".to_string())];
        assert!(complete("load file - select ", 19, &columns)
            .iter()
            .any(|c| c.display == "x"));
        assert!(help("load file - select x", 21).starts_with("select\n\nUsage: "));
        assert_eq!(
            words(":write \"C:\\my files\\out.yml\"").unwrap()[1],
            "C:\\my files\\out.yml"
        );
        assert_eq!(
            tokenize(&format!("x {}", quote_argument(r#"a\b"c"#))).unwrap()[1],
            r#"a\b"c"#
        );
    }

    #[test]
    fn preview_prefix_uses_valid_commands_before_active_segment() {
        for input in ["flatten - select ", "flatten - select \"timestamp"] {
            let prefix = preview_prefix(input);
            assert_eq!(prefix.len(), 1);
            assert!(matches!(&prefix[0], TypedCommand::Flatten(_)));
        }
        let prefix = preview_prefix("flatten - select timestamp - select ");
        assert_eq!(prefix.len(), 2);
        assert!(matches!(&prefix[1], TypedCommand::Select(_)));
        assert!(preview_prefix("select -- - select x").is_empty());
        let prefix = preview_prefix("flatten - select -日志");
        assert_eq!(prefix.len(), 1);
        assert!(matches!(&prefix[0], TypedCommand::Flatten(_)));
        assert!(complete("select -🙂", "select -🙂".len(), &[]).is_empty());
    }

    #[test]
    fn select_completion_replaces_only_the_comma_field_at_cursor() {
        let columns = ["timestamp", "hoge", "fuga"]
            .into_iter()
            .map(|s| (s.to_string(), "String".to_string()))
            .collect::<Vec<_>>();
        let input = "select timestamp,ho,fuga";
        let found = complete(input, "select timestamp,ho".len(), &columns);
        let hoge = found.iter().find(|c| c.display == "hoge").unwrap();
        assert_eq!(&input[..hoge.range.start], "select timestamp,");
        assert_eq!(&input[hoge.range.end..], ",fuga");
        assert_eq!(hoge.replacement, "hoge");
        let middle = "select timestamp,hzz,fuga";
        let cursor = "select timestamp,h".len();
        let completion = complete(middle, cursor, &columns)
            .into_iter()
            .find(|c| c.display == "hoge")
            .unwrap();
        assert_eq!(&middle[completion.range.clone()], "hzz");
        let mut replaced = middle.to_string();
        replaced.replace_range(completion.range, &completion.replacement);
        assert!(
            matches!(&parse_commands(&replaced).unwrap()[0], TypedCommand::Select(args) if args.columns == ["timestamp", "hoge", "fuga"])
        );
        let trailing_draft = "select timestamp,";
        let trailing = complete(trailing_draft, trailing_draft.len(), &columns);
        assert!(trailing
            .iter()
            .any(|c| c.display == "hoge" && c.range.is_empty()));

        let mut draft = "flatten - select timestamp,ho,fuga - head 1".to_string();
        let cursor = "flatten - select timestamp,ho".len();
        let completion = complete(&draft, cursor, &columns)
            .into_iter()
            .find(|c| c.display == "hoge")
            .unwrap();
        draft.replace_range(completion.range, &completion.replacement);
        assert!(parse_commands(&draft).is_ok());

        let unicode = [("名字".to_string(), "String".to_string())];
        let draft = "select 名,名字";
        let cursor = "select 名".len();
        let found = complete(draft, cursor, &unicode);
        let completion = found.iter().find(|c| c.display == "名字").unwrap();
        assert_eq!(&draft[completion.range.clone()], "名");
        let mut replaced = draft.to_string();
        replaced.replace_range(completion.range.clone(), &completion.replacement);
        assert!(parse_commands(&replaced).is_ok());

        let mut quoted = "flatten - select \"a".to_string();
        let completion = complete(&quoted, quoted.len(), &[("a,b c".into(), "String".into())])
            .into_iter()
            .find(|c| c.display == "a,b c")
            .unwrap();
        quoted.replace_range(completion.range, &completion.replacement);
        assert!(
            matches!(&parse_commands(&quoted).unwrap()[1], TypedCommand::Select(args) if args.columns == ["a,b c"])
        );
    }

    #[test]
    fn select_target_and_format_select_round_trip_exact_names() {
        let input = "flatten - select timestamp,hoge,fuga - cast x string";
        let target = select_target(input, "flatten - select timestamp,hoge,fuga".len()).unwrap();
        assert_eq!(&input[target.range.clone()], "select timestamp,hoge,fuga");
        assert_eq!(target.columns, ["timestamp", "hoge", "fuga"]);
        let unicode_space = "select hoge\u{3000}- head 1";
        let target = select_target(unicode_space, "select hoge".len()).unwrap();
        assert_eq!(&unicode_space[target.range.clone()], "select hoge");
        let mut replaced = unicode_space.to_string();
        replaced.replace_range(target.range, &format_select(&["fuga".to_owned()]));
        assert!(parse_commands(&replaced).is_ok());
        assert_eq!(
            format_select(&["timestamp", "hoge", "fuga"].map(str::to_owned)),
            "select timestamp,hoge,fuga"
        );
        let positional = select_target("select -- timestamp,hoge", 24).unwrap();
        assert_eq!(positional.columns, ["timestamp", "hoge"]);
        let blank = select_target("", 0).unwrap();
        assert!(blank.range.is_empty() && blank.columns.is_empty());
        assert!(select_target("flatten - cast x string", 24).is_none());

        let names = [
            "a,b",
            "quote\"x",
            "it's",
            "slash\\name",
            " space ",
            "-lead",
            "a:b",
            "a\":\"b",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        for name in &names {
            let formatted = format_select(std::slice::from_ref(name));
            let parsed = parse_commands(&formatted).unwrap();
            assert!(
                matches!(&parsed[0], TypedCommand::Select(args) if args.columns == [name.clone()])
            );
        }
        let formatted = format_select(&names);
        let parsed = parse_commands(&format!("{} - head 1", formatted)).unwrap();
        assert!(matches!(&parsed[0], TypedCommand::Select(args) if args.columns == names));
    }
}
