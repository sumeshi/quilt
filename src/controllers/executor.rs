use super::command_model::{
    Aggregation, CommandCategory, DumpCacheArgs, HeadersArgs, ParseSizeArgs, RenameColArgs,
    ShowArgs, TypedCommand,
};
use super::pipeline::{Pipeline, PipelineState};
use super::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    None,
    Finalizer(crate::operations::finalizers::FinalizerResult),
    CheckValid { path: std::path::PathBuf },
}

/// Shared typed-command executor used by both the CLI and automation.
pub struct CommandExecutor {
    controller: PipelineState,
    results: Vec<crate::operations::finalizers::FinalizerResult>,
}

impl Default for CommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandExecutor {
    pub fn new() -> Self {
        Self::new_with_resources(ExecutionResources::new())
    }

    pub fn new_with_resources(resources: ExecutionResources) -> Self {
        Self {
            controller: PipelineState::empty(resources),
            results: Vec::new(),
        }
    }

    pub fn from_frame(frame: polars::prelude::LazyFrame) -> Self {
        Self::from_frame_with_resources(frame, ExecutionResources::new())
    }

    pub fn from_frame_with_resources(
        frame: polars::prelude::LazyFrame,
        resources: ExecutionResources,
    ) -> Self {
        Self {
            controller: PipelineState::Loaded(Box::new(Pipeline::new(frame, resources))),
            results: Vec::new(),
        }
    }

    pub fn into_pipeline(self) -> Option<Pipeline> {
        self.controller.into_pipeline()
    }

    pub fn finalizer_results(&self) -> &[crate::operations::finalizers::FinalizerResult] {
        &self.results
    }

    /// Transfer finalizer results to the output boundary. Consuming the
    /// vector avoids retaining already-emitted result values (and drops any
    /// remaining results together when output terminates early).
    pub fn take_finalizer_results(
        &mut self,
    ) -> Vec<crate::operations::finalizers::FinalizerResult> {
        std::mem::take(&mut self.results)
    }

    pub fn resources(&self) -> ExecutionResources {
        self.controller.resources()
    }

    pub fn execute_plan(&mut self, commands: &[TypedCommand]) -> Result<CommandResult, QuiltError> {
        let mut finalizer_seen = false;
        for (index, command) in commands.iter().enumerate() {
            if command.category() == CommandCategory::Automation && commands.len() != 1 {
                return Err(QuiltError::usage(
                    "Error: automation command 'run' cannot be combined with pipeline commands",
                ));
            }
            if index > 0 && command.category() == CommandCategory::Initializer {
                return Err(QuiltError::usage(format!(
                    "Error: initializer '{}' must be the first pipeline command",
                    command.name()
                )));
            }
            if finalizer_seen && command.category() != CommandCategory::Finalizer {
                return Err(QuiltError::usage(format!(
                    "Error: command '{}' cannot follow a finalizer",
                    command.name()
                )));
            }
            finalizer_seen |= command.category() == CommandCategory::Finalizer;
        }

        let mut result = CommandResult::None;
        for command in commands {
            result = self.execute(command)?;
        }

        if let Some(last) = commands.last() {
            if matches!(
                last.category(),
                CommandCategory::Initializer | CommandCategory::Chainable
            ) && matches!(self.controller, PipelineState::Loaded(_))
            {
                // The implicit finalizer is deliberately stable across TTYs
                // and build features: machine-readable `show` is the one
                // automatic output contract.
                self.results
                    .push(self.controller.loaded("show")?.show_result()?);
            }
        }
        Ok(result)
    }

