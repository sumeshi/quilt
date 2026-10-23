use crate::controllers::command_model::{
    parse_typed_commands, registry, render_command_help, CommandCategory, TypedCommand,
};
use crate::controllers::definitions::{argument_specs, option_metadata, ArgumentKind};
use crate::QuiltError;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Completion {
    pub display: String,
    pub replacement: String,
    pub range: Range<usize>,
    pub description: String,
    pub kind: CompletionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompletionKind {
    Command,
    Option,
    Column,
    Builtin,
}

struct BuiltinAction {
    name: &'static str,
    usage: &'static str,
    description: &'static str,
}

const BUILTIN_ACTIONS: &[BuiltinAction] = &[
    BuiltinAction {
        name: ":edit",
        usage: ":edit STEP",
        description: "Edit the original command group containing a 1-based step",
    },
    BuiltinAction {
        name: ":write",
        usage: ":write PATH",
        description: "Write the confirmed pipeline as YAML",
    },
    BuiltinAction {
        name: ":export",
        usage: ":export PATH [--format csv|jsonl]",
        description: "Export the full confirmed result; format defaults by extension",
    },
    BuiltinAction {
        name: ":cache",
        usage: ":cache [PATH]",
        description: "Save a session checkpoint, or persistent Parquet cache",
    },
    BuiltinAction {
        name: ":clear-cache",
        usage: ":clear-cache",
        description: "Clear cached pipeline data",
    },
    BuiltinAction {
        name: ":undo",
        usage: ":undo",
        description: "Undo the last pipeline edit",
    },
    BuiltinAction {
        name: ":redo",
        usage: ":redo",
        description: "Redo the last undone edit",
    },
    BuiltinAction {
        name: ":quit",
        usage: ":quit",
        description: "Leave the TUI",
    },
];

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
        let unquoted = !token.has_shell_quotes;
        let after_max = unquoted
            && positional_only
            && word == "-"
            && spec.is_some_and(|s| s.max_args.is_some_and(|n| positional_count >= n));
        if unquoted && word == "-" && (!positional_only || after_max) {
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
        if unquoted && word == "--" && !positional_only {
            positional_only = true;
            continue;
        }
        if unquoted && !positional_only {
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

/// Split source text into pipeline stages while retaining each stage's original
/// quoting and spacing. Quoted dashes and option values remain part of a stage.
pub(super) fn stage_sources(source: &str) -> Vec<&str> {
    let tokens = token_spans(source);
    let separators = pipeline_separators(&tokens);
    let mut stages = Vec::with_capacity(separators.len() + 1);
    let mut start = 0;
    for separator in separators {
        let token = &tokens[separator];
        let stage = source[start..token.start].trim();
        if !stage.is_empty() {
            stages.push(stage);
        }
        start = token.end;
    }
    let stage = source[start..].trim();
    if !stage.is_empty() {
        stages.push(stage);
    }
    stages
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
        columns: crate::controllers::cli_adapter::parse_column_list(&joined)
            .unwrap_or_else(|_| csv_columns_lenient(&joined)),
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

pub(super) use crate::controllers::cli_export::quote_argument;

pub(super) fn completion_help(completion: &Completion) -> Option<String> {
    match completion.kind {
        CompletionKind::Command => render_command_help(&completion.display),
        CompletionKind::Builtin => BUILTIN_ACTIONS
            .iter()
            .find(|action| action.name == completion.display)
            .map(|action| format!("{}\n  {}", action.usage, action.description)),
        CompletionKind::Option | CompletionKind::Column => None,
    }
}

pub(super) fn placeholder(
    input: &str,
    cursor: usize,
    selected: Option<&Completion>,
    has_source: bool,
) -> String {
    if cursor != input.len() || !input.is_char_boundary(cursor) {
        return String::new();
    }
    if input.trim().is_empty() {
        if let Some(selected) = selected.filter(|completion| {
            matches!(
                completion.kind,
                CompletionKind::Command | CompletionKind::Builtin
            )
        }) {
            return format!(
                "{}{}",
                selected.replacement,
                completion_usage_tail(selected)
            );
        }
        return if has_source {
            "Type a command · count [columns] ...".to_owned()
        } else {
            "load <file.csv|file.jsonl> ...".to_owned()
        };
    }

    let spans = token_spans(input);
    let separators = pipeline_separators(&spans);
    let first = separators
        .iter()
        .rposition(|&index| spans[index].start < cursor)
        .map_or(0, |position| separators[position] + 1);
    let last = separators
        .iter()
        .find(|&&index| spans[index].start >= cursor)
        .copied()
        .unwrap_or(spans.len());
    let segment = spans.get(first..last).unwrap_or_default();
    let command = segment.first().map(|token| token.value.as_str());
    let trailing_space = input.chars().last().is_some_and(char::is_whitespace);

    if trailing_space && segment.len() == 1 {
        match command {
            Some("select") => return "<columns> · Ctrl+A c choose".to_owned(),
            Some("count") => return "[columns] · empty = repeated rows".to_owned(),
            _ => {}
        }
    }

    if let Some(selected) = selected {
        if matches!(
            selected.kind,
            CompletionKind::Command | CompletionKind::Builtin
        ) && selected.range.end == cursor
            && selected.range.start <= selected.range.end
        {
            if let Some(fragment) = input.get(selected.range.clone()) {
                if !fragment.contains(['"', '\'']) && selected.replacement.starts_with(fragment) {
                    let suffix = &selected.replacement[fragment.len()..];
                    let tail = completion_usage_tail(selected);
                    if !suffix.is_empty() || !tail.is_empty() {
                        return format!("{suffix}{tail}");
                    }
                }
            }
        }
    }

    if trailing_space && segment.len() == 1 {
        if let Some(name) = command {
            if let Some(action) = BUILTIN_ACTIONS.iter().find(|action| action.name == name) {
                return action
                    .usage
                    .strip_prefix(action.name)
                    .unwrap_or("")
                    .to_owned();
            }
            if let Some(spec) = registry().iter().find(|spec| spec.name == name) {
                let args = argument_specs(spec);
                if !args.is_empty() {
                    return args
                        .iter()
                        .map(|arg| format!(" <{}>", arg.name))
                        .collect::<String>();
                }
                return spec.help.strip_prefix(spec.name).unwrap_or("").to_owned();
            }
        }
    }
    String::new()
}

fn completion_usage_tail(completion: &Completion) -> String {
    match completion.kind {
        CompletionKind::Command => registry()
            .iter()
            .find(|spec| spec.name == completion.display)
            .map(|spec| spec.help.strip_prefix(spec.name).unwrap_or("").to_owned())
            .unwrap_or_default(),
        CompletionKind::Builtin => BUILTIN_ACTIONS
            .iter()
            .find(|action| action.name == completion.display)
            .map(|action| {
                action
                    .usage
                    .strip_prefix(action.name)
                    .unwrap_or("")
                    .to_owned()
            })
            .unwrap_or_default(),
        CompletionKind::Option | CompletionKind::Column => String::new(),
    }
}

pub(super) fn complete(
    input: &str,
    cursor: usize,
    columns: &[(String, String)],
) -> Vec<Completion> {
    if cursor > input.len() || !input.is_char_boundary(cursor) {
        return vec![];
    }
    let left = &input[..cursor];
    let spans = token_spans(input);
    let active_token = spans.iter().find(|t| t.start <= cursor && cursor <= t.end);
    let start = left
        .rfind(char::is_whitespace)
        .map_or(0, |i| i + left[i..].chars().next().unwrap().len_utf8());
    let token_range = active_token.map_or(start..cursor, |t| t.start..t.end);
    let trimmed_left = left.trim_start();
    if trimmed_left.starts_with(':') && !trimmed_left.chars().any(char::is_whitespace) {
        return BUILTIN_ACTIONS
            .iter()
            .filter(|action| action.name.starts_with(trimmed_left))
            .map(|action| builtin_completion(action, token_range.clone()))
            .collect();
    }
    if let Some((first, last)) = active_select(&spans, cursor) {
        if let Some(completion) = complete_select_field(cursor, columns, &spans[first + 1..last]) {
            return completion;
        }
    }
    let prefix = &left[start..];
    let decoded_prefix = active_token
        .map(|t| {
            let n = t
                .source
                .iter()
                .take_while(|source| source.start < cursor)
                .count();
            t.value.get(..n).unwrap_or("")
        })
        .unwrap_or(prefix);
    let tokens = tokenize(left)
        .or_else(|_| tokenize(&format!("{left}\"")))
        .or_else(|_| tokenize(&format!("{left}'")))
        .unwrap_or_default();
    let trailing_space = left.chars().last().is_some_and(char::is_whitespace);
    let current_word_complete = trailing_space;
    let separators = pipeline_separators(&spans);
    let mut segment_start = separators
        .iter()
        .rev()
        .find(|&&i| spans[i].start < cursor)
        .map_or(0, |&i| i + 1);
    if !current_word_complete {
        segment_start = segment_start.min(tokens.len());
    }
    let segment = &tokens[segment_start..];
    let has_partial = !trailing_space && !tokens.is_empty();
    let at_command = segment.is_empty() || (has_partial && segment.len() == 1);
    let mut candidates: Vec<Completion> = Vec::new();
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
                .map(|s| Completion {
                    display: s.name.to_owned(),
                    replacement: s.name.to_owned(),
                    range: token_range.clone(),
                    description: s.summary.to_owned(),
                    kind: CompletionKind::Command,
                }),
        );
        if tokens.is_empty() {
            candidates.extend(
                BUILTIN_ACTIONS
                    .iter()
                    .map(|action| builtin_completion(action, token_range.clone())),
            );
        }
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
                    .filter(|option| {
                        !(spec.name == "load" && matches!(option.name, "stdin-format" | "rejects"))
                    })
                    .map(|o| {
                        let display = format!("--{}", o.name);
                        let metadata = option_metadata(spec, o);
                        let description = if o.takes_value {
                            format!(
                                "{display} <{}> · {}",
                                metadata.value_hint.unwrap_or("value"),
                                metadata.description
                            )
                        } else {
                            format!("{display} · {}", metadata.description)
                        };
                        Completion {
                            display: display.clone(),
                            replacement: display,
                            range: token_range.clone(),
                            description,
                            kind: CompletionKind::Option,
                        }
                    }),
            );
            let prior = segment.last().map(String::as_str).unwrap_or("");
            let prior_args = if trailing_space {
                &segment[1..]
            } else {
                &segment[1..segment.len().saturating_sub(1)]
            };
            let (arg_index, pending_option) = preceding_argument_context(spec, prior_args);
            let inline_option = if !trailing_space {
                prior.split_once('=').and_then(|(name, _)| {
                    name.strip_prefix("--").and_then(|name| {
                        spec.options
                            .iter()
                            .find(|option| option.name == name && option.takes_value)
                    })
                })
            } else {
                None
            };
            let inline_value_prefix = if inline_option.is_some() {
                prior.split_once('=').map(|(_, value)| value)
            } else {
                None
            };
            let arg_kind = if let Some(option) = inline_option.or(pending_option) {
                option_metadata(spec, option).kind
            } else {
                argument_specs(spec)
                    .get(arg_index)
                    .map(|arg| arg.kind)
                    .unwrap_or(ArgumentKind::Value)
            };
            let list_slot = arg_kind == ArgumentKind::ColumnList;
            if list_slot
                && active_token
                    .is_some_and(|token| token.has_shell_quotes || !token.value.starts_with('-'))
            {
                if let Some(token) = active_token {
                    if let Some(found) =
                        complete_select_field(cursor, columns, std::slice::from_ref(token))
                    {
                        return found;
                    }
                }
            }
            let want_columns = trailing_space
                || pending_option.is_some()
                || inline_option.is_some()
                || (!prior.starts_with('-') && !prior.is_empty());
            if want_columns && arg_kind == ArgumentKind::Path {
                candidates.extend(path_completions(
                    inline_value_prefix.unwrap_or(decoded_prefix),
                    token_range.clone(),
                ));
            }
            if want_columns {
                if let ArgumentKind::Choice(choices) = arg_kind {
                    let option_prefix = inline_option
                        .map(|_| {
                            let (name, _) = prior.split_once('=').unwrap();
                            format!("{name}=")
                        })
                        .unwrap_or_default();
                    candidates.extend(choices.iter().map(|choice| Completion {
                        display: (*choice).to_owned(),
                        replacement: format!("{option_prefix}{choice}"),
                        range: token_range.clone(),
                        description: "Supported value".into(),
                        kind: CompletionKind::Option,
                    }));
                }
            }
            if want_columns && matches!(arg_kind, ArgumentKind::Column | ArgumentKind::ColumnList) {
                candidates.extend(columns.iter().map(|(name, dtype)| Completion {
                    display: name.clone(),
                    replacement: if active_token.is_some_and(|token| token.has_shell_quotes) {
                        quote_argument(name)
                    } else {
                        quote_column(name, list_slot)
                    },
                    range: token_range.clone(),
                    description: format!("Column · {dtype}"),
                    kind: CompletionKind::Column,
                }));
            }
        }
    }
    let match_prefix = decoded_prefix
        .split_once('=')
        .map(|(_, value)| value.to_owned())
        .unwrap_or_else(|| decoded_prefix.trim_start_matches(['"', '\'']).to_owned());
    candidates
        .into_iter()
        .filter(|candidate| candidate.display.starts_with(&match_prefix))
        .collect()
}

