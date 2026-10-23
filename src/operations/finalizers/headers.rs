use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Table};
use polars::prelude::*;

pub fn headers(df: &LazyFrame, plain: bool) -> Result<FinalizerResult, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("headers", None::<String>, error.to_string()))?;
    let names = schema
        .iter_names()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if plain {
        Ok(FinalizerResult::Stdout(format!("{}\n", names.join("\n"))))
    } else {
        let mut table = Table::new();
        table.load_style(UTF8_FULL);
        table.set_header(vec!["#", "Column Name"]);
        for (index, name) in names.iter().enumerate() {
            table.add_row(vec![
                Cell::new(format!("{:02}", index + 1)),
                Cell::new(super::display::escape_controls(name)),
            ]);
        }
        Ok(FinalizerResult::Stdout(format!("{table}\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polars::prelude::Series;

    #[test]
    fn human_headers_and_stats_escape_controls_but_plain_headers_stay_raw() {
        let name = "日本語\u{1b}[31m\u{7}line\n\t";
        let values = Series::new(name.into(), ["日本語\u{1b}[31m\u{7}line\n\t"]);
        let df = DataFrame::new_infer_height(vec![values.into()]).unwrap();
        let lazy = df.lazy();

        let FinalizerResult::Stdout(rendered) = headers(&lazy, false).unwrap() else {
            panic!("headers should return stdout");
        };
        assert!(rendered.contains("日本語\\u{1b}[31m\\u{7}line\\n\\t"));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{7}'));

        let FinalizerResult::Stdout(plain) = headers(&lazy, true).unwrap() else {
            panic!("plain headers should return stdout");
        };
        assert!(plain.contains(name));

        let FinalizerResult::Stdout(stats) = super::super::stats::stats(&lazy).unwrap() else {
            panic!("stats should return stdout");
        };
        assert!(stats.contains("日本語\\u{1b}[31m\\u{7}line\\n\\t"));
        assert!(!stats.contains('\u{1b}'));
        assert!(!stats.contains('\u{7}'));
    }
}