    pub fn execute(&mut self, command: &TypedCommand) -> Result<CommandResult, QuiltError> {
        match command {
            TypedCommand::Load(args) => {
                let resources = self.controller.resources();
                let loaded = Pipeline::load(
                    &args.paths,
                    &args.separator.to_string(),
                    args.low_memory,
                    args.no_headers,
                    args.chunk_size,
                    args.infer_schema_length,
                    resources,
                )?;
                self.controller = PipelineState::Loaded(Box::new(loaded));
            }
            TypedCommand::Select(args) => {
                self.controller
                    .loaded_mut("select")?
                    .select(&args.columns)?;
            }
            TypedCommand::Cast(args) => {
                self.controller.loaded_mut("cast")?.cast(
                    &args.column,
                    &args.target,
                    &args.datetime,
                )?;
            }
            TypedCommand::Bucket(args) => {
                self.controller.loaded_mut("bucket")?.bucket(
                    &args.column,
                    &args.interval,
                    args.output.as_deref(),
                    args.datetime.clone(),
                )?;
            }
            TypedCommand::Delta(args) => {
                self.controller
                    .loaded_mut("delta")?
                    .delta(&args.column, args.output.as_deref())?;
            }
            TypedCommand::Extract(args) => {
                self.controller
                    .loaded_mut("extract")?
                    .extract(&args.column, &args.pattern)?;
            }
            TypedCommand::Flatten(_) => {
                self.controller.loaded_mut("flatten")?.flatten()?;
            }
            TypedCommand::ParseSize(ParseSizeArgs { column }) => {
                self.controller
                    .loaded_mut("parse-size")?
                    .parse_size(column)?;
            }
            TypedCommand::Isin(args) => {
                self.controller
                    .loaded_mut("isin")?
                    .isin(&args.column, &args.values)?;
            }
            TypedCommand::Contains(args) => {
                self.controller.loaded_mut("contains")?.contains(
                    &args.column,
                    &args.pattern,
                    args.ignore_case,
                )?;
            }
            TypedCommand::Sed(args) => {
                self.controller.loaded_mut("sed")?.sed(
                    args.column.as_deref(),
                    &args.pattern,
                    &args.replacement,
                    args.ignore_case,
                )?;
            }
            TypedCommand::Grep(args) => {
                self.controller.loaded_mut("grep")?.grep(
                    &args.pattern,
                    args.ignore_case,
                    args.invert_match,
                    args.columns.as_deref(),
                )?;
            }
            TypedCommand::Head(args) => {
                self.controller.loaded_mut("head")?.head(args.number)?;
            }
            TypedCommand::Tail(args) => {
                self.controller.loaded_mut("tail")?.tail(args.number)?;
            }
            TypedCommand::Sort(args) => {
                self.controller
                    .loaded_mut("sort")?
                    .sort(&args.columns, args.descending)?;
            }
            TypedCommand::Count(args) => {
                self.controller.loaded_mut("count")?.count(&args.columns)?;
            }
            TypedCommand::Uniq(_) => {
                self.controller.loaded_mut("uniq")?.uniq()?;
            }
            TypedCommand::ChangeTz(args) => {
                self.controller.loaded_mut("changetz")?.changetz(
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
                )?;
            }
            TypedCommand::RenameCol(RenameColArgs { old, new }) => {
                self.controller
                    .loaded_mut("renamecol")?
                    .renamecol(old, new)?;
            }
            TypedCommand::TimeSlice(args) => {
                self.controller.loaded_mut("timeslice")?.timeslice(
                    &args.column,
                    args.start.as_deref(),
                    args.end.as_deref(),
                    &args.datetime,
                )?;
            }
            TypedCommand::Show(ShowArgs { debug }) => {
                let result = self.controller.loaded("show")?.show_result()?;
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
            TypedCommand::ShowTable(_) => {
                self.results
                    .push(self.controller.loaded("showtable")?.showtable_result()?);
            }
            TypedCommand::Headers(HeadersArgs { plain }) => {
                self.results
                    .push(self.controller.loaded("headers")?.headers_result(*plain)?);
            }
            TypedCommand::Stats(_) => {
                self.results
                    .push(self.controller.loaded("stats")?.stats_result()?);
            }
            TypedCommand::ShowQuery(_) => {
                let result = self.controller.loaded("showquery")?.showquery_result()?;
                self.results.push(result);
            }
            TypedCommand::Dump(args) => {
                self.results.push(
                    self.controller
                        .loaded("dump")?
                        .dump_result(args.output.as_deref(), args.separator)?,
                );
            }
            TypedCommand::DumpCache(DumpCacheArgs { output }) => {
                self.results.push(
                    self.controller
                        .loaded("dumpcache")?
                        .dumpcache_result(output.as_deref())?,
                );
            }
            TypedCommand::Partition(args) => {
                self.results.push(
                    self.controller
                        .loaded("partition")?
                        .partition_result(&args.column, &args.output_dir)?,
                );
            }
            TypedCommand::Calc(args) => {
                let mode = match args.aggregation {
                    Aggregation::Sum => "sum",
                    Aggregation::Avg => "avg",
                    Aggregation::Min => "min",
                    Aggregation::Max => "max",
                    Aggregation::Median => "median",
                    Aggregation::Std => "std",
                };
                let result = self
                    .controller
                    .loaded("calc")?
                    .calc_result(&args.column, mode)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::command_model::{CastArgs, DumpArgs, LoadArgs};
    use polars::df;
    use polars::prelude::IntoLazy;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample-min.csv.gz"
        ))
    }

    fn temp_root(label: &str) -> PathBuf {
        for nonce in 0..128u32 {
            let path = std::env::temp_dir()
                .join(format!("qlt-t14-{label}-{}-{nonce}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create test directory {path:?}: {error}"),
            }
        }
        panic!("could not reserve unique test directory for {label}");
    }

    fn load_command() -> TypedCommand {
        TypedCommand::Load(LoadArgs {
            paths: vec![fixture()],
            separator: ',',
            low_memory: false,
            no_headers: false,
            chunk_size: None,
            infer_schema_length: Some(1_000),
        })
    }

    #[test]
    fn unloaded_chainable_and_finalizer_operations_share_usage_error() {
        let mut commands = Vec::new();
        for tokens in [
            vec!["select", "id"],
            vec!["cast", "id", "int"],
            vec!["bucket", "id", "1d"],
            vec!["delta", "id"],
            vec!["extract", "id", "x"],
            vec!["flatten"],
            vec!["parse-size", "id"],
            vec!["isin", "id", "x"],
            vec!["contains", "id", "x"],
            vec!["sed", "id", "x"],
            vec!["timeslice", "id", "--start", "00:00"],
            vec!["grep", "x"],
            vec!["head", "1"],
            vec!["tail", "1"],
            vec!["sort", "id"],
            vec!["count", "id"],
            vec!["uniq"],
            vec!["renamecol", "id", "new"],
            vec!["show"],
            vec!["showtable"],
            vec!["headers"],
            vec!["stats"],
            vec!["showquery"],
            vec!["dump"],
            vec!["dumpcache"],
            vec!["partition", "id"],
            vec!["calc", "id", "--sum"],
        ] {
            let args = tokens.into_iter().map(String::from).collect::<Vec<_>>();
            let parsed = crate::controllers::command_model::parse_typed_commands(&args)
                .unwrap_or_else(|error| panic!("failed to parse {args:?}: {error}"));
            commands.push(parsed.into_iter().next().unwrap());
        }
        commands.push(TypedCommand::ChangeTz(
            crate::controllers::command_model::ChangeTzArgs {
                column: "id".into(),
                from_tz: "UTC".into(),
                to_tz: "UTC".into(),
                input_format: None,
                output_format: None,
                ambiguous: crate::operations::datetime::AmbiguousPolicy::Error,
                nonexistent: crate::operations::datetime::NonexistentPolicy::Error,
                strict: false,
                epoch_unit: None,
                timezone: None,
                options_present: false,
            },
        ));
        for command in commands {
            let mut executor = CommandExecutor::new();
            let error = match executor.execute(&command) {
                Ok(_) => panic!("command {:?} unexpectedly succeeded", command),
                Err(error) => error,
            };
            assert!(matches!(error, QuiltError::Usage { .. }), "{error}");
            assert!(
                error.to_string().contains("No data loaded"),
                "command {:?}: {error}",
                command
            );
        }
    }

    #[test]
    fn failed_load_preserves_empty_state_resources() {
        let resources = ExecutionResources::new();
        let mut executor = CommandExecutor::new_with_resources(resources.clone());
        let error = executor
            .execute(&TypedCommand::Load(LoadArgs {
                paths: vec![PathBuf::from("/definitely/missing.csv")],
                separator: ',',
                low_memory: false,
                no_headers: false,
                chunk_size: None,
                infer_schema_length: Some(1000),
            }))
            .unwrap_err();
        assert!(matches!(error, QuiltError::Io { .. }));
        assert!(executor.results.is_empty());
        let error = executor
            .execute(&TypedCommand::Head(
                crate::controllers::command_model::NumberArgs { number: 1 },
            ))
            .unwrap_err();
        assert!(matches!(error, QuiltError::Usage { .. }));
        assert_eq!(
            executor.resources().tracked_count(),
            resources.tracked_count()
        );
    }

    #[test]
    fn load_chain_finalize_transitions_to_loaded_pipeline() {
        let mut executor = CommandExecutor::new();
        executor.execute(&load_command()).unwrap();
        executor
            .execute(&TypedCommand::Head(
                crate::controllers::command_model::NumberArgs { number: 1 },
            ))
            .unwrap();
        assert!(matches!(
            executor.execute(&TypedCommand::Show(ShowArgs { debug: false })),
            Ok(CommandResult::Finalizer(_))
        ));
        assert!(executor.into_pipeline().is_some());
    }

    #[test]
    fn cloned_pipeline_keeps_managed_resource_until_last_clone() {
        let root = temp_root("pipeline-clone");
        let resources = ExecutionResources::new_in(root.clone());
        let reservation = resources.reserve_temp_file("clone", "tmp").unwrap();
        let path = reservation.path().to_path_buf();
        resources.retain_temp_file(reservation).unwrap();
        let pipeline = crate::controllers::pipeline::Pipeline::new(
            df!("id" => &[1i64]).unwrap().lazy(),
            resources.clone(),
        );
        let clone = pipeline.clone();
        drop(pipeline);
        drop(resources);
        assert!(path.exists());
        drop(clone);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn gzip_spool_survives_lazy_conversion_failure_until_executor_drop() {
        let root = temp_root("conversion");
        let resources = ExecutionResources::new_in(root.clone());
        let mut executor = CommandExecutor::new_with_resources(resources.clone());
        executor.execute(&load_command()).unwrap();
        let spool = resources.tracked_paths();
        assert_eq!(spool.len(), 1);
        assert!(spool[0].exists());
        executor
            .execute(&TypedCommand::Cast(CastArgs {
                column: "Level".into(),
                target: "int".into(),
                datetime: Default::default(),
            }))
            .unwrap();
        assert!(executor
            .execute(&TypedCommand::Show(ShowArgs { debug: false }))
            .is_err());
        assert!(spool[0].exists());
        drop(executor);
        drop(resources);
        assert!(!spool[0].exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn gzip_spool_survives_sink_failure_until_executor_drop() {
        let root = temp_root("sink");
        let resources = ExecutionResources::new_in(root.clone());
        let mut executor = CommandExecutor::new_with_resources(resources.clone());
        executor.execute(&load_command()).unwrap();
        let spool = resources.tracked_paths();
        let target = root.join("destination");
        std::fs::create_dir(&target).unwrap();
        assert!(executor
            .execute(&TypedCommand::Dump(DumpArgs {
                output: Some(target.to_string_lossy().into_owned()),
                separator: ',',
            }))
            .is_err());
        assert!(spool[0].exists());
        drop(executor);
        drop(resources);
        assert!(!spool[0].exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn truncated_gzip_reservation_is_removed_before_returning_error() {
        let root = temp_root("truncated");
        let source = root.join("broken.csv.gz");
        std::fs::write(&source, b"not a gzip stream").unwrap();
        let resources = ExecutionResources::new_in(root.clone());
        let result = CommandExecutor::new_with_resources(resources.clone()).execute(
            &TypedCommand::Load(LoadArgs {
                paths: vec![source],
                separator: ',',
                low_memory: false,
                no_headers: false,
                chunk_size: None,
                infer_schema_length: Some(1_000),
            }),
        );
        assert!(result.is_err());
        assert!(resources.tracked_paths().is_empty());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        drop(resources);
        let _ = std::fs::remove_dir_all(root);
    }
}
