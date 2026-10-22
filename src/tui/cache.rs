use super::task::TaskControl;
use crate::controllers::command_model::TypedCommand;
use crate::controllers::materialization::materialize_parquet;
use crate::controllers::pipeline::Pipeline;
use crate::QuiltError;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq)]
struct InputStamp {
    path: PathBuf,
    modified: Option<SystemTime>,
    bytes: Option<u64>,
}

struct Checkpoint {
    commands: Vec<TypedCommand>,
    pipeline: Pipeline,
    bytes: u64,
    input_stamps: Vec<InputStamp>,
}

/// A fully materialized checkpoint that has not yet replaced the live one.
/// Dropping it releases its managed Parquet file.
pub(super) struct PreparedCheckpoint {
    checkpoint: Checkpoint,
}

/// One full-data Parquet checkpoint for the current typed command prefix.
pub(super) struct SessionCache {
    limit: u64,
    checkpoint: Option<Checkpoint>,
}

impl SessionCache {
    pub(super) fn new(limit: u64) -> Self {
        Self {
            limit,
            checkpoint: None,
        }
    }

    pub(super) fn restore(&mut self, commands: &[TypedCommand]) -> Option<(usize, Pipeline)> {
        let Some(checkpoint) = &self.checkpoint else {
            return None;
        };
        if checkpoint.commands.len() <= commands.len()
            && checkpoint
                .commands
                .iter()
                .zip(commands)
                .all(|(a, b)| a == b)
            && checkpoint.input_stamps == input_stamps(&checkpoint.commands)
        {
            Some((checkpoint.commands.len(), checkpoint.pipeline.clone()))
        } else {
            self.checkpoint = None;
            None
        }
    }

    pub(super) fn prepare(
        &self,
        commands: Vec<TypedCommand>,
        pipeline: Pipeline,
    ) -> Result<PreparedCheckpoint, QuiltError> {
        let available = self
            .limit
            .saturating_sub(self.checkpoint.as_ref().map_or(0, |c| c.bytes));
        let source_inventory = pipeline.source_inventory();
        let stamps = input_stamps(&commands);
        let (frame, resources, bytes) = materialize_parquet(
            pipeline.frame().clone(),
            &pipeline.resources(),
            "qlt-tui-cache",
            Some(available),
        )?;
        let new_pipeline = Pipeline::new(frame, resources).with_source_inventory(source_inventory);
        Ok(PreparedCheckpoint {
            checkpoint: Checkpoint {
                commands,
                pipeline: new_pipeline,
                bytes,
                input_stamps: stamps,
            },
        })
    }

    /// Install a previously prepared checkpoint. This is the only mutating
    /// part of cache creation, so callers can check cancellation immediately
    /// before it and safely discard a prepared checkpoint when cancelled.
    pub(super) fn commit(
        &mut self,
        prepared: PreparedCheckpoint,
        task: &TaskControl,
    ) -> Result<super::CacheInfo, QuiltError> {
        task.check()?;
        let info = super::CacheInfo {
            bytes: prepared.checkpoint.bytes,
            steps: prepared.checkpoint.commands.len(),
        };
        self.checkpoint = Some(prepared.checkpoint);
        Ok(info)
    }

    #[cfg(test)]
    pub(super) fn store(
        &mut self,
        commands: Vec<TypedCommand>,
        pipeline: Pipeline,
    ) -> Result<super::CacheInfo, QuiltError> {
        let prepared = self.prepare(commands, pipeline)?;
        self.commit(prepared, &TaskControl::new())
    }

    pub(super) fn clear(&mut self) {
        self.checkpoint = None;
    }

    pub(super) fn info(&self) -> Option<super::CacheInfo> {
        self.checkpoint.as_ref().map(|c| super::CacheInfo {
            bytes: c.bytes,
            steps: c.commands.len(),
        })
    }
}

fn input_stamps(commands: &[TypedCommand]) -> Vec<InputStamp> {
    let mut stamps = Vec::new();
    for command in commands {
        let TypedCommand::Load(load) = command else {
            continue;
        };
        if load.stdin_format.is_some() {
            continue;
        }
        let expanded = crate::operations::initializers::load::expand_input_paths(&load.paths)
            .unwrap_or_else(|_| load.paths.clone());
        for path in expanded {
            let path = if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            };
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            let metadata = std::fs::metadata(&path).ok();
            stamps.push(InputStamp {
                path,
                modified: metadata.as_ref().and_then(|meta| meta.modified().ok()),
                bytes: metadata.map(|meta| meta.len()),
            });
        }
    }
    stamps
}

