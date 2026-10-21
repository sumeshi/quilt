pub mod controllers;
pub mod error;
pub mod operations;

pub use error::QuiltError;

pub use controllers::pipeline::Pipeline;

#[cfg(test)]
mod tests {
    use super::*;
    use polars::prelude::*;

    #[test]
    fn cast_returns_classified_error_and_successful_chain_remains_usable() {
        let frame = df!("value" => &["1", "2"]).unwrap().lazy();
        let cast = crate::operations::chainables::cast::cast(&frame, "value", "int").unwrap();
        let selected = crate::operations::chainables::head::head(&cast, 1).unwrap();
        assert_eq!(selected.collect().unwrap().height(), 1);

        let bad = df!("value" => &["not-an-int"]).unwrap().lazy();
        let error = crate::operations::chainables::cast::cast(&bad, "value", "int")
            .unwrap()
            .collect()
            .expect_err("invalid conversion should surface at finalization");
        assert!(error.to_string().contains("conversion"));
        assert!(error.to_string().contains("value"));
    }

    #[test]
    fn boundary_validation_returns_classified_errors() {
        let separator_error = match crate::controllers::csv::separator_byte("::") {
            Ok(_) => panic!("multi-character separators must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(separator_error, QuiltError::Usage { .. }));

        let missing_path = crate::operations::initializers::load::load(
            &[std::path::PathBuf::from("/definitely/missing.csv")],
            ",",
            false,
            false,
            None,
            &crate::controllers::resources::ExecutionResources::new(),
        );
        assert!(matches!(missing_path, Err(QuiltError::Io { .. })));

        let frame = df!("when" => &["2024-01-01 00:00:00"]).unwrap().lazy();
        let missing_column =
            crate::operations::chainables::select::select(&frame, &["missing".to_string()]);
        assert!(matches!(missing_column, Err(QuiltError::Schema { .. })));

        let timezone_error = crate::operations::chainables::changetz::changetz(
            &frame,
            "when",
            "Not/A_Timezone",
            "UTC",
            Some("auto"),
            Some("auto"),
            Some("earliest"),
            None,
        );
        assert!(matches!(timezone_error, Err(QuiltError::Usage { .. })));

        let timeslice_error = crate::operations::chainables::timeslice::timeslice(
            &frame,
            "when",
            Some("not-a-time"),
            None,
            &crate::operations::datetime::DateTimeConfig::default(),
        );
        assert!(matches!(
            timeslice_error,
            Err(QuiltError::Conversion { .. })
        ));
    }

    #[test]
    fn finalizers_return_inspectable_results_and_file_errors() {
        let frame = df!("value" => &[1i64, 2, 3], "name" => &["a", "b", "c"])
            .unwrap()
            .lazy();
        let calc = crate::operations::finalizers::calc::calc(&frame, "value", "sum").unwrap();
        assert!(
            matches!(calc, crate::operations::finalizers::FinalizerResult::Scalar(ref value) if value == "6")
        );
        let headers = crate::operations::finalizers::headers::headers(&frame, true).unwrap();
        assert!(
            matches!(headers, crate::operations::finalizers::FinalizerResult::Stdout(ref value) if value == "value\nname\n")
        );
        let show = crate::operations::finalizers::show::show(
            &frame,
            &crate::controllers::resources::ExecutionResources::new(),
        )
        .unwrap();
        assert!(matches!(
            show,
            crate::operations::finalizers::FinalizerResult::Artifact(_)
        ));
        let error = crate::operations::finalizers::dump::dump(&frame, Some("-"), ',');
        assert!(matches!(error, Err(QuiltError::Usage { .. })));
    }

    #[test]
    fn finalizer_atomic_write_cleans_failed_temporary_output() {
        use crate::operations::finalizers::atomic_write;
        use std::fs;

        let root = std::env::temp_dir().join(format!(
            "qlt-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let target = root.join("result.csv");
        let failure = atomic_write(&target, "test atomic write", |_file| {
            Err(QuiltError::finalizer(
                "test atomic write",
                "injected failure",
            ))
        });
        assert!(failure.is_err());
        assert!(!target.exists());
        assert!(fs::read_dir(&root).unwrap().next().is_none());
        fs::remove_dir(&root).unwrap();

        let root = std::env::temp_dir().join(format!(
            "qlt-atomic-race-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let target = root.join("result.csv");
        let race = atomic_write(&target, "test atomic race", |_file| {
            fs::write(&target, "preset\n").unwrap();
            Ok(())
        });
        assert!(race.is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "preset\n");
        fs::remove_file(&target).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn finalizer_atomic_path_allows_one_concurrent_publisher() {
        use crate::operations::finalizers::atomic_path;
        use std::fs;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let root = std::env::temp_dir().join(format!(
            "qlt-atomic-concurrent-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let target = Arc::new(root.join("result.csv"));
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|index| {
                let target = Arc::clone(&target);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    atomic_path(&target, "concurrent atomic path", |temp| {
                        fs::write(temp, format!("payload-{index}\n")).map_err(QuiltError::from)
                    })
                    .is_ok()
                })
            })
            .collect::<Vec<_>>();
        let successes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|success| *success)
            .count();
        assert_eq!(successes, 1);
        let content = fs::read_to_string(&*target).unwrap();
        assert!(content == "payload-0\n" || content == "payload-1\n");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partition_sanitizes_traversal_and_collision_values() {
        use crate::operations::finalizers::partition::partition;
        use std::fs;

        let root = std::env::temp_dir().join(format!(
            "qlt-partition-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let frame = df!(
            "part" => &[Some("a/b"), Some("a\\b"), Some("../escape"), Some("CON"), None::<&str>],
            "value" => &[1i64, 2, 3, 4, 5]
        )
        .unwrap()
        .lazy();
        let result = partition(&frame, "part", root.to_str().unwrap()).unwrap();
        let crate::operations::finalizers::FinalizerResult::Files(files) = result else {
            panic!("partition must return file paths");
        };
        assert_eq!(files.len(), 5);
        for file in &files {
            assert_eq!(file.parent(), Some(root.as_path()));
            assert!(file.exists());
        }
        let names = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(names.iter().any(|name| name == "a_b.csv"));
        assert!(names.iter().any(|name| name == "a_b-2.csv"));
        assert!(names.iter().any(|name| name == "_null.csv"));
        assert!(names.iter().any(|name| name == "_CON.csv"));
        assert!(!root.join("escape.csv").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn typed_command_model_validates_before_execution() {
        use crate::controllers::command_model::parse_typed_commands;
        let args = ["load", "input.csv", "-", "grep", "--", "-starts-with-dash"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let commands = parse_typed_commands(&args).unwrap();
        assert!(
            matches!(commands[0], crate::controllers::command_model::TypedCommand::Load(ref load) if load.paths == vec![std::path::PathBuf::from("input.csv")])
        );
        assert!(
            matches!(commands[1], crate::controllers::command_model::TypedCommand::Grep(ref grep) if grep.pattern == "-starts-with-dash")
        );
        let output_dash = parse_typed_commands(&["dump".into(), "--output=-".into()]).unwrap();
        assert!(
            matches!(output_dash[0], crate::controllers::command_model::TypedCommand::Dump(ref dump) if dump.output.as_deref() == Some("-"))
        );

        let unknown = parse_typed_commands(&["load".into(), "file.csv".into(), "--bogus".into()]);
        assert!(matches!(unknown, Err(QuiltError::Usage { .. })));
        let conflict = parse_typed_commands(&[
            "calc".into(),
            "value".into(),
            "--sum".into(),
            "--avg".into(),
        ]);
        assert!(matches!(conflict, Err(QuiltError::Usage { .. })));
        let literal =
            parse_typed_commands(&["grep".into(), "--".into(), "-starts-with-dash".into()])
                .unwrap();
        assert!(
            matches!(literal[0], crate::controllers::command_model::TypedCommand::Grep(ref grep) if grep.pattern == "-starts-with-dash")
        );
    }

    #[test]
    fn typed_command_payloads_preserve_values_and_flags() {
        use crate::controllers::command_model::{
            parse_typed_commands, Aggregation, ChangeTzArgs, RunArgs, SelectArgs, TypedCommand,
        };

        let commands = parse_typed_commands(
            &[
                "select",
                "col1:col3",
                "-",
                "head",
                "--number",
                "7",
                "-",
                "changetz",
                "when",
                "--from-tz",
                "UTC",
                "--to-tz",
                "Asia/Tokyo",
                "--ambiguous",
                "earliest",
                "-",
                "calc",
                "value",
                "--sum",
                "-",
                "run",
                "config.yaml",
                "input.csv",
                "--output=-",
                "--var",
                "A=1",
                "--var=B=2",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>(),
        )
        .unwrap();

        assert!(matches!(
            &commands[0],
            TypedCommand::Select(SelectArgs { columns })
                if *columns == vec![String::from("col1:col3")]
        ));
        assert!(matches!(
            &commands[1],
            TypedCommand::Head(number) if number.number == 7
        ));
        assert!(matches!(
            &commands[2],
            TypedCommand::ChangeTz(ChangeTzArgs {
                column,
                from_tz,
                to_tz,
                ambiguous,
                ..
            }) if column == "when"
                && from_tz == "UTC"
                && to_tz == "Asia/Tokyo"
                && *ambiguous == crate::operations::datetime::AmbiguousPolicy::Earliest
        ));
        assert!(matches!(
            &commands[3],
            TypedCommand::Calc(calc)
                if calc.column == "value" && calc.aggregation == Aggregation::Sum
        ));
        assert!(matches!(
            &commands[4],
            TypedCommand::Run(RunArgs {
                config,
                input_files,
                output,
                vars,
                check,
                ..
            }) if config.as_path() == std::path::Path::new("config.yaml")
                && *input_files == vec![std::path::PathBuf::from("input.csv")]
                && output.as_deref() == Some("-")
                && *vars == vec!["A=1".to_string(), "B=2".to_string()]
                && !check
        ));

        let flagged = parse_typed_commands(
            &["grep", "--ignore-case", "--invert-match", "needle"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(matches!(
            &flagged[0],
            TypedCommand::Grep(grep) if grep.ignore_case && grep.invert_match
        ));

        let load = parse_typed_commands(
            &[
                "load",
                "input.csv",
                "--separator",
                "|",
                "--low-memory",
                "--no-headers",
                "--chunk-size",
                "42",
            ]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(matches!(
            &load[0],
            TypedCommand::Load(load)
                if load.separator == '|'
                    && load.low_memory
                    && load.no_headers
                    && load.chunk_size == Some(42)
                    && load.infer_schema_length.is_none()
        ));

        let full_inference = parse_typed_commands(
            &["load", "events.ndjson", "--infer-schema-length=full"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(matches!(
            &full_inference[0],
            TypedCommand::Load(load) if load.infer_schema_length.is_none()
        ));
        let run_full = crate::controllers::command_model::parse_automation_step(
            "load",
            &yaml_serde::from_str("paths: [events.ndjson]\ninfer-schema-length: full").unwrap(),
        )
        .unwrap();
        assert!(matches!(
            run_full,
            TypedCommand::Load(load) if load.infer_schema_length.is_none()
        ));

        assert!(matches!(
            parse_typed_commands(&["head".into(), "--number=-1".into()]),
            Err(QuiltError::Usage { .. })
        ));
        for range in ["col3:col1", "col1:other2"] {
            assert!(matches!(
                &parse_typed_commands(&["select".into(), range.into()]).unwrap()[0],
                TypedCommand::Select(SelectArgs { columns }) if columns == &[range]
            ));
        }
    }

    #[test]
    fn datetime_boolean_flags_preserve_value_and_presence() {
        use crate::controllers::command_model::{
            parse_automation_step, parse_typed_commands, TypedCommand,
        };
        use crate::operations::chainables::{bucket, timeslice};

        let parse_cast = |flag: Option<&str>| {
            let mut args = vec!["cast", "when", "datetime"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>();
            if let Some(flag) = flag {
                args.push(flag.to_string());
            }
            parse_typed_commands(&args).unwrap().remove(0)
        };
        let omitted = parse_cast(None);
        let explicit_false = parse_cast(Some("--strict=false"));
        let explicit_true = parse_cast(Some("--strict=true"));
        assert!(matches!(
            omitted,
            TypedCommand::Cast(ref args)
                if !args.datetime.strict && !args.datetime.options_present
        ));
        assert!(matches!(
            explicit_false,
            TypedCommand::Cast(ref args)
                if !args.datetime.strict && args.datetime.options_present
        ));
        assert!(matches!(
            explicit_true,
            TypedCommand::Cast(ref args)
                if args.datetime.strict && args.datetime.options_present
        ));

        let bucket_command = parse_automation_step(
            "bucket",
            &yaml_serde::from_str("column: when\ninterval: 1h\nstrict: false").unwrap(),
        )
        .unwrap();
        let timeslice_command = parse_automation_step(
            "timeslice",
            &yaml_serde::from_str("column: when\nstart: '2024-01-01'\nstrict: false").unwrap(),
        )
        .unwrap();
        let (bucket_datetime, timeslice_datetime) = match (&bucket_command, &timeslice_command) {
            (TypedCommand::Bucket(bucket), TypedCommand::TimeSlice(timeslice)) => {
                (&bucket.datetime, &timeslice.datetime)
            }
            _ => panic!("automation steps must produce typed datetime commands"),
        };
        assert!(!bucket_datetime.strict && bucket_datetime.options_present);
        assert!(!timeslice_datetime.strict && timeslice_datetime.options_present);

        let value =
            chrono::NaiveDateTime::parse_from_str("2024-01-02 03:04:05", "%Y-%m-%d %H:%M:%S")
                .unwrap();
        let frame = df!("when" => &[value]).unwrap().lazy();
        let bucket_error =
            match bucket::bucket_with_config(&frame, "when", "1h", None, bucket_datetime.clone()) {
                Ok(_) => panic!("explicit datetime options must reject typed bucket input"),
                Err(error) => error,
            };
        assert!(bucket_error
            .to_string()
            .contains("parsing options apply only to string input"));
        let timeslice_error = match timeslice::timeslice(
            &frame,
            "when",
            Some("2024-01-01"),
            None,
            timeslice_datetime,
        ) {
            Ok(_) => panic!("explicit datetime options must reject typed timeslice input"),
            Err(error) => error,
        };
        assert!(timeslice_error
            .to_string()
            .contains("parsing options apply only to string timeslice input"));
    }

    #[test]
    fn delta_integral_dtype_precision_and_null_contract() {
        use crate::operations::chainables::delta::delta;
        let high = (1i64 << 53) + 1;
        let frame = df!(
            "signed" => &[high, high + 2, high - 1],
            "unsigned" => &[4u64, 2, 3],
            "nullable" => &[Some(7i64), None, Some(9)]
        )
        .unwrap()
        .lazy();
        let plan = delta(&frame, "signed", None).unwrap();
        let plan = delta(&plan, "unsigned", None).unwrap();
        let plan = delta(&plan, "nullable", None).unwrap();
        let result = plan.collect().unwrap();
        assert_eq!(
            result.column("signed_delta").unwrap().dtype(),
            &DataType::Int64
        );
        assert_eq!(
            result.column("unsigned_delta").unwrap().dtype(),
            &DataType::Int64
        );
        assert_eq!(
            result.column("signed_delta").unwrap().i64().unwrap().get(1),
            Some(2)
        );
        assert_eq!(
            result
                .column("unsigned_delta")
                .unwrap()
                .i64()
                .unwrap()
                .get(1),
            Some(-2)
        );
        assert_eq!(
            result
                .column("nullable_delta")
                .unwrap()
                .i64()
                .unwrap()
                .get(0),
            None
        );
        let float_frame = df!("f32" => &[1.0f32, 2.5], "f64" => &[1.0f64, 2.5])
            .unwrap()
            .lazy();
        let float_frame = delta(&float_frame, "f32", None).unwrap();
        let float_frame = delta(&float_frame, "f64", None).unwrap().collect().unwrap();
        assert_eq!(
            float_frame.column("f32_delta").unwrap().dtype(),
            &DataType::Float32
        );
        assert_eq!(
            float_frame.column("f64_delta").unwrap().dtype(),
            &DataType::Float64
        );
        assert_eq!(
            result
                .column("nullable_delta")
                .unwrap()
                .i64()
                .unwrap()
                .get(1),
            None
        );
        assert_eq!(
            result
                .column("nullable_delta")
                .unwrap()
                .i64()
                .unwrap()
                .get(2),
            None
        );
    }

    #[test]
    fn typed_parser_rejects_before_any_input_io() {
        use crate::controllers::command_model::parse_typed_commands;

        let error = parse_typed_commands(&[
            "load".into(),
            "/definitely/missing.csv".into(),
            "--not-an-option".into(),
        ])
        .expect_err("invalid syntax must fail before loading");
        assert!(matches!(error, QuiltError::Usage { .. }));
        assert!(!error.to_string().contains("File not found"));
    }

    #[test]
    fn cli_and_automation_parse_to_the_same_typed_command() {
        use crate::controllers::command_model::{parse_automation_step, parse_typed_commands};
        use yaml_serde::Value;

        let cli = parse_typed_commands(
            &["grep", "--ignore-case", "needle"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let yaml: Value = yaml_serde::from_str("{pattern: needle, ignore-case: true}").unwrap();
        let automation = parse_automation_step("grep", &yaml).unwrap();
        assert_eq!(cli, vec![automation]);
    }

    #[test]
    fn executor_preserves_finalizer_result_order_and_does_not_print() {
        use crate::controllers::command_model::TypedCommand;
        use crate::controllers::executor::CommandExecutor;

        let frame = df!("value" => &[1i64, 2]).unwrap().lazy();
        let mut executor = CommandExecutor::from_frame(frame);
        executor
            .execute(&TypedCommand::Headers(
                crate::controllers::command_model::HeadersArgs { plain: true },
            ))
            .unwrap();
        executor
            .execute(&TypedCommand::Show(
                crate::controllers::command_model::ShowArgs { debug: false },
            ))
            .unwrap();
        assert_eq!(executor.finalizer_results().len(), 2);
        assert!(matches!(
            &executor.finalizer_results()[0],
            crate::operations::finalizers::FinalizerResult::Stdout(text)
                if text == "value\n"
        ));
        let mut rendered = Vec::new();
        crate::operations::finalizers::write_stdout(
            &executor.finalizer_results()[1],
            &mut rendered,
        )
        .unwrap();
        assert!(String::from_utf8(rendered)
            .unwrap()
            .starts_with("value\n1\n"));
    }

    #[test]
    fn delta_rejects_uint64_values_outside_lossless_int64_policy() {
        use crate::operations::chainables::delta::delta;
        let frame = df!("value" => &[0u64, u64::MAX]).unwrap().lazy();
        let error = delta(&frame, "value", None).unwrap().collect().unwrap_err();
        assert!(error.to_string().contains("Int64") || error.to_string().contains("conversion"));
    }

    #[test]
    fn bucket_preserves_datetime_units_and_floors_negative_values() {
        use crate::operations::chainables::bucket::{bucket, validate_interval};
        let value = chrono::NaiveDateTime::parse_from_str(
            "1969-12-31 23:59:59.999999",
            "%Y-%m-%d %H:%M:%S%.f",
        )
        .unwrap();
        for unit in [
            TimeUnit::Nanoseconds,
            TimeUnit::Microseconds,
            TimeUnit::Milliseconds,
        ] {
            let source =
                DatetimeChunked::from_naive_datetime("when".into(), [value], unit).into_series();
            let frame = DataFrame::new_infer_height(vec![source.into()])
                .unwrap()
                .lazy();
            let result = bucket(&frame, "when", "1s", None)
                .unwrap()
                .collect()
                .unwrap();
            assert_eq!(
                result.column("when_bucket").unwrap().dtype(),
                &DataType::Datetime(unit, None)
            );
            assert_eq!(result.height(), 1);
            let raw = result
                .column("when_bucket")
                .unwrap()
                .datetime()
                .unwrap()
                .physical()
                .get(0);
            let expected = match unit {
                TimeUnit::Nanoseconds => -1_000_000_000,
                TimeUnit::Microseconds => -1_000_000,
                TimeUnit::Milliseconds => -1_000,
            };
            assert_eq!(raw, Some(expected));
        }
        let mut timezone_source =
            DatetimeChunked::from_naive_datetime("when".into(), [value], TimeUnit::Microseconds);
        timezone_source.set_time_zone(TimeZone::UTC).unwrap();
        let timezone_frame =
            DataFrame::new_infer_height(vec![timezone_source.into_series().into()])
                .unwrap()
                .lazy();
        let timezone_result = bucket(&timezone_frame, "when", "1s", None)
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(
            timezone_result.column("when_bucket").unwrap().dtype(),
            &DataType::Datetime(TimeUnit::Microseconds, Some(TimeZone::UTC))
        );

        let nullable = DatetimeChunked::from_naive_datetime_options(
            "when".into(),
            [Some(value), None],
            TimeUnit::Milliseconds,
        );
        let nullable_frame = DataFrame::new_infer_height(vec![nullable.into_series().into()])
            .unwrap()
            .lazy();
        let nullable_result = bucket(&nullable_frame, "when", "1s", None)
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(
            nullable_result
                .column("when_bucket")
                .unwrap()
                .datetime()
                .unwrap()
                .physical()
                .get(0),
            Some(-1_000)
        );
        assert_eq!(
            nullable_result
                .column("when_bucket")
                .unwrap()
                .datetime()
                .unwrap()
                .physical()
                .get(1),
            None
        );

        assert!(validate_interval("999999999999999999999999s").is_err());
        let overflow = bucket(&nullable_frame, "when", "999999999999999999999999s", None);
        assert!(overflow.is_err());
        let nanosecond_overflow = bucket(
            &DataFrame::new_infer_height(vec![DatetimeChunked::from_naive_datetime(
                "when".into(),
                [value],
                TimeUnit::Nanoseconds,
            )
            .into_series()
            .into()])
            .unwrap()
            .lazy(),
            "when",
            "999999999999999999999999s",
            None,
        );
        assert!(nanosecond_overflow.is_err());

        let collision_frame = df!(
            "when" => &[value],
            "when_bucket" => &[value]
        )
        .unwrap()
        .lazy();
        let collision = bucket(&collision_frame, "when", "1s", None);
        assert!(collision.is_err());
    }

    #[test]
    fn showquery_plan_tests_keep_each_mandatory_operation_pending() {
        use crate::operations::chainables::{bucket, cast, delta, extract, flatten, parse_size};
        let datetime =
            chrono::NaiveDateTime::parse_from_str("2024-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")
                .unwrap();
        let frame = df!(
            "value" => &["1", "2"],
            "number" => &[1i64, 2],
            "size" => &["1KB", "2KB"],
            "message" => &["id=one", "id=two"],
            "when" => &[datetime, datetime]
        )
        .unwrap()
        .lazy();
        let cases = [
            ("cast", cast::cast(&frame, "value", "int").unwrap()),
            (
                "parse-size",
                parse_size::parse_size_column(&frame, "size").unwrap(),
            ),
            (
                "bucket",
                bucket::bucket(&frame, "when", "1d", None).unwrap(),
            ),
            ("delta", delta::delta(&frame, "number", None).unwrap()),
            (
                "extract",
                extract::extract(&frame, "message", r"id=(?P<id>\w+)").unwrap(),
            ),
            ("flatten", flatten::flatten(&frame).unwrap()),
        ];
        for (name, plan) in cases {
            let description = plan.describe_plan().unwrap();
            assert!(
                description.contains("WITH_COLUMNS") || description.contains("SELECT"),
                "{name} plan lost its pending transformation: {description}"
            );
            assert!(
                !description.contains("DataFrame"),
                "{name} forced eager materialization"
            );
        }
    }

    #[test]
    fn ndjson_inference_default_is_full_and_bounded_rejects_late_fields() {
        use crate::operations::initializers::load::{
            load, load_with_ndjson_inference_with_resources,
        };
        let resources = crate::controllers::resources::ExecutionResources::new();
        use std::fs;
        let path = std::env::temp_dir().join(format!(
            "qlt-ndjson-{}-{}.jsonl",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut contents = String::new();
        for _ in 0..1_001 {
            contents.push_str("{\"id\":1}\n");
        }
        contents.push_str("{\"id\":1,\"late\":true}\n");
        fs::write(&path, contents).unwrap();
        let default_schema = load(
            std::slice::from_ref(&path),
            ",",
            false,
            false,
            None,
            &resources,
        )
        .unwrap()
        .collect_schema()
        .unwrap();
        let full_schema = load_with_ndjson_inference_with_resources(
            std::slice::from_ref(&path),
            ",",
            false,
            false,
            None,
            None,
            None,
            &resources,
        )
        .unwrap()
        .collect_schema()
        .unwrap();
        let bounded = load_with_ndjson_inference_with_resources(
            std::slice::from_ref(&path),
            ",",
            false,
            false,
            None,
            Some(1_000),
            None,
            &resources,
        );
        assert!(bounded
            .err()
            .unwrap()
            .to_string()
            .contains("inference limit"));
        fs::remove_file(path).unwrap();
        assert_eq!(default_schema.get("late"), Some(&DataType::Boolean));
        assert_eq!(full_schema.get("late"), Some(&DataType::Boolean));
    }
}
