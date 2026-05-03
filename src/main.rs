use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process;

mod controllers;
mod operations;

use controllers::command::{
    parse_batch_size, parse_commands, print_chainable_help, print_help, Command,
};
use controllers::dataframe::DataFrameController;
use once_cell::sync::Lazy;
use regex::Regex;

// Define static Regex patterns for column range parsing (both colon and hyphen notation)
static RE_COL_RANGE_COLON: Lazy<Regex> = Lazy::new(|| {
    // This regex captures colon notation: col1:col3 or col1:3
    // p1: The prefix of the start of the range (e.g., "col")
    // n1: The number of the start of the range (e.g., "1")
    // p2: (Optional) The prefix of the end of the range if specified (e.g., "col" in "col1:col3")
    // n2: (Conditional) The number of the end of the range if p2 is specified (e.g., "3" in "col1:col3")
    // n3: (Conditional) The number of the end of the range if p2 is NOT specified (e.g., "3" in "col1:3")
    Regex::new(r"^(?P<p1>[a-zA-Z_][a-zA-Z_0-9]*)(?P<n1>\d+):(?:(?P<p2>[a-zA-Z_][a-zA-Z_0-9]*)(?P<n2>\d+)|(?P<n3>\d+))$").unwrap()
});

static RE_COL_RANGE_HYPHEN: Lazy<Regex> = Lazy::new(|| {
    // This regex captures hyphen notation: col1-col3 or col1-3
    // p1: The prefix of the start of the range (e.g., "col")
    // n1: The number of the start of the range (e.g., "1")
    // p2: (Optional) The prefix of the end of the range if specified (e.g., "col" in "col1-col3")
    // n2: (Conditional) The number of the end of the range if p2 is specified (e.g., "3" in "col1-col3")
    // n3: (Conditional) The number of the end of the range if p2 is NOT specified (e.g., "3" in "col1-3")
    Regex::new(r"^(?P<p1>[a-zA-Z_][a-zA-Z_0-9]*)(?P<n1>\d+)-(?:(?P<p2>[a-zA-Z_][a-zA-Z_0-9]*)(?P<n2>\d+)|(?P<n3>\d+))$").unwrap()
});

