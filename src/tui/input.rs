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

pub(super) fn parse_commands(input: &str) -> Result<Vec<TypedCommand>, QuiltError> {
    let argv = tokenize(input)?;
    if argv.is_empty() {
        return Ok(Vec::new());
    }
    parse_typed_commands(&argv)
}

pub(super) fn words(input: &str) -> Result<Vec<String>, QuiltError> {
    tokenize(input)
}

fn tokenize(input: &str) -> Result<Vec<String>, QuiltError> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut quote, mut started) = (None, false);
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(q)
                if c == '\\'
                    && matches!(chars.peek(), Some(next) if *next == q || *next == '\\') =>
            {
                word.push(chars.next().unwrap());
            }
            Some(_) => word.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            None => {
                word.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(QuiltError::usage("Error: Incomplete quoted argument"));
    }
    if started {
        out.push(word);
    }
    Ok(out)
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

fn looks_like_path_slot(command: &str, arg_index: usize) -> bool {
    matches!(command, "load" | "dump" | "dumpcache" | "partition" | "run") && arg_index == 0
}

fn quote_column(name: &str, command: &str) -> String {
    if command == "select" && name.chars().any(|c| matches!(c, ',' | '"')) {
        let csv = format!("\"{}\"", name.replace('"', "\"\""));
        quote_argument(&csv)
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
}
