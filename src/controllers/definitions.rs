//! Declarative command definitions: the single source of truth for public operations.

use super::arguments::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Initializer,
    Chainable,
    Finalizer,
    Automation,
}

#[derive(Debug, Clone, Copy)]
pub struct OptionSpec {
    pub name: &'static str,
    pub short: Option<&'static str>,
    pub takes_value: bool,
    pub repeated: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub category: CommandCategory,
    pub name: &'static str,
    pub min_args: usize,
    pub max_args: Option<usize>,
    pub options: &'static [OptionSpec],
    pub help: &'static str,
}

impl CommandSpec {
    pub const fn run_available(&self) -> bool {
        !matches!(self.category, CommandCategory::Automation)
    }
}

impl TypedCommand {
    pub fn category(&self) -> CommandCategory {
        self.operation_id().category()
    }
    pub fn name(&self) -> &'static str {
        self.operation_id().name()
    }
}

const NONE: &[OptionSpec] = &[];
const DATETIME: &[OptionSpec] = &[
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "timezone",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const BUCKET: &[OptionSpec] = &[
    OptionSpec {
        name: "output",
        short: Some("o"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "timezone",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const LOAD: &[OptionSpec] = &[
    OptionSpec {
        name: "separator",
        short: Some("s"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "low-memory",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "no-headers",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "chunk-size",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "infer-schema-length",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const OUTPUT: &[OptionSpec] = &[OptionSpec {
    name: "output",
    short: Some("o"),
    takes_value: true,
    repeated: false,
}];
const IGNORE: &[OptionSpec] = &[OptionSpec {
    name: "ignore-case",
    short: Some("i"),
    takes_value: false,
    repeated: false,
}];
const SED: &[OptionSpec] = &[
    OptionSpec {
        name: "ignore-case",
        short: Some("i"),
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "column",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const GREP: &[OptionSpec] = &[
    OptionSpec {
        name: "ignore-case",
        short: Some("i"),
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "invert-match",
        short: Some("v"),
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "column",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const NUMBER: &[OptionSpec] = &[OptionSpec {
    name: "number",
    short: Some("n"),
    takes_value: true,
    repeated: false,
}];
const DESC: &[OptionSpec] = &[OptionSpec {
    name: "desc",
    short: Some("d"),
    takes_value: false,
    repeated: false,
}];
const TZ: &[OptionSpec] = &[
    OptionSpec {
        name: "from-tz",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "to-tz",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "output-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const TIMESLICE: &[OptionSpec] = &[
    OptionSpec {
        name: "start",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "end",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "strict",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "input-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "epoch-unit",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "timezone",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "ambiguous",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "nonexistent",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const SHOW: &[OptionSpec] = &[OptionSpec {
    name: "debug",
    short: None,
    takes_value: false,
    repeated: false,
}];
const DUMP: &[OptionSpec] = &[
    OptionSpec {
        name: "output",
        short: Some("o"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "separator",
        short: Some("s"),
        takes_value: true,
        repeated: false,
    },
];
const HEADERS: &[OptionSpec] = &[OptionSpec {
    name: "plain",
    short: Some("p"),
    takes_value: false,
    repeated: false,
}];
const CALC: &[OptionSpec] = &[
    OptionSpec {
        name: "sum",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "avg",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "min",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "max",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "median",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "std",
        short: None,
        takes_value: false,
        repeated: false,
    },
];
const RUN: &[OptionSpec] = &[
    OptionSpec {
        name: "check",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "output",
        short: Some("o"),
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "var",
        short: None,
        takes_value: true,
        repeated: true,
    },
    OptionSpec {
        name: "show-plan",
        short: None,
        takes_value: true,
        repeated: false,
    },
];

macro_rules! define_operations {
    ($( $id:ident($arg:ty) => { name: $name:literal, category: $category:ident, min_args: $min:expr, max_args: $max:expr, options: $options:expr, help: $help:literal },)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum OperationId { $( $id, )+ }
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum TypedCommand { $( $id($arg), )+ }
        impl OperationId {
            pub const fn name(self) -> &'static str {
                match self { $( Self::$id => $name, )+ }
            }
            pub fn parse(name: &str) -> Option<Self> {
                match name { $( $name => Some(Self::$id), )+ _ => None }
            }
            pub const fn category(self) -> CommandCategory {
                match self { $( Self::$id => CommandCategory::$category, )+ }
            }
        }
        impl TypedCommand {
            pub const fn operation_id(&self) -> OperationId {
                match self { $( Self::$id(_) => OperationId::$id, )+ }
            }
        }
        static SPECS: &[CommandSpec] = &[
            $( CommandSpec { category: CommandCategory::$category, name: $name, min_args: $min, max_args: $max, options: $options, help: $help }, )+
        ];
    };
}

define_operations! {
    Load(LoadArgs) => { name: "load", category: Initializer, min_args: 1, max_args: None, options: LOAD, help: "load [files...] [options] — CSV, JSONL/NDJSON, or Parquet" },
    Select(SelectArgs) => { name: "select", category: Chainable, min_args: 1, max_args: None, options: NONE, help: "select <columns>" },
    Cast(CastArgs) => { name: "cast", category: Chainable, min_args: 2, max_args: Some(2), options: DATETIME, help: "cast <column> <type>" },
    Bucket(BucketArgs) => { name: "bucket", category: Chainable, min_args: 2, max_args: Some(2), options: BUCKET, help: "bucket <column> <interval>" },
    Delta(ColumnOutputArgs) => { name: "delta", category: Chainable, min_args: 1, max_args: Some(1), options: OUTPUT, help: "delta <column>" },
    Extract(ExtractArgs) => { name: "extract", category: Chainable, min_args: 2, max_args: Some(2), options: NONE, help: "extract <column> <regex>" },
    Flatten(UnitArgs) => { name: "flatten", category: Chainable, min_args: 0, max_args: Some(0), options: NONE, help: "flatten" },
    ParseSize(ParseSizeArgs) => { name: "parse-size", category: Chainable, min_args: 1, max_args: Some(1), options: NONE, help: "parse-size <column>" },
    Isin(IsinArgs) => { name: "isin", category: Chainable, min_args: 2, max_args: Some(2), options: NONE, help: "isin <column> <values>" },
    Contains(ContainsArgs) => { name: "contains", category: Chainable, min_args: 2, max_args: Some(2), options: IGNORE, help: "contains <column> <pattern>" },
    Sed(SedArgs) => { name: "sed", category: Chainable, min_args: 2, max_args: Some(2), options: SED, help: "sed <pattern> <replacement>" },
    Grep(GrepArgs) => { name: "grep", category: Chainable, min_args: 1, max_args: Some(1), options: GREP, help: "grep <pattern>" },
    Head(NumberArgs) => { name: "head", category: Chainable, min_args: 0, max_args: Some(1), options: NUMBER, help: "head [number]" },
    Tail(NumberArgs) => { name: "tail", category: Chainable, min_args: 0, max_args: Some(1), options: NUMBER, help: "tail [number]" },
    Sort(SortArgs) => { name: "sort", category: Chainable, min_args: 1, max_args: Some(1), options: DESC, help: "sort <columns>" },
    Count(CountArgs) => { name: "count", category: Chainable, min_args: 0, max_args: Some(1), options: NONE, help: "count [columns]" },
    Uniq(UnitArgs) => { name: "uniq", category: Chainable, min_args: 0, max_args: Some(0), options: NONE, help: "uniq" },
    ChangeTz(ChangeTzArgs) => { name: "changetz", category: Chainable, min_args: 1, max_args: Some(1), options: TZ, help: "changetz <column> --from-tz <tz> --to-tz <tz>" },
    RenameCol(RenameColArgs) => { name: "renamecol", category: Chainable, min_args: 2, max_args: Some(2), options: NONE, help: "renamecol <old> <new>" },
    TimeSlice(TimeSliceArgs) => { name: "timeslice", category: Chainable, min_args: 1, max_args: Some(1), options: TIMESLICE, help: "timeslice <column> [--start <time>] [--end <time>]" },
    Show(ShowArgs) => { name: "show", category: Finalizer, min_args: 0, max_args: Some(0), options: SHOW, help: "show" },
    ShowTable(UnitArgs) => { name: "showtable", category: Finalizer, min_args: 0, max_args: Some(0), options: NONE, help: "showtable" },
    Headers(HeadersArgs) => { name: "headers", category: Finalizer, min_args: 0, max_args: Some(0), options: HEADERS, help: "headers [--plain]" },
    Stats(UnitArgs) => { name: "stats", category: Finalizer, min_args: 0, max_args: Some(0), options: NONE, help: "stats" },
    ShowQuery(UnitArgs) => { name: "showquery", category: Finalizer, min_args: 0, max_args: Some(0), options: NONE, help: "showquery" },
    Dump(DumpArgs) => { name: "dump", category: Finalizer, min_args: 0, max_args: Some(0), options: DUMP, help: "dump [options]" },
    DumpCache(DumpCacheArgs) => { name: "dumpcache", category: Finalizer, min_args: 0, max_args: Some(0), options: OUTPUT, help: "dumpcache [--output <file>]" },
    Partition(PartitionArgs) => { name: "partition", category: Finalizer, min_args: 1, max_args: Some(2), options: NONE, help: "partition <column> [directory]" },
    Calc(CalcArgs) => { name: "calc", category: Finalizer, min_args: 1, max_args: Some(1), options: CALC, help: "calc <column> --sum|--avg|--min|--max|--median|--std" },
    Run(RunArgs) => { name: "run", category: Automation, min_args: 1, max_args: None, options: RUN, help: "run <config> [files...] [options]" },
}
pub fn command_specs() -> &'static [CommandSpec] {
    SPECS
}

pub type OperationDefinition = CommandSpec;

pub fn registry() -> &'static [OperationDefinition] {
    SPECS
}

pub fn by_name(name: &str) -> Option<&'static OperationDefinition> {
    registry().iter().find(|definition| definition.name == name)
}

pub fn record_operations() -> impl Iterator<Item = &'static OperationDefinition> {
    registry().iter().filter(|definition| {
        matches!(
            definition.category,
            CommandCategory::Initializer | CommandCategory::Chainable | CommandCategory::Finalizer
        )
    })
}

pub fn automation_operations() -> impl Iterator<Item = &'static OperationDefinition> {
    registry()
        .iter()
        .filter(|definition| definition.category == CommandCategory::Automation)
}

pub const fn option_count(definition: &OperationDefinition) -> usize {
    definition.options.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_one_definition_per_public_operation() {
        let definitions = registry();
        assert!(!definitions.is_empty());
        for definition in definitions {
            assert_eq!(
                definitions
                    .iter()
                    .filter(|candidate| candidate.name == definition.name)
                    .count(),
                1
            );
            assert!(!definition.help.trim().is_empty());
        }
        assert_eq!(
            automation_operations().map(|d| d.name).collect::<Vec<_>>(),
            vec!["run"]
        );
        assert!(record_operations().all(|d| d.name != "run"));
        assert!(registry()
            .iter()
            .all(|d| d.run_available() == (d.name != "run")));
    }

    #[test]
    fn record_registry_is_the_automation_allowlist() {
        let expected = registry()
            .iter()
            .filter(|d| d.category != CommandCategory::Automation)
            .map(|d| d.name)
            .collect::<Vec<_>>();
        assert_eq!(
            record_operations().map(|d| d.name).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn every_definition_has_cli_adapter_and_run_excludes_run() {
        for definition in registry() {
            let mut argv = vec![definition.name.to_string()];
            argv.extend(std::iter::repeat_n(
                "column".to_string(),
                definition.min_args,
            ));
            if definition.name == "changetz" {
                argv.extend([
                    "--from-tz".into(),
                    "UTC".into(),
                    "--to-tz".into(),
                    "UTC".into(),
                ]);
            } else if definition.name == "calc" {
                argv.push("--sum".into());
            } else if definition.name == "timeslice" {
                argv.extend(["--start".into(), "00:00".into()]);
            }
            let parsed = crate::controllers::command_model::parse_typed_commands(&argv).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].name(), definition.name);
            assert_eq!(parsed[0].category(), definition.category);
        }
        let empty = serde_yml::Value::Mapping(serde_yml::Mapping::new());
        assert!(crate::controllers::command_model::parse_automation_step("run", &empty).is_err());
    }

    #[test]
    fn test_only_macro_definition_generates_a_complete_metadata_set() {
        define_operations! {
            TestOnly(UnitArgs) => {
                name: "test-only",
                category: Chainable,
                min_args: 0,
                max_args: Some(0),
                options: NONE,
                help: "test-only"
            },
        }
        assert_eq!(OperationId::TestOnly.name(), "test-only");
        assert_eq!(OperationId::TestOnly.category(), CommandCategory::Chainable);
        let test_command = TypedCommand::TestOnly(UnitArgs);
        assert_eq!(test_command.operation_id(), OperationId::TestOnly);
        assert_eq!(SPECS.len(), 1);
        assert_eq!(SPECS[0].name, OperationId::TestOnly.name());
        assert_eq!(SPECS[0].category, CommandCategory::Chainable);
        assert_eq!(OperationId::parse("test-only"), Some(OperationId::TestOnly));
    }
}