#[cfg(test)]
mod tests {
    use super::SessionCache;
    use crate::controllers::command_model::parse_typed_commands;
    use crate::controllers::pipeline::Pipeline;
    use crate::controllers::resources::ExecutionResources;
    use polars::prelude::*;

    fn resources(dir: &tempfile::TempDir) -> ExecutionResources {
        ExecutionResources::new_in(dir.path().to_path_buf())
    }

    fn commands(argv: &[&str]) -> Vec<crate::controllers::command_model::TypedCommand> {
        parse_typed_commands(&argv.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn restores_prefix_and_invalidates_changed_history() {
        let mut cache = SessionCache::new(1_000_000);
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.csv");
        std::fs::write(&input, "a\n1\n").unwrap();
        let input = input.to_str().unwrap();
        let cmds = commands(&["load", input, "-", "head", "2"]);
        let pipeline = Pipeline::new(
            df!("n" => [1u64, u64::MAX]).unwrap().lazy(),
            resources(&dir),
        );
        cache.store(cmds.clone(), pipeline).unwrap();
        let longer = commands(&["load", input, "-", "head", "2", "-", "tail", "1"]);
        let (steps, restored) = cache.restore(&longer).unwrap();
        assert_eq!(steps, cmds.len());
        assert_eq!(
            restored.frame().clone().collect().unwrap()["n"]
                .u64()
                .unwrap()
                .get(1),
            Some(u64::MAX)
        );
        let edited = commands(&["load", input, "-", "head", "3"]);
        assert!(cache.restore(&edited).is_none());
        assert!(cache.info().is_none());

        cache
            .store(
                cmds.clone(),
                Pipeline::new(df!("n" => [1u64]).unwrap().lazy(), resources(&dir)),
            )
            .unwrap();
        std::fs::write(input, "a\n1\n2\n").unwrap();
        assert!(cache.restore(&longer).is_none());
    }

    #[test]
    fn failed_over_budget_replacement_keeps_old_checkpoint_and_cleans_partial() {
        let mut cache = SessionCache::new(1_000_000);
        let old_dir = tempfile::tempdir().unwrap();
        let first = commands(&["load", "input.csv"]);
        cache
            .store(
                first.clone(),
                Pipeline::new(df!("n" => [1u64]).unwrap().lazy(), resources(&old_dir)),
            )
            .unwrap();
        let original = cache.info().unwrap();
        let replacement = commands(&["load", "other.csv"]);
        cache.limit = original.bytes * 2;
        let new_dir = tempfile::tempdir().unwrap();
        let dir = new_dir.path().to_path_buf();
        let big = Pipeline::new(
            df!("s" => ["x".repeat(100_000)]).unwrap().lazy(),
            resources(&new_dir),
        );
        assert!(cache.store(replacement, big).is_err());
        assert_eq!(cache.info().unwrap(), original);
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
    }

    #[test]
    fn nested_empty_struct_fails_without_replacing_checkpoint() {
        let mut cache = SessionCache::new(1_000_000);
        let old_dir = tempfile::tempdir().unwrap();
        let first = commands(&["load", "input.csv"]);
        cache
            .store(
                first.clone(),
                Pipeline::new(df!("n" => [1u64]).unwrap().lazy(), resources(&old_dir)),
            )
            .unwrap();
        let info = cache.info().unwrap();
        let empty = StructChunked::from_series("empty".into(), 0, std::iter::empty::<&Series>())
            .unwrap()
            .into_series();
        let nested = StructChunked::from_series("nested".into(), 0, [&empty].into_iter())
            .unwrap()
            .into_series();
        let frame = DataFrame::new_infer_height(vec![nested.into()])
            .unwrap()
            .lazy();
        let new_dir = tempfile::tempdir().unwrap();
        assert!(cache
            .store(
                commands(&["load", "empty.csv"]),
                Pipeline::new(frame, resources(&new_dir))
            )
            .is_err());
        assert_eq!(cache.info().unwrap(), info);
    }
}