fn main() {
    // Initialize logger without timestamp (LogController provides high-precision timestamps)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error"))
        .format(|buf, record| writeln!(buf, "{}", record.args()))
        .init();

    // Get command line arguments
    let args: Vec<String> = std::env::args().collect();

    // No arguments provided - show help
    if args.len() == 1 {
        print_help();
        return;
    }

    // -h, --help
    if args.len() == 2 && (args[1] == "-h" || args[1] == "--help") {
        print_help();
        return;
    }
    // -v, --version
    if args.len() == 2 && (args[1] == "-v" || args[1] == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Subcommand help: e.g., qsv select -h
    if args.len() >= 3 && (args[2] == "-h" || args[2] == "--help") {
        print_chainable_help(&args[1]);
        return;
    }

    // Parse commands using dedicated command parser module
    let commands = parse_commands(&args[1..]);

    if commands.is_empty() {
        eprintln!("Error: No commands provided. Use the format: qsv load file.csv - select col1,col2 - head 5");
        process::exit(1);
    }

    // Initialize dataframe controller
    let mut controller = DataFrameController::new();

    // Process commands sequentially
    process_commands(&mut controller, &commands);
}

// Process all commands in sequence
fn process_commands(controller: &mut DataFrameController, commands: &[Command]) {
    for cmd in commands.iter() {
        process_command(controller, cmd);
    }

    // Check if the last command was a finalizer
    if let Some(last_cmd) = commands.last() {
        let finalizer_commands = [
            "show",
            "showtable",
            "headers",
            "stats",
            "showquery",
            "dump",
            "dumpcache",
            "partition",
            "quilt",
        ];

        if !finalizer_commands.contains(&last_cmd.name.as_str()) {
            // Last command was not a finalizer, so call showtable as default
            if !controller.is_empty() {
                controller.showtable();
            }
        }
    }
}

// Check if data is loaded
fn check_data_loaded(controller: &DataFrameController, cmd_name: &str) {
    if controller.is_empty() {
        eprintln!("Error: No data loaded. Please load data first before using '{cmd_name}'.");
        process::exit(1);
    }
}

// New parse_column_names function with range expansion
fn parse_column_names(input: &str) -> Vec<String> {
    let mut result = Vec::new();

    for part in input.split(',') {
        let part = part.trim();

        // Handle quoted colon notation: "col1":"col3"
        if part.starts_with('"') && part.contains(":") && part.ends_with('"') {
            // Pass quoted colon range as-is to select.rs for proper processing
            result.push(part.to_string());
            continue;
        }

        // Try colon notation first (col1:col3), then hyphen notation (col1-col3)
        let captures_opt = RE_COL_RANGE_COLON
            .captures(part)
            .or_else(|| RE_COL_RANGE_HYPHEN.captures(part));

        if let Some(captures) = captures_opt {
            let prefix1 = captures.name("p1").unwrap().as_str();
            let num1: usize = captures.name("n1").unwrap().as_str().parse().unwrap();

            let (prefix2, num2) = if let Some(p2) = captures.name("p2") {
                // Format: col1:col3 or col1-col3
                let prefix2 = p2.as_str();
                let num2: usize = captures.name("n2").unwrap().as_str().parse().unwrap();
                (prefix2, num2)
            } else {
                // Format: col1:3 or col1-3
                let num2: usize = captures.name("n3").unwrap().as_str().parse().unwrap();
                (prefix1, num2)
            };

            // Ensure both prefixes are the same
            if prefix1 != prefix2 {
                eprintln!(
                    "Error: Mismatched prefixes in range '{part}'. Both sides must have the same prefix."
                );
                process::exit(1);
            }

            // Generate the range
            if num1 <= num2 {
                for i in num1..=num2 {
                    result.push(format!("{prefix1}{i}"));
                }
            } else {
                eprintln!("Error: Invalid range '{part}'. Start number must be <= end number.");
                process::exit(1);
            }
        } else {
            // Not a range, add as-is
            result.push(part.to_string());
        }
    }

    result
}

// Process a single command
fn process_command(controller: &mut DataFrameController, cmd: &Command) {
    // Validate command options
    if let Err(error_msg) = controllers::command::validate_command_options(cmd) {
        eprintln!("{error_msg}");
        process::exit(1);
    }

    match cmd.name.as_str() {
        // Initializers
        "load" => {
            if cmd.args.is_empty() {
                eprintln!("Error: 'load' command requires at least one file path");
                process::exit(1);
            }

            let mut paths = Vec::new();
            let separator = cmd
                .options
                .get("separator")
                .or_else(|| cmd.options.get("s"))
                .and_then(|opt| opt.as_ref())
                .cloned()
                .unwrap_or_else(|| ",".to_string());

            let low_memory = cmd.options.contains_key("low_memory");

            let no_headers = cmd.options.contains_key("no_headers");

            let chunk_size = cmd
                .options
                .get("chunk_size")
                .and_then(|opt| opt.as_ref())
                .and_then(|size_str| size_str.parse::<usize>().ok());

            for path_str in &cmd.args {
                paths.push(PathBuf::from(path_str));
            }

            controller.load(&paths, &separator, low_memory, no_headers, chunk_size);
        }

        // Chainables
        "select" => {
            check_data_loaded(controller, "select");

            if cmd.args.is_empty() {
                eprintln!("Error: 'select' command requires column names");
                process::exit(1);
            }

            // Parse as column names
            let colnames = if cmd.args.len() == 1 {
                parse_column_names(&cmd.args[0])
            } else {
                cmd.args.clone()
            };
            controller.select(&colnames);
        }

        "isin" => {
            check_data_loaded(controller, "isin");

            if cmd.args.len() < 2 {
                eprintln!("Error: 'isin' command requires a column name and at least one value string (e.g., isin colname val1,val2,val3)");
                process::exit(1);
            }

            let colname = &cmd.args[0];

            let values_str = &cmd.args[1];
            let values: Vec<String> = values_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if values.is_empty() {
                eprintln!("Error: 'isin' command requires at least one value after splitting the value string by comma.");
                process::exit(1);
            }

            controller.isin(colname, &values);
        }

        "contains" => {
            check_data_loaded(controller, "contains");

            if cmd.args.len() < 2 {
                eprintln!("Error: 'contains' command requires a column name and a pattern");
                process::exit(1);
            }

            let colname = &cmd.args[0];
            let pattern = &cmd.args[1];
            let ignorecase = cmd.options.contains_key("ignore_case");

            controller.contains(colname, pattern, ignorecase);
        }

        "sed" => {
            check_data_loaded(controller, "sed");

            if cmd.args.len() < 2 {
                eprintln!("Error: 'sed' command requires pattern and replacement");
                process::exit(1);
            }

            let pattern = &cmd.args[0];
            let replacement = &cmd.args[1];
            let colname = cmd.options.get("column").and_then(|opt| opt.as_deref());
            let ignorecase = cmd.options.contains_key("ignore_case");

            controller.sed(colname, pattern, replacement, ignorecase);
        }

        "grep" => {
            check_data_loaded(controller, "grep");

            if cmd.args.is_empty() {
                eprintln!("Error: 'grep' command requires a pattern.");
                process::exit(1);
            }

            let pattern = &cmd.args[0];

            let ignorecase = cmd.options.contains_key("ignore_case");
            let is_inverted = cmd.options.contains_key("invert_match");

            controller.grep(pattern, ignorecase, is_inverted);
        }

        "head" => {
            check_data_loaded(controller, "head");

            let number = if !cmd.args.is_empty() {
                cmd.args[0].parse::<usize>().unwrap_or_else(|_| {
                    eprintln!("Error: 'head' command requires a valid number");
                    process::exit(1);
                })
            } else if let Some(Some(n_str)) = cmd.options.get("number") {
                n_str.parse::<usize>().unwrap_or_else(|_| {
                    eprintln!(
                        "Error: 'head' command requires a valid number for --number or -n option"
                    );
                    process::exit(1);
                })
            } else {
                5 // Default value
            };

            controller.head(number);
        }

        "tail" => {
            check_data_loaded(controller, "tail");

            let number = if !cmd.args.is_empty() {
                cmd.args[0].parse::<usize>().unwrap_or_else(|_| {
                    eprintln!("Error: 'tail' command requires a valid number");
                    process::exit(1);
                })
            } else if let Some(Some(n_str)) = cmd.options.get("number") {
                n_str.parse::<usize>().unwrap_or_else(|_| {
                    eprintln!(
                        "Error: 'tail' command requires a valid number for --number or -n option"
                    );
                    process::exit(1);
                })
            } else {
                5 // Default value
            };

            controller.tail(number);
        }

        "sort" => {
            check_data_loaded(controller, "sort");

            if cmd.args.is_empty() {
                eprintln!("Error: 'sort' command requires column names");
                process::exit(1);
            }

            let colnames = if cmd.args.len() == 1 {
                parse_column_names(&cmd.args[0])
            } else {
                cmd.args.clone()
            };

            let desc = cmd.options.contains_key("desc");

            controller.sort(&colnames, desc);
        }

        "count" => {
            check_data_loaded(controller, "count");
            controller.count();
        }

        "uniq" => {
            check_data_loaded(controller, "uniq");
            controller.uniq();
        }

        "changetz" => {
            check_data_loaded(controller, "changetz");

            if cmd.args.is_empty() {
                eprintln!("Error: 'changetz' command requires a column name");
                process::exit(1);
            }

            let colname = &cmd.args[0];

            let tz_from = match cmd.options.get("from_tz") {
                Some(Some(tz)) => tz,
                _ => {
                    eprintln!("Error: 'changetz' command requires --from-tz option");
                    process::exit(1);
                }
            };

            let tz_to = match cmd.options.get("to_tz") {
                Some(Some(tz)) => tz,
                _ => {
                    eprintln!("Error: 'changetz' command requires --to-tz option");
                    process::exit(1);
                }
            };

            let input_format = cmd
                .options
                .get("input_format")
                .and_then(|opt_val| opt_val.as_deref());
            let output_format = cmd
                .options
                .get("output_format")
                .and_then(|opt_val| opt_val.as_deref());
            let ambiguous_time = cmd
                .options
                .get("ambiguous")
                .and_then(|opt_val| opt_val.as_deref());

            controller.changetz(
                colname,
                tz_from,
                tz_to,
                input_format,
                output_format,
                ambiguous_time,
            );
        }

        "renamecol" => {
            check_data_loaded(controller, "renamecol");
            if cmd.args.len() < 2 {
                eprintln!("Error: 'renamecol' command requires the current column name and the new column name.");
                process::exit(1);
            }
            let colname = &cmd.args[0];
            let new_colname = &cmd.args[1];
            controller.renamecol(colname, new_colname);
        }

        "convert" => {
            check_data_loaded(controller, "convert");
            if cmd.args.is_empty() {
                eprintln!("Error: 'convert' command requires a column name");
                process::exit(1);
            }
            let colname = &cmd.args[0];

            let from_format = match cmd.options.get("from") {
                Some(Some(format)) => format,
                _ => {
                    eprintln!("Error: 'convert' command requires --from option");
                    process::exit(1);
                }
            };

            let to_format = match cmd.options.get("to") {
                Some(Some(format)) => format,
                _ => {
                    eprintln!("Error: 'convert' command requires --to option");
                    process::exit(1);
                }
            };

            controller.convert(colname, from_format, to_format);
        }

        "timeline" => {
            check_data_loaded(controller, "timeline");

            if cmd.args.is_empty() {
                eprintln!("Error: 'timeline' command requires a time column name");
                process::exit(1);
            }

            let time_column = &cmd.args[0];

            let interval = match cmd.options.get("interval") {
                Some(Some(interval)) => interval,
                _ => {
                    eprintln!("Error: 'timeline' command requires --interval option (e.g., --interval 1h)");
                    process::exit(1);
                }
            };

            // Determine aggregation type and column
            let (agg_type, agg_column) = if let Some(Some(col)) = cmd.options.get("sum") {
                ("sum", Some(col.as_str()))
            } else if let Some(Some(col)) = cmd.options.get("avg") {
                ("avg", Some(col.as_str()))
            } else if let Some(Some(col)) = cmd.options.get("min") {
                ("min", Some(col.as_str()))
            } else if let Some(Some(col)) = cmd.options.get("max") {
                ("max", Some(col.as_str()))
            } else if let Some(Some(col)) = cmd.options.get("std") {
                ("std", Some(col.as_str()))
            } else {
                ("count", None) // Default to count
            };

            controller.timeline(time_column, interval, agg_type, agg_column);
        }

        "timeslice" => {
            check_data_loaded(controller, "timeslice");

            if cmd.args.is_empty() {
                eprintln!("Error: 'timeslice' command requires a time column name");
                process::exit(1);
            }

            let time_column = &cmd.args[0];

            let start_time = cmd.options.get("start").and_then(|opt| opt.as_deref());
            let end_time = cmd.options.get("end").and_then(|opt| opt.as_deref());

            if start_time.is_none() && end_time.is_none() {
                eprintln!(
                    "Error: 'timeslice' command requires at least one of --start or --end options"
                );
                process::exit(1);
            }

            controller.timeslice(time_column, start_time, end_time);
        }

        "partition" => {
            check_data_loaded(controller, "partition");

            if cmd.args.is_empty() {
                eprintln!("Error: 'partition' command requires a column name");
                process::exit(1);
            }

            let colname = &cmd.args[0];
            let output_dir = if cmd.args.len() > 1 {
                &cmd.args[1]
            } else {
                "./partitions"
            };

            controller.partition(colname, output_dir);
        }

        "pivot" => {
            check_data_loaded(controller, "pivot");

            let rows_str = cmd
                .options
                .get("rows")
                .and_then(|opt| opt.as_deref())
                .unwrap_or("");
            let cols_str = cmd
                .options
                .get("cols")
                .and_then(|opt| opt.as_deref())
                .unwrap_or("");
            let values = cmd
                .options
                .get("values")
                .and_then(|opt| opt.as_deref())
                .unwrap_or_else(|| {
                    eprintln!("Error: 'pivot' command requires --values option");
                    process::exit(1);
                });
            let agg_func = cmd
                .options
                .get("agg")
                .and_then(|opt| opt.as_deref())
                .unwrap_or("sum");

            if rows_str.is_empty() && cols_str.is_empty() {
                eprintln!(
                    "Error: 'pivot' command requires at least one of --rows or --cols options"
                );
                process::exit(1);
            }

            let rows: Vec<String> = if rows_str.is_empty() {
                Vec::new()
            } else {
                rows_str.split(',').map(|s| s.trim().to_string()).collect()
            };

            let columns: Vec<String> = if cols_str.is_empty() {
                Vec::new()
            } else {
                cols_str.split(',').map(|s| s.trim().to_string()).collect()
            };

            controller.pivot(&rows, &columns, values, agg_func);
        }

        "timeround" => {
            check_data_loaded(controller, "timeround");

            if cmd.args.is_empty() {
                eprintln!("Error: 'timeround' command requires a column name");
                process::exit(1);
            }

            let colname = &cmd.args[0];

            let unit = cmd
                .options
                .get("unit")
                .and_then(|opt| opt.as_deref())
                .unwrap_or_else(|| {
                    eprintln!("Error: 'timeround' command requires --unit option (e.g., --unit d)");
                    process::exit(1);
                });

            let output_colname = cmd.options.get("output").and_then(|opt| opt.as_deref());

            controller.timeround(colname, unit, output_colname);
        }

        // Quilters
        "quilt" => {
            if cmd.args.is_empty() {
                eprintln!("Error: 'quilt' command requires a config_path argument.");
                process::exit(1);
            }
            let config_path_str = &cmd.args[0];

            let cli_input_files = if cmd.args.len() > 1 {
                Some(
                    cmd.args[1..]
                        .iter()
                        .map(PathBuf::from)
                        .collect::<Vec<PathBuf>>(),
                )
            } else {
                None
            };

            let output_path_str = cmd.options.get("output").and_then(|o| o.as_deref());
            let quilt_vars = cmd.repeated_options.get("var").cloned().unwrap_or_default();

            // quilt operation is destructive / stateful for the controller for now
            operations::quilters::quilt::quilt(
                controller,
                config_path_str,
                cli_input_files,
                output_path_str,
                &quilt_vars,
            );
        }

        // Finalizers
        "showtable" => {
            check_data_loaded(controller, "showtable");
            controller.showtable();
        }

        "headers" => {
            check_data_loaded(controller, "headers");
            let plain = cmd.options.contains_key("plain");
            controller.headers(plain);
        }

        "show" => {
            check_data_loaded(controller, "show");
            if let Some(batch_size_str) = cmd.options.get("batch_size").and_then(|v| v.as_ref()) {
                match parse_batch_size(batch_size_str) {
                    Ok(batch_size) => {
                        controller.show_with_batch_size(batch_size);
                    }
                    Err(e) => {
                        eprintln!("Error parsing batch-size: {e}");
                        process::exit(1);
                    }
                }
            } else {
                controller.show();
            }
        }

        "stats" => {
            check_data_loaded(controller, "stats");
            controller.stats();
        }

        "showquery" => {
            check_data_loaded(controller, "showquery");
            controller.showquery();
        }

        "dump" => {
            check_data_loaded(controller, "dump");

            let output_path = cmd
                .options
                .get("output")
                .or_else(|| cmd.options.get("o"))
                .and_then(|v| v.as_ref());

            let separator = cmd
                .options
                .get("separator")
                .or_else(|| cmd.options.get("s"))
                .and_then(|v| v.as_ref())
                .and_then(|s| s.chars().next())
                .unwrap_or(',');

            if let Some(batch_size_str) = cmd.options.get("batch_size").and_then(|v| v.as_ref()) {
                match parse_batch_size(batch_size_str) {
                    Ok(batch_size) => {
                        controller.dump_with_batch_size(
                            output_path.map(|s| s.as_str()),
                            separator,
                            batch_size,
                        );
                    }
                    Err(e) => {
                        eprintln!("Error parsing batch_size: {e}");
                        process::exit(1);
                    }
                }
            } else {
                controller.dump(output_path.map(|s| s.as_str()), Some(separator));
            }
        }
        "dumpcache" => {
            check_data_loaded(controller, "dumpcache");
            let output_path = cmd
                .options
                .get("output")
                .and_then(|opt_val| opt_val.as_deref());
            controller.dumpcache(output_path);
        }

        // Unsupported commands
        _ => {
            eprintln!("Error: Unknown command '{}'", cmd.name);
            print_help();
            process::exit(1);
        }
    }
}
