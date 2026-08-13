use crate::operations::chainables::{
    bucket, cast, changetz, contains, count, delta, extract, flatten, grep, head, isin, parse_size,
    renamecol, sed, select, sort, tail, timeslice, uniq,
};
use crate::operations::finalizers::{
    calc, dump, dumpcache, headers, partition, show, showquery, showtable, stats,
};
use crate::operations::initializers::load;
use chrono::Local;
use polars::prelude::*;
use std::path::PathBuf;

#[derive(Clone)]
pub struct DataFrameController {
    df: Option<LazyFrame>,
}
impl DataFrameController {
    pub fn new() -> Self {
        Self { df: None }
    }
    pub fn set_df(&mut self, df: LazyFrame) {
        self.df = Some(df);
    }
    pub fn is_empty(&self) -> bool {
        self.df.is_none()
    }
    // -- initializers --
    pub fn load(
        &mut self,
        paths: &[PathBuf],
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
    ) -> &mut Self {
        self.df = Some(load::load(
            paths, separator, low_memory, no_headers, chunk_size,
        ));
        self
    }
    // -- chainables --
    pub fn cast(&mut self, colname: &str, target: &str) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(cast::cast(df, colname, target));
        }
        self
    }

    pub fn parse_size(&mut self, colname: &str) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(parse_size::parse_size_column(df, colname));
        }
        self
    }

    pub fn bucket(&mut self, colname: &str, interval: &str, output: Option<&str>) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(bucket::bucket(df, colname, interval, output));
        }
        self
    }

    pub fn delta(&mut self, colname: &str, output: Option<&str>) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(delta::delta(df, colname, output));
        }
        self
    }

    pub fn extract(&mut self, colname: &str, pattern: &str) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(extract::extract(df, colname, pattern));
        }
        self
    }

    pub fn flatten(&mut self) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(flatten::flatten(df));
        }
        self
    }

    pub fn select(&mut self, colnames: &[String]) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(select::select(df, colnames));
        }
        self
    }
    pub fn isin(&mut self, colname: &str, values: &[String]) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(isin::isin(df, colname, values));
        }
        self
    }
    pub fn contains(&mut self, colname: &str, pattern: &str, ignorecase: bool) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(contains::contains(df, colname, pattern, ignorecase));
        }
        self
    }
    pub fn sed(
        &mut self,
        colname: Option<&str>,
        pattern: &str,
        replacement: &str,
        ignorecase: bool,
    ) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(sed::sed(df, colname, pattern, replacement, ignorecase));
        }
        self
    }
    pub fn grep(
        &mut self,
        pattern: &str,
        ignorecase: bool,
        is_inverted: bool,
        columns: Option<&[String]>,
    ) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(grep::grep(df, pattern, ignorecase, is_inverted, columns));
        }
        self
    }
    pub fn head(&mut self, number: usize) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(head::head(df, number));
        }
        self
    }
    pub fn tail(&mut self, number: usize) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(tail::tail(df, number));
        }
        self
    }
    pub fn sort(&mut self, colnames: &[String], desc: bool) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(sort::sort(df, colnames, desc));
        }
        self
    }
    pub fn count(&mut self, columns: &[String]) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(count::count(df, columns));
        }
        self
    }
    pub fn uniq(&mut self) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(uniq::uniq(df));
        }
        self
    }
    pub fn changetz(
        &mut self,
        colname: &str,
        tz_from: &str,
        tz_to: &str,
        input_format: Option<&str>,
        output_format: Option<&str>,
        ambiguous_time: Option<&str>,
    ) -> &mut Self {
        if let Some(df) = &self.df {
            let input_format_str = input_format.unwrap_or("auto");
            let output_format_str = output_format.unwrap_or("auto");
            let ambiguous_str = ambiguous_time.unwrap_or("earliest");
            self.df = Some(changetz::changetz(
                df,
                colname,
                tz_from,
                tz_to,
                input_format_str,
                output_format_str,
                ambiguous_str,
            ));
        }
        self
    }
    pub fn renamecol(&mut self, old_name: &str, new_name: &str) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(renamecol::renamecol(df, old_name, new_name));
        }
        self
    }
    pub fn timeslice(
        &mut self,
        time_column: &str,
        start_time: Option<&str>,
        end_time: Option<&str>,
    ) -> &mut Self {
        if let Some(df) = &self.df {
            self.df = Some(timeslice::timeslice(df, time_column, start_time, end_time));
        }
        self
    }
    // -- finalizers --
    pub fn headers(&self, plain: bool) {
        if let Some(df) = &self.df {
            headers::headers(df, plain);
        }
    }
    pub fn stats(&self) {
        if let Some(df) = &self.df {
            stats::stats(df);
        }
    }
    pub fn showquery(&self) {
        if let Some(df) = &self.df {
            showquery::showquery(df);
        }
    }
    pub fn show(&self) {
        if let Some(df) = &self.df {
            show::show(df);
        }
    }
    pub fn show_with_batch_size(&self, batch_size: usize) {
        if let Some(df) = &self.df {
            show::show_with_batch_size(df, batch_size);
        }
    }
    pub fn showtable(&self) {
        if let Some(df) = &self.df {
            showtable::showtable(df);
        }
    }
    pub fn partition(&self, colname: &str, output_dir: &str) {
        if let Some(df) = &self.df {
            partition::partition(df, colname, output_dir);
        }
    }
    pub fn dump(&self, path: Option<&str>, separator: Option<char>) {
        if let Some(df) = &self.df {
            let output_path_str = path.map(|p| p.to_string()).unwrap_or_else(|| {
                let now = Local::now();
                format!("dump_{}.csv", now.format("%Y%m%d_%H%M%S"))
            });
            let sep_char = separator.unwrap_or(',');
            dump::dump(df, Some(&output_path_str), sep_char);
        }
    }
    pub fn dump_with_batch_size(&self, path: Option<&str>, separator: char, batch_size: usize) {
        if let Some(df) = &self.df {
            let output_path_str = path.map(|p| p.to_string()).unwrap_or_else(|| {
                let now = Local::now();
                format!("dump_{}.csv", now.format("%Y%m%d_%H%M%S"))
            });
            dump::dump_with_batch_size(df, Some(&output_path_str), separator, batch_size);
        }
    }
    pub fn dumpcache(&self, output_path: Option<&str>) {
        if let Some(df) = &self.df {
            dumpcache::dumpcache(df, output_path);
        }
    }

    pub fn calc(&self, column: &str, mode: &str) {
        if let Some(df) = &self.df {
            calc::calc(df, column, mode);
        }
    }
}
