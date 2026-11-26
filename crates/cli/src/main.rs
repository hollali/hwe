use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use hwe_core::App;
use hwe_core::state::{Settings, SortMode};
use hwe_tui::{TuiRenderer, is_valid_color, is_valid_theme, spawn_input};
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
        value_parser = parse_theme,
        help = "Color theme (env: FM_THEME)"
    )]
    theme: Option<String>,

    #[arg(
        long,
        value_name = "MODE",
        value_parser = parse_color,
        help = "Color mode (env: FM_COLOR)"
    )]
    color: Option<String>,
}

fn parse_theme(value: &str) -> std::result::Result<String, String> {
    if is_valid_theme(value) {
        Ok(value.to_string())
    } else {
        Err(format!("invalid theme: {value}"))
    }
}

fn parse_color(value: &str) -> std::result::Result<String, String> {
    if is_valid_color(value) {
        Ok(value.to_string())
    } else {
        Err(format!("invalid color mode: {value}"))
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let start = resolve_start(&cli.path)?;
    let config_path = config_path();
    let (config, config_writable) = load_config(config_path.as_deref());
    validate_config(&config);

    let theme = cli
        .theme
        .or_else(|| env_value("FM_THEME", is_valid_theme))
        .or_else(|| config.theme.clone());
    let color = cli
        .color
        .or_else(|| env_value("FM_COLOR", is_valid_color))
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
        config_writable,
    ))
}

fn env_value(name: &str, valid: fn(&str) -> bool) -> Option<String> {
    let value = std::env::var(name).ok()?;
    if value.is_empty() {
        return None;
    }
    if valid(&value) {
        Some(value)
    } else {
        eprintln!("warning: ignoring invalid {name}: {value}");
        None
    }
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

fn non_empty_path(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn config_path() -> Option<PathBuf> {
    non_empty_path(std::env::var_os("XDG_CONFIG_HOME"))
        .or_else(|| non_empty_path(std::env::var_os("HOME")))
        .or_else(|| {
            non_empty_path(std::env::var_os("USERPROFILE")).map(|home| home.join(".config"))
        })
        .map(|dir| dir.join("hwe").join("config.toml"))
}

fn load_config(path: Option<&Path>) -> (FileConfig, bool) {
    let Some(path) = path else {
        return (FileConfig::default(), false);
    };
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (FileConfig::default(), true);
        }
        Err(error) => {
            eprintln!("warning: cannot read config {}: {error}", path.display());
            return (FileConfig::default(), false);
        }
    };
    match toml::from_str(&contents) {
        Ok(config) => (config, true),
        Err(error) => {
            eprintln!(
                "warning: ignoring invalid config {}: {error}",
                path.display()
            );
            (FileConfig::default(), false)
        }
    }
}

fn validate_config(config: &FileConfig) {
    if let Some(theme) = config.theme.as_deref()
        && !is_valid_theme(theme)
    {
        eprintln!("warning: unknown theme in config: {theme}");
    }
    if let Some(color) = config.color.as_deref()
        && !is_valid_color(color)
    {
        eprintln!("warning: unknown color mode in config: {color}");
    }
    if let Some(sort) = config.sort.as_deref()
        && SortMode::parse(sort).is_none()
    {
        eprintln!("warning: unknown sort mode in config: {sort}");
    }
}

fn save_settings(
    path: Option<&Path>,
    config: &FileConfig,
    settings: Settings,
    config_writable: bool,
) {
    let Some(path) = path else {
        return;
    };
    if !config_writable {
        return;
    }
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
    config_writable: bool,
) -> Result<()> {
    let (events_tx, events_rx) = mpsc::unbounded_channel();

    let mut renderer =
        TuiRenderer::with_options(theme, color).context("failed to initialize the terminal")?;
    let input = spawn_input(events_tx);

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
    let _ = input.join();
    save_settings(config_path, config, settings, config_writable);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_parsers_accept_aliases_and_reject_junk() {
        assert!(parse_theme("tokyo").is_ok());
        assert!(parse_theme("Tokyo_Night").is_ok());
        assert!(parse_theme("neon").is_err());
        assert!(parse_color("truecolor").is_ok());
        assert!(parse_color("24bit").is_ok());
        assert!(parse_color("rainbow").is_err());
        assert_eq!(env_value("HWE_TEST_MISSING_VALUE", is_valid_theme), None);
    }

    #[test]
    fn config_path_points_at_the_config_file() {
        let Some(path) = config_path() else {
            return;
        };
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("config.toml")
        );
        assert_eq!(
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str()),
            Some("hwe")
        );
    }

    #[test]
    fn non_empty_paths_skip_missing_and_blank_values() {
        assert_eq!(non_empty_path(None), None);
        assert_eq!(non_empty_path(Some(std::ffi::OsString::new())), None);
        assert_eq!(
            non_empty_path(Some(std::ffi::OsString::from("/tmp"))),
            Some(PathBuf::from("/tmp"))
        );
    }
}
