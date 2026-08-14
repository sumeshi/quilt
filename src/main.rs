use std::io::Write;
use std::process;

use controllers::command_model::{parse_typed_commands, render_command_help, render_registry_help};
use controllers::executor::{CommandExecutor, CommandResult};
use quilt::controllers;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format(|buf, record| writeln!(buf, "{}", record.args()))
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 {
        print_registry_help();
        return;
    }
    if args.len() == 2 && (args[1] == "-h" || args[1] == "--help") {
        print_registry_help();
        return;
    }
    if args.len() == 2 && (args[1] == "-v" || args[1] == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.len() >= 3 && (args[2] == "-h" || args[2] == "--help") {
        if let Some(help) = render_command_help(&args[1]) {
            print!("{help}");
        } else {
            print_registry_help();
        }
        return;
    }

    let commands = match parse_typed_commands(&args[1..]) {
        Ok(commands) => commands,
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    };

    let mut executor = CommandExecutor::new();
    match executor.execute_plan(&commands) {
        Ok(CommandResult::CheckValid { path }) => {
            eprintln!("run document '{}' is valid", path.display());
        }
        Ok(_) => {
            let mut stdout = std::io::stdout();
            let mut stderr = std::io::stderr();
            for result in executor.take_finalizer_results() {
                let status = match &result {
                    quilt::operations::finalizers::FinalizerResult::Stderr(_) => {
                        quilt::operations::finalizers::write_stdout(&result, &mut stderr)
                    }
                    _ => quilt::operations::finalizers::write_stdout(&result, &mut stdout),
                };
                match status {
                    Ok(quilt::operations::finalizers::WriteStatus::Complete) => {}
                    Ok(quilt::operations::finalizers::WriteStatus::BrokenPipe) => break,
                    Err(error) => {
                        eprintln!("Error: {error}");
                        process::exit(1);
                    }
                }
            }
        }
        Err(error) => {
            eprintln!("Error: {error}");
            process::exit(1);
        }
    }
}

fn print_registry_help() {
    println!(
        "Quilt: A Rust CLI for processing CSV/TSV, JSONL/NDJSON, and Parquet data with composable pipelines and streaming-capable execution."
    );
    println!("Built for ad-hoc analysis of logs, event exports, and forensic datasets.\n");
    println!("Usage: qlt <initializer> <args> - <chainable> <args> - <finalizer> <args>\n");
    print!("{}", render_registry_help());
    println!("\nIf an option value is '-', use an attached form such as --separator=- or -s-.");
    println!("If a positional value begins with '-', pass -- first.");
    println!("If no finalizer is specified, Quilt uses machine-readable show.");
}
