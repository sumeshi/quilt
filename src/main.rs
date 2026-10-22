use std::io::Write;
use std::process::ExitCode;

use controllers::command_model::{parse_typed_commands, render_command_help, render_registry_help};
use controllers::executor::{CommandExecutor, CommandResult};
use quilt::controllers;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format(|buf, record| writeln!(buf, "{}", record.args()))
        .init();

    run()
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--lazy") {
        if args.len() == 3 && matches!(args[2].as_str(), "-h" | "--help") {
            println!(
                r#"Usage: qlt --lazy [load options] [files...]

Interactive pipeline editor with live previews and completion.

Enter applies a command; Tab completes commands, options, and columns.
Ctrl+A shows available shortcuts; Esc cancels the prefix.
Ctrl+A actions: ? help, c columns, w YAML, o export, C checkpoint, x clear,
p sample/full, u/r undo/redo, z cancel, q quit.
t/b/i focus table/pipeline/input; f expands or shrinks the focused pane; h command help;
e error details; d row details. Ctrl-C exits.
Table: arrows or hjkl move, Enter opens cell details, Esc focuses input.
Pipeline: arrows or j/k move, Enter edits a source group, Esc focuses input.
Details: arrows or j/k scroll, PgUp/PgDn page, Home/End jump, w toggles wrap,
Esc closes. Column picker: Space toggles, arrows or hjkl move, / filters,
a selects filtered columns, i inverts, u clears, Enter applies, Esc cancels.
Detail text is capped at 256 KiB and marked when truncated.

Use :edit STEP to reopen its original command group (the step range is shown),
:write PATH, :export PATH [--format csv|jsonl],
:cache [PATH], :clear-cache, :undo, :redo, or :quit. Export writes the full
confirmed result; .jsonl/.ndjson paths select JSONL, other paths select CSV.
Exports do not overwrite existing files. Busy work shows elapsed time; cancel
is best effort and may take time to finish.

Requires an interactive terminal. Preview samples up to 10,000 input rows;
full preview applies the pipeline to all input data."#
            );
            return ExitCode::SUCCESS;
        }
        log::set_max_level(log::LevelFilter::Off);
        return match quilt::tui::run(&args[2..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args.len() == 1 {
        print_registry_help();
        return ExitCode::SUCCESS;
    }
    if args.len() == 2 && (args[1] == "-h" || args[1] == "--help") {
        print_registry_help();
        return ExitCode::SUCCESS;
    }
    if args.len() == 2 && (args[1] == "-v" || args[1] == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.len() >= 3 && (args[2] == "-h" || args[2] == "--help") {
        if let Some(help) = render_command_help(&args[1]) {
            print!("{help}");
        } else {
            print_registry_help();
        }
        return ExitCode::SUCCESS;
    }

    let commands = match parse_typed_commands(&args[1..]) {
        Ok(commands) => commands,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let mut executor = CommandExecutor::new();
    match executor.execute_plan(&commands) {
        Ok(CommandResult::CheckValid { path }) => {
            eprintln!("run document '{}' is valid", path.display());
        }
        Ok(_) => {
            let mut stdout = std::io::stdout();
            for result in executor.take_finalizer_results() {
                let status = quilt::operations::finalizers::write_stdout(&result, &mut stdout);
                match status {
                    Ok(quilt::operations::finalizers::WriteStatus::Complete) => {}
                    Ok(quilt::operations::finalizers::WriteStatus::BrokenPipe) => break,
                    Err(error) => {
                        eprintln!("Error: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
        Err(error) => {
            eprintln!("Error: {error}");
            return ExitCode::FAILURE;
        }
    }

    // Returning an exit code lets executor-owned resources drop on every path.
    ExitCode::SUCCESS
}

fn print_registry_help() {
    println!(
        "Quilt: A Rust CLI for processing CSV/TSV, JSONL/NDJSON, and Parquet data with composable pipelines and streaming-capable execution."
    );
    println!("Built for ad-hoc analysis of logs, event exports, and forensic datasets.\n");
    println!("Usage: qlt <initializer> <args> - <chainable> <args> - <finalizer> <args>\n");
    println!("       qlt --lazy [load options] [files...]\n");
    print!("{}", render_registry_help());
    println!("\nIf an option value is '-', use an attached form such as --separator=- or -s-.");
    println!("If a positional value begins with '-', pass -- first.");
    println!("If no finalizer is specified, Quilt uses machine-readable show.");
}
