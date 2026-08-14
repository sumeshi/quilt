use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::chainables::{
    bucket, cast, changetz, contains, count, delta, extract, flatten, grep, head, isin, parse_size,
    renamecol, sed, select, sort, tail, timeslice, uniq,
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
}

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
        Self { frame, resources }
    }
    pub fn frame(&self) -> &LazyFrame {
        &self.frame
    }
    pub fn resources(&self) -> ExecutionResources {
        self.resources.clone()
    }
    pub fn into_parts(self) -> (LazyFrame, ExecutionResources) {
        (self.frame, self.resources)
    }
    pub fn load(
        paths: &[PathBuf],
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
        infer_schema_length: Option<usize>,
        resources: ExecutionResources,
    ) -> Result<Self, QuiltError> {
        let frame = load::load_with_ndjson_inference_with_resources(
            paths,
            separator,
            low_memory,
            no_headers,
            chunk_size,
            infer_schema_length,
            &resources,
        )?;
        Ok(Self::new(frame, resources))
    }

    pub fn cast(
        &mut self,
        colname: &str,
        target: &str,
        datetime: &crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        self.frame = cast::cast_with_config(&self.frame, colname, target, datetime)?;
        Ok(self)
    }

    pub fn parse_size(&mut self, colname: &str) -> Result<&mut Self, QuiltError> {
        self.frame = parse_size::parse_size_column(&self.frame, colname)?;
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
        Ok(self)
    }
    pub fn renamecol(&mut self, old_name: &str, new_name: &str) -> Result<&mut Self, QuiltError> {
        self.frame = renamecol::renamecol(&self.frame, old_name, new_name)?;
        Ok(self)
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
