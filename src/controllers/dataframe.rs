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

#[derive(Clone)]
pub struct DataFrameController {
    df: Option<LazyFrame>,
}

impl Default for DataFrameController {
    fn default() -> Self {
        Self::new()
    }
}

impl DataFrameController {
    pub fn new() -> Self {
        Self { df: None }
    }
    pub fn set_df(&mut self, df: LazyFrame) {
        self.df = Some(df);
    }
    pub fn into_df(self) -> Option<LazyFrame> {
        self.df
    }
    pub fn is_empty(&self) -> bool {
        self.df.is_none()
    }
    pub fn frame(&self) -> Option<&LazyFrame> {
        self.df.as_ref()
    }
    // -- initializers --
    pub fn load(
        &mut self,
        paths: &[PathBuf],
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
        infer_schema_length: Option<usize>,
    ) -> Result<&mut Self, QuiltError> {
        self.df = Some(load::load_with_ndjson_inference(
            paths,
            separator,
            low_memory,
            no_headers,
            chunk_size,
            infer_schema_length,
        )?);
        Ok(self)
    }
    // -- chainables --
    pub fn cast(
        &mut self,
        colname: &str,
        target: &str,
        datetime: &crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(cast::cast_with_config(df, colname, target, datetime)?);
        }
        Ok(self)
    }

    pub fn parse_size(&mut self, colname: &str) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(parse_size::parse_size_column(df, colname)?);
        }
        Ok(self)
    }

    pub fn bucket(
        &mut self,
        colname: &str,
        interval: &str,
        output: Option<&str>,
        datetime: crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(bucket::bucket_with_config(
                df, colname, interval, output, datetime,
            )?);
        }
        Ok(self)
    }

    pub fn delta(&mut self, colname: &str, output: Option<&str>) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(delta::delta(df, colname, output)?);
        }
        Ok(self)
    }

    pub fn extract(&mut self, colname: &str, pattern: &str) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(extract::extract(df, colname, pattern)?);
        }
        Ok(self)
    }

    pub fn flatten(&mut self) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(flatten::flatten(df)?);
        }
        Ok(self)
    }

    pub fn select(&mut self, colnames: &[String]) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(select::select(df, colnames)?);
        }
        Ok(self)
    }
    pub fn isin(&mut self, colname: &str, values: &[String]) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(isin::isin(df, colname, values)?);
        }
        Ok(self)
    }
    pub fn contains(
        &mut self,
        colname: &str,
        pattern: &str,
        ignorecase: bool,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(contains::contains(df, colname, pattern, ignorecase)?);
        }
        Ok(self)
    }
    pub fn sed(
        &mut self,
        colname: Option<&str>,
        pattern: &str,
        replacement: &str,
        ignorecase: bool,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(sed::sed(df, colname, pattern, replacement, ignorecase)?);
        }
        Ok(self)
    }
    pub fn grep(
        &mut self,
        pattern: &str,
        ignorecase: bool,
        is_inverted: bool,
        columns: Option<&[String]>,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(grep::grep(df, pattern, ignorecase, is_inverted, columns)?);
        }
        Ok(self)
    }
    pub fn head(&mut self, number: usize) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(head::head(df, number)?);
        }
        Ok(self)
    }
    pub fn tail(&mut self, number: usize) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(tail::tail(df, number)?);
        }
        Ok(self)
    }
    pub fn sort(&mut self, colnames: &[String], desc: bool) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(sort::sort(df, colnames, desc)?);
        }
        Ok(self)
    }
    pub fn count(&mut self, columns: &[String]) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(count::count(df, columns)?);
        }
        Ok(self)
    }
    pub fn uniq(&mut self) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(uniq::uniq(df)?);
        }
        Ok(self)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn changetz(
        &mut self,
        colname: &str,
        tz_from: &str,
        tz_to: &str,
        input_format: Option<&str>,
        output_format: Option<&str>,
        ambiguous_time: Option<&str>,
        nonexistent_time: Option<&str>,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(changetz::changetz(
                df,
                colname,
                tz_from,
                tz_to,
                input_format,
                output_format,
                ambiguous_time,
                nonexistent_time,
            )?);
        }
        Ok(self)
    }
    pub fn changetz_with_config(
        &mut self,
        colname: &str,
        tz_from: &str,
        tz_to: &str,
        output_format: Option<&str>,
        datetime: crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(changetz::changetz_with_config(
                df,
                colname,
                tz_from,
                tz_to,
                output_format,
                datetime,
            )?);
        }
        Ok(self)
    }
    pub fn renamecol(&mut self, old_name: &str, new_name: &str) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(renamecol::renamecol(df, old_name, new_name)?);
        }
        Ok(self)
    }
    pub fn timeslice(
        &mut self,
        time_column: &str,
        start_time: Option<&str>,
        end_time: Option<&str>,
        datetime: &crate::operations::datetime::DateTimeConfig,
    ) -> Result<&mut Self, QuiltError> {
        if let Some(df) = &self.df {
            self.df = Some(timeslice::timeslice(
                df,
                time_column,
                start_time,
                end_time,
                datetime,
            )?);
        }
        Ok(self)
    }
    // -- finalizers --
    pub fn headers_result(
        &self,
        plain: bool,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(|df| headers::headers(df, plain))
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn stats_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(stats::stats)
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn showquery_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(showquery::showquery)
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn show_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(show::show)
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn showtable_result(
        &self,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(showtable::showtable)
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn partition_result(
        &self,
        colname: &str,
        output_dir: &str,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(|df| partition::partition(df, colname, output_dir))
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn dumpcache_result(
        &self,
        output_path: Option<&str>,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(|df| dumpcache::dumpcache(df, output_path))
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn dump_result(
        &self,
        path: Option<&str>,
        separator: char,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(|df| dump::dump(df, path, separator))
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
    pub fn calc_result(
        &self,
        column: &str,
        mode: &str,
    ) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
        self.df
            .as_ref()
            .map(|df| calc::calc(df, column, mode))
            .transpose()?
            .ok_or_else(|| QuiltError::usage("no data loaded"))
    }
}
