use std::collections::HashMap;

use crate::effect::Effect;
use crate::state::AppState;

#[derive(Debug, Clone, Copy)]
pub struct ArgSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

#[derive(Debug)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub args: &'static [ArgSpec],
}

#[derive(Debug, Default, Clone)]
pub struct Args {
    positional: Vec<String>,
}

impl Args {
    pub fn new(positional: Vec<String>) -> Self {
        Self { positional }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn positional(&self) -> &[String] {
        &self.positional
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        self.positional.get(index).map(String::as_str)
    }
}

pub struct CommandContext<'a> {
    pub state: &'a mut AppState,
}

pub trait CommandHandler: Send + Sync {
    fn spec(&self) -> &'static CommandSpec;
    fn run(&self, ctx: &mut CommandContext<'_>, args: &Args) -> Effect;
}

#[derive(Default)]
pub struct CommandRegistry {
    handlers: Vec<Box<dyn CommandHandler>>,
    index: HashMap<&'static str, usize>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<H>(&mut self, handler: H)
    where
        H: CommandHandler + 'static,
    {
        let name = handler.spec().name;
        assert!(
            !self.index.contains_key(name),
            "command already registered: {name}"
        );
        let position = self.handlers.len();
        self.handlers.push(Box::new(handler));
        self.index.insert(name, position);
    }

    pub fn get(&self, name: &str) -> Option<&dyn CommandHandler> {
        self.index
            .get(name)
            .map(|position| self.handlers[*position].as_ref())
    }

    pub fn dispatch(
        &self,
        name: &str,
        ctx: &mut CommandContext<'_>,
        args: &Args,
    ) -> Option<Effect> {
        self.get(name).map(|handler| handler.run(ctx, args))
    }

    pub fn specs(&self) -> impl Iterator<Item = &'static CommandSpec> + '_ {
        self.handlers.iter().map(|handler| handler.spec())
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands;
    use hwe_vfs::VPath;

    #[test]
    fn registry_exposes_commands_with_descriptions() {
        let registry = commands::registry();
        assert!(registry.len() >= 8);
        for spec in registry.specs() {
            assert!(!spec.name.is_empty());
            assert!(!spec.description.is_empty());
        }
        assert!(registry.get("nav.down").is_some());
        assert!(registry.get("does.not.exist").is_none());
    }

    #[test]
    fn dispatch_runs_command_and_mutates_state() {
        let registry = commands::registry();
        let mut state = AppState::new(VPath::new("/tmp"));
        state.tab.loading = false;
        state.tab.entries.push(crate::state::Entry {
            name: "a".into(),
            path: VPath::new("/tmp/a"),
            kind: hwe_vfs::EntryKind::File,
            size: None,
            modified: None,
            is_hidden: false,
            is_symlink: false,
            is_executable: false,
        });

        let mut ctx = CommandContext { state: &mut state };
        let effect = registry
            .dispatch("nav.down", &mut ctx, &Args::empty())
            .expect("command");
        assert!(matches!(effect, Effect::None));
        assert_eq!(ctx.state.tab.cursor, 0);
    }
}