fn path_completions(prefix: &str, range: Range<usize>) -> Vec<Completion> {
    use std::path::Path;
    let path = Path::new(prefix);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stem = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return vec![];
    };
    entries
        .take(256)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with(stem) {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            let mut value = if parent == Path::new(".") {
                name
            } else {
                format!("{}/{}", parent.display(), name)
            };
            if is_dir {
                value.push('/');
            }
            Some(Completion {
                display: value.clone(),
                replacement: quote_argument(&value),
                range: range.clone(),
                description: if is_dir {
                    "Directory".into()
                } else {
                    "File path".into()
                },
                kind: CompletionKind::Option,
            })
        })
        .collect()
}

fn builtin_completion(action: &BuiltinAction, range: Range<usize>) -> Completion {
    Completion {
        display: action.name.to_owned(),
        replacement: action.name.to_owned(),
        range,
        description: action.description.to_owned(),
        kind: CompletionKind::Builtin,
    }
}

fn preceding_argument_context<'a>(
    spec: &'a crate::controllers::definitions::CommandSpec,
    tokens: &[String],
) -> (
    usize,
    Option<&'a crate::controllers::definitions::OptionSpec>,
) {
    let mut positional = 0;
    let mut pending = None;
    let mut positional_only = false;
    for token in tokens {
        if pending.take().is_some() {
            continue;
        }
        if !positional_only && token == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && token.starts_with("--") {
            let raw = &token[2..];
            let (name, inline) = raw
                .split_once('=')
                .map_or((raw, false), |(name, _)| (name, true));
            if let Some(option) = spec.options.iter().find(|option| option.name == name) {
                if option.takes_value && !inline {
                    pending = Some(option);
                }
                continue;
            }
        } else if !positional_only && token.starts_with('-') && token.len() > 1 {
            let short_len = token[1..].chars().next().map(char::len_utf8).unwrap_or(0);
            let short = &token[1..1 + short_len];
            if let Some(option) = spec
                .options
                .iter()
                .find(|option| option.short == Some(short))
            {
                if option.takes_value && token.len() == 2 {
                    pending = Some(option);
                }
                continue;
            }
        }
        positional += 1;
    }
    (positional, pending)
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
            .map(|(name, dtype)| {
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
                    (quote_column(name, true), raw_start..raw_end)
                };
                Completion {
                    display: name.clone(),
                    replacement,
                    range,
                    description: format!("Column · {dtype}"),
                    kind: CompletionKind::Column,
                }
            })
            .collect(),
    )
}

