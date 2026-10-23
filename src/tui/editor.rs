use crate::controllers::command_model::TypedCommand;

#[derive(Clone, Debug, Default)]
pub(super) struct EditorDocument {
    groups: Vec<CommandGroup>,
}

#[derive(Clone, Debug)]
struct CommandGroup {
    source: String,
    commands: Vec<TypedCommand>,
}

impl EditorDocument {
    pub(super) fn new(commands: Vec<TypedCommand>, source: Option<String>) -> Self {
        let groups = if commands.is_empty() {
            Vec::new()
        } else {
            vec![CommandGroup {
                source: source.unwrap_or_default(),
                commands,
            }]
        };
        Self { groups }
    }

    pub(super) fn from_recipe(
        commands: Vec<TypedCommand>,
    ) -> Result<Self, crate::error::QuiltError> {
        let mut groups = Vec::with_capacity(commands.len());
        for command in commands {
            let source = crate::controllers::cli_export::format_command(&command)?;
            groups.push(CommandGroup {
                source,
                commands: vec![command],
            });
        }
        Ok(Self { groups })
    }

    pub(super) fn commands(&self) -> Vec<TypedCommand> {
        self.groups
            .iter()
            .flat_map(|group| group.commands.iter().cloned())
            .collect()
    }
    pub(super) fn len(&self) -> usize {
        self.groups.iter().map(|group| group.commands.len()).sum()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(super) fn get(&self, index: usize) -> Option<&TypedCommand> {
        let mut offset = index;
        self.groups.iter().find_map(|group| {
            if offset < group.commands.len() {
                group.commands.get(offset)
            } else {
                offset -= group.commands.len();
                None
            }
        })
    }
    pub(super) fn ranges(&self) -> Vec<(usize, usize, String)> {
        let mut start = 0;
        self.groups
            .iter()
            .map(|group| {
                let range = (start, group.commands.len(), group.source.clone());
                start += group.commands.len();
                range
            })
            .collect()
    }

    pub(super) fn append(&mut self, commands: Vec<TypedCommand>, source: String) {
        if !commands.is_empty() {
            self.groups.push(CommandGroup { source, commands });
        }
    }

    pub(super) fn replace_group(
        &mut self,
        commands: Vec<TypedCommand>,
        start: usize,
        old_len: usize,
        source: String,
    ) {
        let old_total = self.len();
        let new_len = commands
            .len()
            .saturating_sub(old_total.saturating_sub(old_len));
        let mut offset = 0;
        let mut replaced = false;
        let mut next = Vec::new();
        for group in self.groups.drain(..) {
            let end = offset + group.commands.len();
            if end <= start || offset >= start + old_len {
                next.push(group);
            } else if !replaced {
                if new_len > 0 {
                    next.push(CommandGroup {
                        source: source.clone(),
                        commands: commands[start..start + new_len].to_vec(),
                    });
                }
                replaced = true;
            }
            offset = end;
        }
        if !replaced && new_len > 0 {
            next.push(CommandGroup {
                source,
                commands: commands[start..start + new_len].to_vec(),
            });
        }
        self.groups = next;
    }
}

#[cfg(test)]
mod tests {
    use super::EditorDocument;
    use crate::controllers::command_model::{IsinArgs, TypedCommand};
    use crate::tui::input::parse_commands;

    #[test]
    fn formatted_pipeline_opens_as_editable_groups_and_replaces_one_group() {
        let commands = parse_commands("load input.csv - head 5 - tail 1").unwrap();
        let mut document = EditorDocument::from_recipe(commands.clone()).unwrap();
        let groups = document.ranges();
        assert_eq!(groups.len(), commands.len());
        for (index, (start, len, source)) in groups.iter().enumerate() {
            assert_eq!((*start, *len), (index, 1));
            assert_eq!(parse_commands(source).unwrap(), [commands[index].clone()]);
        }

        let replacement = parse_commands("load input.csv - head 8 - tail 1").unwrap();
        document.replace_group(replacement.clone(), 1, 1, "head 8".into());
        assert_eq!(document.commands(), replacement);
        assert_eq!(document.ranges()[1], (1, 1, "head 8".into()));
        assert_eq!(document.ranges()[2], groups[2]);
    }

    #[test]
    fn empty_membership_command_keeps_its_group_and_can_be_reparsed() {
        let command = TypedCommand::Isin(IsinArgs {
            column: "kind".into(),
            values: Vec::new(),
        });
        let mut document = EditorDocument::from_recipe(vec![command.clone()]).unwrap();
        let source = document.ranges()[0].2.clone();
        assert_eq!(
            parse_commands(&source).unwrap().as_slice(),
            std::slice::from_ref(&command)
        );

        document.replace_group(vec![command.clone()], 0, 1, source);
        assert_eq!(document.commands(), [command]);
        assert_eq!(document.len(), 1);
    }
}
