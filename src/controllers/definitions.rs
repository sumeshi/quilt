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
    pub summary: &'static str,
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
    OptionSpec {
        name: "json-string",
        short: None,
        takes_value: true,
        repeated: true,
    },
    OptionSpec {
        name: "cache-input",
        short: None,
        takes_value: false,
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
    ($( $id:ident($arg:ty) => { name: $name:literal, category: $category:ident, max_args: $max:expr, options: $options:expr, help: $help:literal, summary: $summary:literal },)+) => {
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
            $( CommandSpec { category: CommandCategory::$category, name: $name, max_args: $max, options: $options, help: $help, summary: $summary }, )+
        ];
    };
}

define_operations! {
    Load(LoadArgs) => { name: "load", category: Initializer, max_args: None, options: LOAD, help: "load [files...] [options] — CSV, JSONL/NDJSON, or Parquet", summary: "Read CSV, JSONL, or Parquet data from files or standard input" },
    Select(SelectArgs) => { name: "select", category: Chainable, max_args: None, options: NONE, help: "select <columns>", summary: "Keep selected columns and choose their order" },
    Filter(FilterArgs) => { name: "filter", category: Chainable, max_args: Some(1), options: NONE, help: "filter <expression>", summary: "Keep rows that match a Boolean expression" },
    Derive(DeriveArgs) => { name: "derive", category: Chainable, max_args: Some(2), options: NONE, help: "derive <output> <expression>", summary: "Create a column from a typed expression" },
    Aggregate(AggregateArgs) => { name: "aggregate", category: Chainable, max_args: None, options: TIMEINFO, help: "aggregate <mode:column:output...> [--by columns]", summary: "Compute summary metrics, optionally grouped by columns" },
    ArrayContains(ArrayContainsArgs) => { name: "array-contains", category: Chainable, max_args: Some(2), options: NONE, help: "array-contains <column> <value>", summary: "Keep rows whose list contains a value" },
    ArrayGet(ArrayGetArgs) => { name: "array-get", category: Chainable, max_args: Some(2), options: OUTPUT, help: "array-get <column> <index> [--output name]", summary: "Get one item from each list into a new column" },
    Explode(ExplodeArgs) => { name: "explode", category: Chainable, max_args: Some(1), options: NONE, help: "explode <columns>", summary: "Expand list items into separate rows" },
    Cast(CastArgs) => { name: "cast", category: Chainable, max_args: Some(2), options: DATETIME, help: "cast <column> <type>", summary: "Convert a column to another supported type" },
    Bucket(BucketArgs) => { name: "bucket", category: Chainable, max_args: Some(2), options: BUCKET, help: "bucket <column> <interval>", summary: "Group date or time values into fixed intervals" },
    Delta(ColumnOutputArgs) => { name: "delta", category: Chainable, max_args: Some(1), options: DELTA, help: "delta <column>", summary: "Calculate differences between consecutive values" },
    Extract(ExtractArgs) => { name: "extract", category: Chainable, max_args: Some(2), options: NONE, help: "extract <column> <regex>", summary: "Extract matching text from a column with a regular expression" },
    Flatten(UnitArgs) => { name: "flatten", category: Chainable, max_args: Some(0), options: NONE, help: "flatten", summary: "Expand nested object fields into columns" },
    ParseSize(ParseSizeArgs) => { name: "parse-size", category: Chainable, max_args: Some(1), options: NONE, help: "parse-size <column>", summary: "Parse human-readable sizes into numbers" },
    Isin(IsinArgs) => { name: "isin", category: Chainable, max_args: Some(2), options: NONE, help: "isin <column> <values>", summary: "Keep rows whose values match a supplied list" },
    Contains(ContainsArgs) => { name: "contains", category: Chainable, max_args: Some(2), options: IGNORE, help: "contains <column> <pattern>", summary: "Keep rows where a column contains text" },
    Sed(SedArgs) => { name: "sed", category: Chainable, max_args: Some(2), options: SED, help: "sed <pattern> <replacement>", summary: "Replace matching text in string columns" },
    Grep(GrepArgs) => { name: "grep", category: Chainable, max_args: Some(1), options: GREP, help: "grep <pattern>", summary: "Keep rows where any column matches text" },
    Head(NumberArgs) => { name: "head", category: Chainable, max_args: Some(1), options: NUMBER, help: "head [number]", summary: "Keep the first rows" },
    Tail(NumberArgs) => { name: "tail", category: Chainable, max_args: Some(1), options: NUMBER, help: "tail [number]", summary: "Keep the last rows" },
    Sort(SortArgs) => { name: "sort", category: Chainable, max_args: Some(1), options: DESC, help: "sort <columns>", summary: "Sort rows by one or more columns" },
    Count(CountArgs) => { name: "count", category: Chainable, max_args: Some(1), options: NONE, help: "count [columns]", summary: "Count repeated rows or values, highest first" },
    TimeInfo(TimeInfoArgs) => { name: "timeinfo", category: Chainable, max_args: Some(1), options: TIMEINFO, help: "timeinfo <datetime-column> [--by columns]", summary: "Summarize time gaps and ranges in a datetime column" },
    Uniq(UnitArgs) => { name: "uniq", category: Chainable, max_args: Some(0), options: NONE, help: "uniq", summary: "Keep one copy of each distinct row" },
    ChangeTz(ChangeTzArgs) => { name: "changetz", category: Chainable, max_args: Some(1), options: TZ, help: "changetz <column> --from-tz <tz> --to-tz <tz>", summary: "Convert datetime values between time zones" },
    RenameCol(RenameColArgs) => { name: "renamecol", category: Chainable, max_args: Some(2), options: NONE, help: "renamecol <old> <new>", summary: "Rename a column" },
    TimeSlice(TimeSliceArgs) => { name: "timeslice", category: Chainable, max_args: Some(1), options: TIMESLICE, help: "timeslice <column> [--start <time>] [--end <time>]", summary: "Keep rows within a datetime range" },
    Show(ShowArgs) => { name: "show", category: Finalizer, max_args: Some(0), options: SHOW, help: "show", summary: "Display rows as CSV or JSONL" },
    ShowTable(UnitArgs) => { name: "showtable", category: Finalizer, max_args: Some(0), options: NONE, help: "showtable", summary: "Display a compact table preview" },
    Headers(HeadersArgs) => { name: "headers", category: Finalizer, max_args: Some(0), options: HEADERS, help: "headers [--plain]", summary: "Print column names" },
    Stats(UnitArgs) => { name: "stats", category: Finalizer, max_args: Some(0), options: NONE, help: "stats", summary: "Show row and column statistics" },
    ShowQuery(UnitArgs) => { name: "showquery", category: Finalizer, max_args: Some(0), options: NONE, help: "showquery", summary: "Show the query plan" },
    Dump(DumpArgs) => { name: "dump", category: Finalizer, max_args: Some(0), options: DUMP, help: "dump [options]", summary: "Write rows to a file or standard output" },
    DumpCache(DumpCacheArgs) => { name: "dumpcache", category: Finalizer, max_args: Some(0), options: OUTPUT, help: "dumpcache [--output <file>]", summary: "Save the query result as a Parquet cache" },
    Partition(PartitionArgs) => { name: "partition", category: Finalizer, max_args: Some(2), options: NONE, help: "partition <column> [directory]", summary: "Write separate output files for each column value" },
    Calc(CalcArgs) => { name: "calc", category: Finalizer, max_args: Some(1), options: CALC, help: "calc <column> --sum|--avg|--min|--max|--median|--std", summary: "Calculate a statistic for one column" },
    Run(RunArgs) => { name: "run", category: Automation, max_args: None, options: RUN, help: "run <config> [files...] [options]", summary: "Run a workflow from a YAML configuration file" },
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
            assert!(!definition.summary.trim().is_empty());
        }
    }
}