fn quote_column(name: &str, column_list: bool) -> String {
    if column_list {
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
        let action_name = format!(":{name}");
        return BUILTIN_ACTIONS
            .iter()
            .find(|action| action.name == action_name)
            .map(|action| format!("{}  {}", action.usage, action.description))
            .unwrap_or_else(|| {
                ":export PATH [--format csv|jsonl], :write PATH, :cache [PATH], \
                 :clear-cache, :undo, :redo, :edit STEP, :quit"
                    .into()
            });
    }
    let tokens = tokenize(trimmed).unwrap_or_default();
    let spans = token_spans(trimmed);
    let start = pipeline_separators(&spans)
        .last()
        .map_or(0, |index| index + 1);
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
    fn metadata_drives_positional_and_option_choice_completion() {
        let no_columns = &[];
        let cast = complete("cast amount st", "cast amount st".len(), no_columns);
        assert!(cast.iter().any(|item| item.display == "string"));
        let cast_with_flag = complete(
            "cast --strict amount da",
            "cast --strict amount da".len(),
            no_columns,
        );
        assert!(cast_with_flag.iter().any(|item| item.display == "datetime"));
        let columns = [("timestamp".to_owned(), "Datetime".to_owned())];
        let sorted = complete("sort --desc ti", "sort --desc ti".len(), &columns);
        assert!(sorted.iter().any(|item| item.display == "timestamp"));
        let contains_column = complete("contains ti", "contains ti".len(), &columns);
        assert!(contains_column
            .iter()
            .any(|item| item.display == "timestamp"));
        let contains_pattern =
            complete("contains amount pat", "contains amount pat".len(), &columns);
        assert!(contains_pattern
            .iter()
            .all(|item| item.kind != CompletionKind::Column));
        let sed_column = complete(
            "sed pat repl --column ti",
            "sed pat repl --column ti".len(),
            &columns,
        );
        assert!(sed_column.iter().any(|item| item.display == "timestamp"));
        let grep_column = complete("grep --column ti", "grep --column ti".len(), &columns);
        assert!(grep_column.iter().any(|item| item.display == "timestamp"));
        let format = complete("show --format j", "show --format j".len(), no_columns);
        assert!(format.iter().any(|item| item.display == "jsonl"));
        let inline_format = complete("show --format=j", "show --format=j".len(), no_columns);
        assert!(inline_format
            .iter()
            .any(|item| item.display == "jsonl" && item.replacement == "--format=jsonl"));
        let policy = complete(
            "bucket amount 1d --ambiguous e",
            "bucket amount 1d --ambiguous e".len(),
            no_columns,
        );
        assert!(policy.iter().any(|item| item.display == "error"));

        let sort = crate::controllers::definitions::by_name("sort").unwrap();
        let strange_short = vec!["-名".to_owned()];
        let _ = preceding_argument_context(sort, &strange_short);

        let dir = tempfile::tempdir().unwrap();
        let filenames: &[&str] = if cfg!(windows) {
            &["quote'日本語"]
        } else {
            &["quote'日本語", "quote\"\\name"]
        };
        for filename in filenames {
            std::fs::write(dir.path().join(filename), "").unwrap();
            let prefix = format!("{}/quote", dir.path().display());
            let found = path_completions(&prefix, 0..prefix.len());
            let found = found
                .iter()
                .find(|item| item.display.ends_with(filename))
                .unwrap();
            assert_eq!(
                tokenize(&found.replacement).unwrap().as_slice(),
                std::slice::from_ref(&found.display)
            );
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
    fn list_column_completion_round_trips_comma_space_and_quote_names() {
        let names = ["a,b c", "a'b", "say\"hi"];
        let columns = names
            .iter()
            .map(|name| (name.to_string(), "String".to_string()))
            .collect::<Vec<_>>();
        for (draft, prefix) in [
            ("count ", "count "),
            ("sort ", "sort "),
            ("grep pattern --column ", "grep pattern --column "),
            (
                "flatten - select x - grep pattern --column ",
                "flatten - select x - grep pattern --column ",
            ),
        ] {
            for name in names {
                let found = complete(draft, draft.len(), &columns);
                let item = found
                    .iter()
                    .find(|candidate| candidate.display == name)
                    .unwrap();
                let command = format!("{draft}{}", item.replacement);
                let parsed = parse_commands(&command).unwrap();
                let selected = parsed.last().unwrap();
                let result = match selected {
                    TypedCommand::Count(args) => &args.columns,
                    TypedCommand::Sort(args) => &args.columns,
                    TypedCommand::Grep(args) => args.columns.as_ref().unwrap(),
                    _ => unreachable!(),
                };
                assert_eq!(result, &[name.to_string()], "{prefix}");
            }
        }
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
    fn generic_completion_uses_decoded_prefix_and_replaces_whole_token() {
        let cols = vec![
            ("status".to_owned(), "String".to_owned()),
            ("field name".to_owned(), "String".to_owned()),
            ("group".to_owned(), "String".to_owned()),
        ];
        let input = "cast statusus";
        let cursor = "cast status".len();
        let result = complete(input, cursor, &cols)
            .into_iter()
            .find(|c| c.display == "status")
            .unwrap();
        assert_eq!(&input[result.range.clone()], "statusus");
        let mut replaced = input.to_owned();
        replaced.replace_range(result.range, &result.replacement);
        assert_eq!(replaced, "cast status");

        let quoted = r#"cast "field n""#;
        let result = complete(quoted, quoted.len() - 1, &cols)
            .into_iter()
            .find(|c| c.display == "field name")
            .unwrap();
        let mut replaced = quoted.to_owned();
        replaced.replace_range(result.range, &result.replacement);
        assert_eq!(tokenize(&replaced).unwrap(), ["cast", "field name"]);

        let unicode_cols = [("名名字".to_owned(), "String".to_owned())];
        let unicode = r#"cast "名x""#;
        let result = complete(unicode, "cast \"名".len(), &unicode_cols)
            .into_iter()
            .find(|c| c.display == "名名字")
            .unwrap();
        assert_eq!(&unicode[result.range], r#""名x""#);

        let builtin = ":undoSuffix";
        let result = complete(builtin, ":un".len(), &[])
            .into_iter()
            .find(|c| c.display == ":undo")
            .unwrap();
        assert_eq!(&builtin[result.range], builtin);

        for list in ["count id,gr", "sort id,gr", "explode id,gr"] {
            assert!(
                complete(list, list.len(), &cols)
                    .iter()
                    .any(|c| c.display == "group"),
                "{list}"
            );
        }
        assert!(complete("sort --d", "sort --d".len(), &cols)
            .iter()
            .any(|c| c.kind == CompletionKind::Option && c.display == "--desc"));
    }

    #[test]
    fn completion_catalog_and_builtin_metadata_are_complete() {
        let initial = complete("", 0, &[]);
        for spec in registry().iter().filter(|spec| {
            matches!(
                spec.category,
                CommandCategory::Initializer | CommandCategory::Chainable
            )
        }) {
            let candidate = initial.iter().find(|c| c.display == spec.name).unwrap();
            assert_eq!(candidate.kind, CompletionKind::Command);
            assert_eq!(candidate.description, spec.summary);
        }
        assert!(!initial.iter().any(|c| {
            c.kind == CompletionKind::Command
                && registry().iter().any(|spec| {
                    spec.name == c.display
                        && !matches!(
                            spec.category,
                            CommandCategory::Initializer | CommandCategory::Chainable
                        )
                })
        }));
        assert!(initial
            .iter()
            .any(|c| c.display == ":cache" && c.kind == CompletionKind::Builtin));

        let export = initial.iter().find(|c| c.display == ":export").unwrap();
        assert_eq!(
            export.description,
            "Export the full confirmed result; format defaults by extension"
        );
        assert!(completion_help(export)
            .unwrap()
            .contains(":export PATH [--format csv|jsonl]"));

        let colon = complete(":ed", 3, &[]);
        let edit = colon.iter().find(|c| c.display == ":edit").unwrap();
        assert_eq!(
            edit.description,
            "Edit the original command group containing a 1-based step"
        );
        assert!(completion_help(edit).unwrap().contains(":edit STEP"));
        assert!(complete("flatten - :ed", "flatten - :ed".len(), &[])
            .iter()
            .all(|c| c.kind != CompletionKind::Builtin));

        let options = complete("load --", "load --".len(), &[]);
        assert!(!options
            .iter()
            .any(|c| matches!(c.display.as_str(), "--stdin-format" | "--rejects")));
        let option = options
            .iter()
            .find(|c| c.kind == CompletionKind::Option)
            .unwrap();
        assert!(option.description.starts_with("--"));
        assert!(option.description.contains(" · ") || option.description.contains(" <"));
        let columns = [("timestamp".to_owned(), "i64".to_owned())];
        let column = complete("select ", "select ".len(), &columns)
            .into_iter()
            .find(|c| c.display == "timestamp")
            .unwrap();
        assert_eq!(column.description, "Column · i64");
        assert_eq!(column.kind, CompletionKind::Column);
    }

    #[test]
    fn placeholder_follows_context_and_selected_usage_at_the_end_only() {
        assert_eq!(
            placeholder("", 0, None, false),
            "load <file.csv|file.jsonl> ..."
        );
        assert_eq!(
            placeholder("", 0, None, true),
            "Type a command · count [columns] ..."
        );
        assert_eq!(
            placeholder("select ", 7, None, true),
            "<columns> · Ctrl+A c choose"
        );
        assert_eq!(
            placeholder("count ", 6, None, true),
            "[columns] · empty = repeated rows"
        );
        assert_eq!(placeholder("filter ", 7, None, true), " <expression>");
        let chained = "flatten - select timestamp - count ";
        assert_eq!(
            placeholder(chained, chained.len(), None, true),
            "[columns] · empty = repeated rows"
        );

        let selected = complete("sel", 3, &[])
            .into_iter()
            .find(|c| c.display == "select")
            .unwrap();
        assert_eq!(
            placeholder("sel", 3, Some(&selected), true),
            "ect <columns>"
        );
        assert_eq!(placeholder("sel", 2, Some(&selected), true), "");
        let count = complete("", 0, &[])
            .into_iter()
            .find(|c| c.display == "count")
            .unwrap();
        assert_eq!(placeholder("", 0, Some(&count), true), "count [columns]");
    }

    #[test]
    fn preview_prefix_uses_valid_commands_before_active_segment() {
        let source = r#"load "logs/a-b.csv" - grep -- "-" - head 3"#;
        assert_eq!(
            stage_sources(source),
            [r#"load "logs/a-b.csv""#, r#"grep -- "-""#, "head 3"]
        );
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
        let quoted_range = r#"select '"col:1":"col:3"'"#;
        let target = select_target(quoted_range, quoted_range.len()).unwrap();
        let available = ["col:1", "middle", "col:3"].map(str::to_owned);
        assert_eq!(
            crate::operations::chainables::select::expand_select_columns(
                &target.columns,
                &available,
            )
            .unwrap(),
            available,
        );

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
