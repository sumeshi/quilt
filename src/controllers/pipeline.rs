use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::chainables::{
    bucket, cast, changetz, contains, count, delta, extract, flatten, grep, head, isin, parse_size,
    renamecol, sed, select, sort, tail, timeinfo, timeslice, uniq,
};
use crate::operations::finalizers::{
    calc, dump, dumpcache, headers, partition, show, showquery, showtable, stats,
};
use crate::operations::initializers::load;
use polars::prelude::*;
use std::path::PathBuf;

/// A valid lazy pipeline and the resources that keep its managed inputs and
/// intermediate artifacts alive. Cloning a pipeline intentionally clones both.
#[derive(Clone)]
pub struct Pipeline {
    frame: LazyFrame,
    resources: ExecutionResources,
    source_inventory: Option<SourceInventory>,
}

pub type SourceInventory = (String, Vec<String>);

pub enum PipelineState {
    Empty(ExecutionResources),
    Loaded(Box<Pipeline>),
}

impl PipelineState {
    pub fn empty(resources: ExecutionResources) -> Self {
        Self::Empty(resources)
    }
    pub fn resources(&self) -> ExecutionResources {
        match self {
            Self::Empty(r) => r.clone(),
            Self::Loaded(p) => p.resources(),
        }
    }
    pub fn loaded(&self, command: &str) -> Result<&Pipeline, QuiltError> {
        match self {
            Self::Loaded(p) => Ok(p),
            Self::Empty(_) => Err(QuiltError::usage(format!(
                "Error: No data loaded. Please load data first before using '{command}'."
            ))),
        }
    }
    pub fn loaded_mut(&mut self, command: &str) -> Result<&mut Pipeline, QuiltError> {
        match self {
            Self::Loaded(p) => Ok(p),
            Self::Empty(_) => Err(QuiltError::usage(format!(
                "Error: No data loaded. Please load data first before using '{command}'."
            ))),
        }
    }
    pub fn replace_with_frame(&mut self, frame: LazyFrame) {
        let resources = self.resources();
        *self = Self::Loaded(Box::new(Pipeline::new(frame, resources)));
    }
    pub fn into_pipeline(self) -> Option<Pipeline> {
        match self {
            Self::Loaded(p) => Some(*p),
            Self::Empty(_) => None,
        }
    }
}

impl Pipeline {
    pub fn new(frame: LazyFrame, resources: ExecutionResources) -> Self {
        Self {
            frame,
            resources,
            source_inventory: None,
        }
    }
    pub fn frame(&self) -> &LazyFrame {
        &self.frame
    }
    pub fn resources(&self) -> ExecutionResources {
        self.resources.clone()
    }
    pub fn source_inventory(&self) -> Option<SourceInventory> {
        self.source_inventory.clone()
    }
    pub fn with_source_inventory(mut self, inventory: Option<SourceInventory>) -> Self {
        self.source_inventory = inventory;
        self
    }
    pub fn set_source_inventory(&mut self, inventory: Option<SourceInventory>) {
        self.source_inventory = inventory;
    }
    pub fn into_parts(self) -> (LazyFrame, ExecutionResources) {
        (self.frame, self.resources)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        paths: &[PathBuf],
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
        infer_schema_length: Option<usize>,
        source_column: Option<&str>,
        resources: ExecutionResources,
    ) -> Result<Self, QuiltError> {
        let frame = load::load_with_ndjson_inference_with_resources(
            paths,
            separator,
            low_memory,
            no_headers,
            chunk_size,
            infer_schema_length,
            source_column,
            &resources,
        )?;
        let inventory = source_column
            .map(|name| {
                load::expand_input_paths(paths).map(|paths| {
                    (
                        name.to_owned(),
                        paths
                            .iter()
                            .map(|path| path.to_string_lossy().into_owned())
                            .collect(),
                    )
                })
            })
            .transpose()?;
        Ok(Self::new(frame, resources).with_source_inventory(inventory))
    }

