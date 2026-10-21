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
    pub max_args: Option<usize>,
    pub options: &'static [OptionSpec],
    pub help: &'static str,
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
        name: "source-column",
        short: None,
        takes_value: true,
        repeated: false,
    },
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
    OptionSpec {
        name: "all-strings",
        short: None,
        takes_value: false,
        repeated: false,
    },
    OptionSpec {
        name: "schema",
        short: None,
        takes_value: true,
        repeated: true,
    },
    OptionSpec {
        name: "stdin-format",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "rejects",
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
const DELTA: &[OptionSpec] = &[
    OptionSpec {
        name: "output",
        short: None,
        takes_value: true,
        repeated: false,
    },
    OptionSpec {
        name: "by",
        short: None,
        takes_value: true,
        repeated: false,
    },
];
const TIMEINFO: &[OptionSpec] = &[OptionSpec {
    name: "by",
    short: None,
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
    name: "format",
    short: None,
    takes_value: true,
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
    OptionSpec {
        name: "format",
        short: None,
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
        name: "stage",
        short: None,
        takes_value: true,
        repeated: false,
    },
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
    ($( $id:ident($arg:ty) => { name: $name:literal, category: $category:ident, max_args: $max:expr, options: $options:expr, help: $help:literal },)+) => {
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
            $( CommandSpec { category: CommandCategory::$category, name: $name, max_args: $max, options: $options, help: $help }, )+
        ];
    };
}

define_operations! {
    Load(LoadArgs) => { name: "load", category: Initializer, max_args: None, options: LOAD, help: "load [files...] [options] — CSV, JSONL/NDJSON, or Parquet" },
    Select(SelectArgs) => { name: "select", category: Chainable, max_args: None, options: NONE, help: "select <columns>" },
    Filter(FilterArgs) => { name: "filter", category: Chainable, max_args: Some(1), options: NONE, help: "filter <expression>" },
    Derive(DeriveArgs) => { name: "derive", category: Chainable, max_args: Some(2), options: NONE, help: "derive <output> <expression>" },
    Aggregate(AggregateArgs) => { name: "aggregate", category: Chainable, max_args: None, options: TIMEINFO, help: "aggregate <mode:column:output...> [--by columns]" },
    ArrayContains(ArrayContainsArgs) => { name: "array-contains", category: Chainable, max_args: Some(2), options: NONE, help: "array-contains <column> <value>" },
    ArrayGet(ArrayGetArgs) => { name: "array-get", category: Chainable, max_args: Some(2), options: OUTPUT, help: "array-get <column> <index> [--output name]" },
    Explode(ExplodeArgs) => { name: "explode", category: Chainable, max_args: Some(1), options: NONE, help: "explode <columns>" },
    Cast(CastArgs) => { name: "cast", category: Chainable, max_args: Some(2), options: DATETIME, help: "cast <column> <type>" },
    Bucket(BucketArgs) => { name: "bucket", category: Chainable, max_args: Some(2), options: BUCKET, help: "bucket <column> <interval>" },
    Delta(ColumnOutputArgs) => { name: "delta", category: Chainable, max_args: Some(1), options: DELTA, help: "delta <column>" },
    Extract(ExtractArgs) => { name: "extract", category: Chainable, max_args: Some(2), options: NONE, help: "extract <column> <regex>" },
    Flatten(UnitArgs) => { name: "flatten", category: Chainable, max_args: Some(0), options: NONE, help: "flatten" },
    ParseSize(ParseSizeArgs) => { name: "parse-size", category: Chainable, max_args: Some(1), options: NONE, help: "parse-size <column>" },
    Isin(IsinArgs) => { name: "isin", category: Chainable, max_args: Some(2), options: NONE, help: "isin <column> <values>" },
    Contains(ContainsArgs) => { name: "contains", category: Chainable, max_args: Some(2), options: IGNORE, help: "contains <column> <pattern>" },
    Sed(SedArgs) => { name: "sed", category: Chainable, max_args: Some(2), options: SED, help: "sed <pattern> <replacement>" },
    Grep(GrepArgs) => { name: "grep", category: Chainable, max_args: Some(1), options: GREP, help: "grep <pattern>" },
    Head(NumberArgs) => { name: "head", category: Chainable, max_args: Some(1), options: NUMBER, help: "head [number]" },
    Tail(NumberArgs) => { name: "tail", category: Chainable, max_args: Some(1), options: NUMBER, help: "tail [number]" },
    Sort(SortArgs) => { name: "sort", category: Chainable, max_args: Some(1), options: DESC, help: "sort <columns>" },
    Count(CountArgs) => { name: "count", category: Chainable, max_args: Some(1), options: NONE, help: "count [columns]" },
    TimeInfo(TimeInfoArgs) => { name: "timeinfo", category: Chainable, max_args: Some(1), options: TIMEINFO, help: "timeinfo <datetime-column> [--by columns]" },
    Uniq(UnitArgs) => { name: "uniq", category: Chainable, max_args: Some(0), options: NONE, help: "uniq" },
    ChangeTz(ChangeTzArgs) => { name: "changetz", category: Chainable, max_args: Some(1), options: TZ, help: "changetz <column> --from-tz <tz> --to-tz <tz>" },
    RenameCol(RenameColArgs) => { name: "renamecol", category: Chainable, max_args: Some(2), options: NONE, help: "renamecol <old> <new>" },
    TimeSlice(TimeSliceArgs) => { name: "timeslice", category: Chainable, max_args: Some(1), options: TIMESLICE, help: "timeslice <column> [--start <time>] [--end <time>]" },
    Show(ShowArgs) => { name: "show", category: Finalizer, max_args: Some(0), options: SHOW, help: "show" },
    ShowTable(UnitArgs) => { name: "showtable", category: Finalizer, max_args: Some(0), options: NONE, help: "showtable" },
    Headers(HeadersArgs) => { name: "headers", category: Finalizer, max_args: Some(0), options: HEADERS, help: "headers [--plain]" },
    Stats(UnitArgs) => { name: "stats", category: Finalizer, max_args: Some(0), options: NONE, help: "stats" },
    ShowQuery(UnitArgs) => { name: "showquery", category: Finalizer, max_args: Some(0), options: NONE, help: "showquery" },
    Dump(DumpArgs) => { name: "dump", category: Finalizer, max_args: Some(0), options: DUMP, help: "dump [options]" },
    DumpCache(DumpCacheArgs) => { name: "dumpcache", category: Finalizer, max_args: Some(0), options: OUTPUT, help: "dumpcache [--output <file>]" },
    Partition(PartitionArgs) => { name: "partition", category: Finalizer, max_args: Some(2), options: NONE, help: "partition <column> [directory]" },
    Calc(CalcArgs) => { name: "calc", category: Finalizer, max_args: Some(1), options: CALC, help: "calc <column> --sum|--avg|--min|--max|--median|--std" },
    Run(RunArgs) => { name: "run", category: Automation, max_args: None, options: RUN, help: "run <config> [files...] [options]" },
}
pub fn registry() -> &'static [CommandSpec] {
    SPECS
}

pub fn by_name(name: &str) -> Option<&'static CommandSpec> {
    registry().iter().find(|definition| definition.name == name)
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
    }
}
