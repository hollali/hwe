use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use hwe_core::{App, state::Settings};
use hwe_tui::{TuiRenderer, spawn_input};
use hwe_vfs::{LocalFs, VPath};
use serde::Deserialize;
use tokio::sync::mpsc;

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    theme: Option<String>,
    color: Option<String>,
    show_hidden: Option<bool>,
    sort: Option<String>,
    sort_reverse: Option<bool>,
    dirs_first: Option<bool>,
}

#[derive(Debug, Parser)]
#[command(
    name = "fm",
    version,
    about = "A keyboard-driven terminal file manager"
)]
struct Cli {
    #[arg(value_name = "PATH", default_value = ".")]
    path: PathBuf,

    #[arg(
        long,
        value_name = "THEME",
        value_parser = ["tokyo-night", "catppuccin", "gruvbox", "nord"],
        help = "Color theme (env: FM_THEME)"
    )]
    theme: Option<String>,

    #[arg(
        long,
        value_name = "MODE",
        value_parser = ["auto", "truecolor", "256", "none"],
        help = "Color mode (env: FM_COLOR)"
    )]
    color: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let start = resolve_start(&cli.path)?;
    let config = load_config();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;

    runtime.block_on(run(
        start,
        cli.theme.as_deref(),
        cli.color.as_deref(),
        config,
    ))
}

fn resolve_start(path: &Path) -> Result<VPath> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to read the current directory")?
            .join(path)
    };

    let metadata = std::fs::metadata(&absolute)
        .with_context(|| format!("cannot open {}", absolute.display()))?;
    if !metadata.is_dir() {
        bail!("not a directory: {}", absolute.display());
    }

    Ok(VPath::new(absolute))
}

fn load_config() -> FileConfig {
    let path = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|dir| dir.join("hwe").join("config.toml"));
    let Some(path) = path else {
        return FileConfig::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return FileConfig::default();
    };
    toml::from_str(&contents).unwrap_or_default()
}

async fn run(
    start: VPath,
    theme: Option<&str>,
    color: Option<&str>,
    config: FileConfig,
) -> Result<()> {
    let (events_tx, events_rx) = mpsc::unbounded_channel();

    let theme = theme.or(config.theme.as_deref());
    let color = color.or(config.color.as_deref());

    let mut renderer =
        TuiRenderer::with_options(theme, color).context("failed to initialize the terminal")?;
    let _input = spawn_input(events_tx);

    let mut settings = Settings::default();
    if let Some(show_hidden) = config.show_hidden {
        settings.show_hidden = show_hidden;
    }
    if let Some(sort) = config.sort.as_deref()
        && let Some(mode) = hwe_core::state::SortMode::parse(sort)
    {
        settings.sort.mode = mode;
    }
    if let Some(reverse) = config.sort_reverse {
        settings.sort.reverse = reverse;
    }
    if let Some(dirs_first) = config.dirs_first {
        settings.sort.dirs_first = dirs_first;
    }

    let app = App::new(Arc::new(LocalFs::new()), start, events_rx, settings);
    app.run(&mut renderer)
        .await
        .context("the event loop stopped unexpectedly")?;

    Ok(())
}