    pub fn load_args(
        args: &crate::controllers::arguments::LoadArgs,
        resources: ExecutionResources,
    ) -> Result<Self, QuiltError> {
        let csv = crate::controllers::csv_input::CsvInputOptions {
            all_strings: args.all_strings,
            schema: args.schema.clone(),
            infer_schema_length: args.infer_schema_length,
            rejects: args.rejects.clone(),
        };
        let frame = load::load_with_options(
            &args.paths,
            &args.separator.to_string(),
            args.low_memory,
            args.no_headers,
            args.chunk_size,
            args.infer_schema_length,
            args.source_column.as_deref(),
            load::LoadOptions {
                stdin_format: args.stdin_format.as_deref(),
                rejects_path: args.rejects.as_deref(),
                csv_options: Some(&csv),
            },
            &resources,
        )?;
        let inventory = args
            .source_column
            .as_ref()
            .map(|name| {
                if args.stdin_format.is_some() {
                    Ok((name.clone(), vec!["<stdin>".into()]))
                } else {
                    load::expand_input_paths(&args.paths).map(|paths| {
                        (
                            name.clone(),
                            paths
                                .iter()
                                .map(|p| p.to_string_lossy().into_owned())
                                .collect(),
                        )
                    })
                }
            })
            .transpose()?;
        Ok(Self::new(frame, resources).with_source_inventory(inventory))
    }
    pub fn filter(&mut self, expression: &str) -> Result<&mut Self, QuiltError> {
        self.frame = crate::operations::chainables::filter::filter(&self.frame, expression)?;
        Ok(self)
    }
    pub fn derive(&mut self, output: &str, expression: &str) -> Result<&mut Self, QuiltError> {
        self.frame =
            crate::operations::chainables::filter::derive(&self.frame, output, expression)?;
        Ok(self)
    }
    pub fn aggregate(
        &mut self,
        metrics: &[crate::operations::chainables::aggregate::AggregateSpec],
        by: &[String],
    ) -> Result<&mut Self, QuiltError> {
        self.frame = crate::operations::chainables::aggregate::aggregate(&self.frame, metrics, by)?;
        self.source_inventory = None;
        Ok(self)
    }
    pub fn array_contains(&mut self, column: &str, value: &str) -> Result<&mut Self, QuiltError> {
        self.frame = crate::operations::chainables::arrays::contains(&self.frame, column, value)?;
        Ok(self)
    }
    pub fn array_get(
        &mut self,
        column: &str,
        index: i64,
        output: &str,
    ) -> Result<&mut Self, QuiltError> {
        self.frame =
            crate::operations::chainables::arrays::get(&self.frame, column, index, output)?;
        Ok(self)
    }
    pub fn explode(&mut self, columns: &[String]) -> Result<&mut Self, QuiltError> {
        self.frame = crate::operations::chainables::arrays::explode(&self.frame, columns)?;
        Ok(self)
    }
    pub fn delta_by(
        &mut self,
        column: &str,
        output: Option<&str>,
        by: &[String],
    ) -> Result<&mut Self, QuiltError> {
        self.frame = delta::delta_with_groups(&self.frame, column, output, by)?;
        Ok(self)
    }
    pub fn show_jsonl_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        crate::operations::finalizers::jsonl::show_jsonl(&self.frame, &self.resources)
    }
    pub fn dump_jsonl_result(
        &self,
        output: Option<&str>,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        let path = output.map(str::to_owned).unwrap_or_else(|| {
            format!(
                "dump_{}.jsonl",
                chrono::Local::now().format("%Y%m%d_%H%M%S_%9f")
            )
        });
        crate::operations::finalizers::jsonl::dump_jsonl(&self.frame, &path)
    }
    pub fn cast(
        &mut self,
        colname: &str,
        target: &str,
        datetime: &crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        self.frame = cast::cast_with_config(&self.frame, colname, target, datetime)?;
        self.clear_modified_source(Some(colname));
        Ok(self)
    }

    pub fn parse_size(&mut self, colname: &str) -> Result<&mut Self, QuiltError> {
        self.frame = parse_size::parse_size_column(&self.frame, colname)?;
        self.clear_modified_source(Some(colname));
        Ok(self)
    }

    pub fn bucket(
        &mut self,
        colname: &str,
        interval: &str,
        output: Option<&str>,
        datetime: crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        self.frame = bucket::bucket_with_config(&self.frame, colname, interval, output, datetime)?;
        Ok(self)
    }

    pub fn delta(&mut self, colname: &str, output: Option<&str>) -> Result<&mut Self, QuiltError> {
        self.frame = delta::delta(&self.frame, colname, output)?;
        Ok(self)
    }

    pub fn extract(&mut self, colname: &str, pattern: &str) -> Result<&mut Self, QuiltError> {
        self.frame = extract::extract(&self.frame, colname, pattern)?;
        Ok(self)
    }

    pub fn flatten(&mut self) -> Result<&mut Self, QuiltError> {
        self.frame = flatten::flatten(&self.frame)?;
        Ok(self)
    }

    pub fn select(&mut self, colnames: &[String]) -> Result<&mut Self, QuiltError> {
        self.frame = select::select(&self.frame, colnames)?;
        self.clear_missing_source("select")?;
        Ok(self)
    }
    pub fn isin(&mut self, colname: &str, values: &[String]) -> Result<&mut Self, QuiltError> {
        self.frame = isin::isin(&self.frame, colname, values)?;
        Ok(self)
    }
    pub fn contains(
        &mut self,
        colname: &str,
        pattern: &str,
        ignorecase: bool,
    ) -> Result<&mut Self, QuiltError> {
        self.frame = contains::contains(&self.frame, colname, pattern, ignorecase)?;
        Ok(self)
    }
    pub fn sed(
        &mut self,
        colname: Option<&str>,
        pattern: &str,
        replacement: &str,
        ignorecase: bool,
    ) -> Result<&mut Self, QuiltError> {
        self.frame = sed::sed(&self.frame, colname, pattern, replacement, ignorecase)?;
        self.clear_modified_source(colname);
        Ok(self)
    }
    pub fn grep(
        &mut self,
        pattern: &str,
        ignorecase: bool,
        is_inverted: bool,
        columns: Option<&[String]>,
    ) -> Result<&mut Self, QuiltError> {
        self.frame = grep::grep(&self.frame, pattern, ignorecase, is_inverted, columns)?;
        Ok(self)
    }
    pub fn head(&mut self, number: usize) -> Result<&mut Self, QuiltError> {
        self.frame = head::head(&self.frame, number)?;
        Ok(self)
    }
    pub fn tail(&mut self, number: usize) -> Result<&mut Self, QuiltError> {
        self.frame = tail::tail(&self.frame, number)?;
        Ok(self)
    }
    pub fn sort(&mut self, colnames: &[String], desc: bool) -> Result<&mut Self, QuiltError> {
        self.frame = sort::sort(&self.frame, colnames, desc)?;
        Ok(self)
    }
    pub fn count(&mut self, columns: &[String]) -> Result<&mut Self, QuiltError> {
        self.frame = count::count(&self.frame, columns)?;
        self.clear_missing_source("count")?;
        Ok(self)
    }
    pub fn timeinfo(&mut self, column: &str, by: &[String]) -> Result<&mut Self, QuiltError> {
        let mut summary = timeinfo::timeinfo(&self.frame, column, by)?;
        if let Some((name, paths)) = &self.source_inventory {
            if by == std::slice::from_ref(name) {
                let mut paths = paths.clone();
                paths.sort();
                paths.dedup();
                let inventory =
                    DataFrame::new_infer_height(vec![
                        Series::new(name.clone().into(), paths).into()
                    ])
                    .map_err(|error| QuiltError::operation("timeinfo", error.to_string()))?
                    .lazy();
                summary = inventory
                    .join(
                        summary,
                        [Expr::Column(name.as_str().into())],
                        [Expr::Column(name.as_str().into())],
                        JoinArgs::new(JoinType::Left),
                    )
                    .with_columns(
                        ["rows", "valid_times", "null_times"]
                            .map(|name| col(name).fill_null(lit(0u32))),
                    );
            }
        }
        self.frame = summary;
        self.source_inventory = None;
        Ok(self)
    }
    pub fn uniq(&mut self) -> Result<&mut Self, QuiltError> {
        self.frame = uniq::uniq(&self.frame)?;
        Ok(self)
    }
    pub fn changetz(
        &mut self,
        colname: &str,
        tz_from: &str,
        tz_to: &str,
        output_format: Option<&str>,
        datetime: crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        self.frame = changetz::changetz_with_config(
            &self.frame,
            colname,
            tz_from,
            tz_to,
            output_format,
            datetime,
        )?;
        self.clear_modified_source(Some(colname));
        Ok(self)
    }
    pub fn renamecol(&mut self, old_name: &str, new_name: &str) -> Result<&mut Self, QuiltError> {
        self.frame = renamecol::renamecol(&self.frame, old_name, new_name)?;
        if let Some((name, _)) = &mut self.source_inventory {
            if name == old_name {
                *name = new_name.to_owned();
            }
        }
        Ok(self)
    }
    fn clear_modified_source(&mut self, column: Option<&str>) {
        if self
            .source_inventory
            .as_ref()
            .is_some_and(|(name, _)| column.is_none_or(|column| column == name))
        {
            self.source_inventory = None;
        }
    }
    fn clear_missing_source(&mut self, operation: &str) -> Result<(), QuiltError> {
        if let Some((name, _)) = &self.source_inventory {
            let schema = self.frame.clone().collect_schema().map_err(|error| {
                QuiltError::schema(operation, None::<String>, error.to_string())
            })?;
            if schema.get(name).is_none() {
                self.source_inventory = None;
            }
        }
        Ok(())
    }
    pub fn timeslice(
        &mut self,
        time_column: &str,
        start_time: Option<&str>,
        end_time: Option<&str>,
        datetime: &crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        self.frame =
            timeslice::timeslice(&self.frame, time_column, start_time, end_time, datetime)?;
        Ok(self)
    }
    // -- finalizers --
    pub fn headers_result(
        &self,
        plain: bool,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        headers::headers(&self.frame, plain)
    }
    pub fn stats_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        stats::stats(&self.frame)
    }
    pub fn showquery_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        showquery::showquery(&self.frame)
    }
    pub fn show_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        show::show(&self.frame, &self.resources)
    }
    pub fn showtable_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        showtable::showtable(&self.frame)
    }
    pub fn partition_result(
        &self,
        colname: &str,
        output_dir: &str,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        partition::partition(&self.frame, colname, output_dir)
    }
    pub fn dumpcache_result(
        &self,
        output_path: Option<&str>,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        dumpcache::dumpcache(&self.frame, output_path)
    }
    pub fn dump_result(
        &self,
        path: Option<&str>,
        separator: char,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        dump::dump(&self.frame, path, separator)
    }
    pub fn calc_result(
        &self,
        column: &str,
        mode: &str,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        calc::calc(&self.frame, column, mode)
    }
}
