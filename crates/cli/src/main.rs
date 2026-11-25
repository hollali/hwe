use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use hwe_core::App;
use hwe_core::state::{Settings, SortMode};
use hwe_tui::{TuiRenderer, spawn_input};
use hwe_vfs::{LocalFs, VPath};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Serialize)]
struct SavedConfig<'a> {
    theme: Option<&'a str>,
    color: Option<&'a str>,
    show_hidden: bool,
    sort: &'a str,
    sort_reverse: bool,
    dirs_first: bool,
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

const THEMES: [&str; 4] = ["tokyo-night", "catppuccin", "gruvbox", "nord"];
const COLORS: [&str; 4] = ["auto", "truecolor", "256", "none"];

fn main() -> Result<()> {
    let cli = Cli::parse();
    let start = resolve_start(&cli.path)?;
    let config_path = config_path();
    let config = load_config(config_path.as_deref());
    validate_config(&config);

    let theme = cli
        .theme
        .or_else(|| env_value("FM_THEME"))
        .or_else(|| config.theme.clone());
    let color = cli
        .color
        .or_else(|| env_value("FM_COLOR"))
        .or_else(|| config.color.clone());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;

    runtime.block_on(run(
        start,
        theme.as_deref(),
        color.as_deref(),
        &config,
        config_path.as_deref(),
    ))
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
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

fn config_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|dir| dir.join("hwe").join("config.toml"))
}

fn load_config(path: Option<&Path>) -> FileConfig {
    let Some(path) = path else {
        return FileConfig::default();
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return FileConfig::default();
    };
    match toml::from_str(&contents) {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "warning: ignoring invalid config {}: {error}",
                path.display()
            );
            FileConfig::default()
        }
    }
}

fn validate_config(config: &FileConfig) {
    if let Some(theme) = config.theme.as_deref()
        && !THEMES.contains(&theme)
    {
        eprintln!("warning: unknown theme in config: {theme}");
    }
    if let Some(color) = config.color.as_deref()
        && !COLORS.contains(&color)
    {
        eprintln!("warning: unknown color mode in config: {color}");
    }
    if let Some(sort) = config.sort.as_deref()
        && SortMode::parse(sort).is_none()
    {
        eprintln!("warning: unknown sort mode in config: {sort}");
    }
}

fn save_settings(path: Option<&Path>, config: &FileConfig, settings: Settings) {
    let Some(path) = path else {
        return;
    };
    let saved = SavedConfig {
        theme: config.theme.as_deref(),
        color: config.color.as_deref(),
        show_hidden: settings.show_hidden,
        sort: settings.sort.mode.label(),
        sort_reverse: settings.sort.reverse,
        dirs_first: settings.sort.dirs_first,
    };
    let Ok(contents) = toml::to_string_pretty(&saved) else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        eprintln!("warning: cannot create config directory: {error}");
        return;
    }
    if let Err(error) = std::fs::write(path, contents) {
        eprintln!("warning: cannot save settings: {error}");
    }
}

async fn run(
    start: VPath,
    theme: Option<&str>,
    color: Option<&str>,
    config: &FileConfig,
    config_path: Option<&Path>,
) -> Result<()> {
    let (events_tx, events_rx) = mpsc::unbounded_channel();

    let mut renderer =
        TuiRenderer::with_options(theme, color).context("failed to initialize the terminal")?;
    let _input = spawn_input(events_tx);

    let mut settings = Settings::default();
    if let Some(show_hidden) = config.show_hidden {
        settings.show_hidden = show_hidden;
    }
    if let Some(mode) = config.sort.as_deref().and_then(SortMode::parse) {
        settings.sort.mode = mode;
    }
    if let Some(reverse) = config.sort_reverse {
        settings.sort.reverse = reverse;
    }
    if let Some(dirs_first) = config.dirs_first {
        settings.sort.dirs_first = dirs_first;
    }

    let app = App::new(Arc::new(LocalFs::new()), start, events_rx, settings);
    let settings = app
        .run(&mut renderer)
        .await
        .context("the event loop stopped unexpectedly")?;
    save_settings(config_path, config, settings);

    Ok(())
}
