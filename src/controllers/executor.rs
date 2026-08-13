use super::command_model::{Aggregation, CommandCategory, TypedCommand};
use super::dataframe::DataFrameController;
use crate::error::QuiltError;
use crate::operations;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    None,
    Finalizer(crate::operations::finalizers::FinalizerResult),
    CheckValid { path: std::path::PathBuf },
}

/// Shared typed-command executor used by both the CLI and automation.
#[derive(Default)]
pub struct CommandExecutor {
    controller: DataFrameController,
    results: Vec<crate::operations::finalizers::FinalizerResult>,
}

impl CommandExecutor {
    pub fn new() -> Self {
        Self {
            controller: DataFrameController::new(),
            results: Vec::new(),
        }
    }

    pub fn from_frame(frame: polars::prelude::LazyFrame) -> Self {
        let mut executor = Self::new();
        executor.controller.set_df(frame);
        executor
    }

    pub fn into_frame(self) -> Option<polars::prelude::LazyFrame> {
        self.controller.into_df()
    }

    pub fn set_frame(&mut self, frame: polars::prelude::LazyFrame) {
        self.controller.set_df(frame);
    }

    pub fn is_empty(&self) -> bool {
        self.controller.is_empty()
    }

    pub fn finalizer_results(&self) -> &[crate::operations::finalizers::FinalizerResult] {
        &self.results
    }

    pub fn execute_plan(&mut self, commands: &[TypedCommand]) -> Result<CommandResult, QuiltError> {
        let mut result = CommandResult::None;
        for command in commands {
            result = self.execute(command)?;
        }

        if let Some(last) = commands.last() {
            if matches!(
                last.category(),
                CommandCategory::Initializer | CommandCategory::Chainable
            ) && !self.controller.is_empty()
            {
                // The implicit finalizer is deliberately stable across TTYs
                // and build features: machine-readable `show` is the one
                // automatic output contract.
                if !self.controller.is_empty() {
                    self.results.push(self.controller.show_result()?);
                }
            }
        }
        Ok(result)
    }

    pub fn execute(&mut self, command: &TypedCommand) -> Result<CommandResult, QuiltError> {
        match command {
            TypedCommand::Load(args) => self
                .controller
                .load(
                    &args.paths,
                    &args.separator.to_string(),
                    args.low_memory,
                    args.no_headers,
                    args.chunk_size,
                    args.infer_schema_length,
                )
                .map(|_| ())?,
            TypedCommand::Select(args) => {
                self.require_frame("select")?;
                self.controller.select(&args.columns).map(|_| ())?;
            }
            TypedCommand::Cast(args) => {
                self.require_frame("cast")?;
                self.controller
                    .cast(&args.column, &args.target, &args.datetime)
                    .map(|_| ())?;
            }
            TypedCommand::Bucket(args) => {
                self.require_frame("bucket")?;
                self.controller
                    .bucket(
                        &args.column,
                        &args.interval,
                        args.output.as_deref(),
                        args.datetime.clone(),
                    )
                    .map(|_| ())?;
            }
            TypedCommand::Delta(args) => {
                self.require_frame("delta")?;
                self.controller
                    .delta(&args.column, args.output.as_deref())
                    .map(|_| ())?;
            }
            TypedCommand::Extract(args) => {
                self.require_frame("extract")?;
                self.controller
                    .extract(&args.column, &args.pattern)
                    .map(|_| ())?;
            }
            TypedCommand::Flatten => {
                self.require_frame("flatten")?;
                self.controller.flatten().map(|_| ())?;
            }
            TypedCommand::ParseSize { column } => {
                self.require_frame("parse-size")?;
                self.controller.parse_size(column).map(|_| ())?;
            }
            TypedCommand::Isin(args) => {
                self.require_frame("isin")?;
                self.controller
                    .isin(&args.column, &args.values)
                    .map(|_| ())?;
            }
            TypedCommand::Contains(args) => {
                self.require_frame("contains")?;
                self.controller
                    .contains(&args.column, &args.pattern, args.ignore_case)
                    .map(|_| ())?;
            }
            TypedCommand::Sed(args) => {
                self.require_frame("sed")?;
                self.controller
                    .sed(
                        args.column.as_deref(),
                        &args.pattern,
                        &args.replacement,
                        args.ignore_case,
                    )
                    .map(|_| ())?;
            }
            TypedCommand::Grep(args) => {
                self.require_frame("grep")?;
                self.controller
                    .grep(
                        &args.pattern,
                        args.ignore_case,
                        args.invert_match,
                        args.columns.as_deref(),
                    )
                    .map(|_| ())?;
            }
            TypedCommand::Head(args) => {
                self.require_frame("head")?;
                self.controller.head(args.number).map(|_| ())?;
            }
            TypedCommand::Tail(args) => {
                self.require_frame("tail")?;
                self.controller.tail(args.number).map(|_| ())?;
            }
            TypedCommand::Sort(args) => {
                self.require_frame("sort")?;
                self.controller
                    .sort(&args.columns, args.descending)
                    .map(|_| ())?;
            }
            TypedCommand::Count(args) => {
                self.require_frame("count")?;
                self.controller.count(&args.columns).map(|_| ())?;
            }
            TypedCommand::Uniq => {
                self.require_frame("uniq")?;
                self.controller.uniq().map(|_| ())?;
            }
            TypedCommand::ChangeTz(args) => {
                self.require_frame("changetz")?;
                self.controller
                    .changetz_with_config(
                        &args.column,
                        &args.from_tz,
                        &args.to_tz,
                        args.output_format.as_deref(),
                        crate::operations::datetime::DateTimeConfig {
                            strict: args.strict,
                            input_format: args.input_format.clone(),
                            epoch_unit: args.epoch_unit,
                            timezone: args.timezone.clone(),
                            ambiguous: args.ambiguous,
                            nonexistent: args.nonexistent,
                            options_present: args.options_present,
                        },
                    )
                    .map(|_| ())?;
            }
            TypedCommand::RenameCol { old, new } => {
                self.require_frame("renamecol")?;
                self.controller.renamecol(old, new).map(|_| ())?;
            }
            TypedCommand::TimeSlice(args) => {
                self.require_frame("timeslice")?;
                self.controller
                    .timeslice(
                        &args.column,
                        args.start.as_deref(),
                        args.end.as_deref(),
                        &args.datetime,
                    )
                    .map(|_| ())?;
            }
            TypedCommand::Show { debug } => {
                self.require_frame("show")?;
                if !self.controller.is_empty() {
                    let result = self.controller.show_result()?;
                    self.results.push(if *debug {
                        match result {
                            crate::operations::finalizers::FinalizerResult::Stdout(text) => {
                                crate::operations::finalizers::FinalizerResult::Stderr(text)
                            }
                            other => other,
                        }
                    } else {
                        result
                    });
                }
            }
            TypedCommand::ShowTable => {
                self.require_frame("showtable")?;
                self.results.push(self.controller.showtable_result()?);
            }
            TypedCommand::Headers { plain } => {
                self.require_frame("headers")?;
                self.results.push(self.controller.headers_result(*plain)?);
            }
            TypedCommand::Stats => {
                self.require_frame("stats")?;
                self.results.push(self.controller.stats_result()?);
            }
            TypedCommand::ShowQuery => {
                self.require_frame("showquery")?;
                let result = self.controller.showquery_result()?;
                self.results.push(result);
            }
            TypedCommand::Dump(args) => {
                self.require_frame("dump")?;
                self.results.push(
                    self.controller
                        .dump_result(args.output.as_deref(), args.separator)?,
                );
            }
            TypedCommand::DumpCache { output } => {
                self.require_frame("dumpcache")?;
                self.results
                    .push(self.controller.dumpcache_result(output.as_deref())?);
            }
            TypedCommand::Partition(args) => {
                self.require_frame("partition")?;
                self.results.push(
                    self.controller
                        .partition_result(&args.column, &args.output_dir)?,
                );
            }
            TypedCommand::Calc(args) => {
                self.require_frame("calc")?;
                let mode = match args.aggregation {
                    Aggregation::Sum => "sum",
                    Aggregation::Avg => "avg",
                    Aggregation::Min => "min",
                    Aggregation::Max => "max",
                    Aggregation::Median => "median",
                    Aggregation::Std => "std",
                };
                let result = self.controller.calc_result(&args.column, mode)?;
                self.results.push(result);
            }
            TypedCommand::Run(args) => {
                if let Some(stage) = &args.show_plan {
                    self.results
                        .push(operations::automation::run::run_show_plan(
                            &args.config.to_string_lossy(),
                            stage,
                        )?);
                    let result = self.results.last().cloned().ok_or_else(|| {
                        QuiltError::operation(
                            "run show-plan",
                            "show-plan produced no finalizer result",
                        )
                    })?;
                    return Ok(CommandResult::Finalizer(result));
                }
                let run_results = operations::automation::run::run(
                    &mut self.controller,
                    &args.config.to_string_lossy(),
                    if args.input_files.is_empty() {
                        None
                    } else {
                        Some(args.input_files.clone())
                    },
                    args.output.as_deref(),
                    &args.vars,
                    args.check,
                )?;
                if args.check {
                    return Ok(CommandResult::CheckValid {
                        path: args.config.clone(),
                    });
                }
                self.results.extend(run_results);
            }
        }
        Ok(self
            .results
            .last()
            .cloned()
            .map(CommandResult::Finalizer)
            .unwrap_or(CommandResult::None))
    }

    fn require_frame(&self, command: &str) -> Result<(), QuiltError> {
        if self.controller.is_empty() {
            return Err(QuiltError::usage(format!(
                "Error: No data loaded. Please load data first before using '{command}'."
            )));
        }
        Ok(())
    }
}
