use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use hwe_core::App;
use hwe_tui::{TuiRenderer, spawn_input};
use hwe_vfs::{LocalFs, VPath};
use tokio::sync::mpsc;

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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;

    runtime.block_on(run(start, cli.theme.as_deref(), cli.color.as_deref()))
}

fn resolve_start(path: &Path) -> Result<VPath> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to read the current directory")?
            .join(path)
    };

    let canonical = absolute
        .canonicalize()
        .with_context(|| format!("cannot open {}", absolute.display()))?;

    if !canonical.is_dir() {
        bail!("not a directory: {}", canonical.display());
    }

    Ok(VPath::new(canonical))
}

async fn run(start: VPath, theme: Option<&str>, color: Option<&str>) -> Result<()> {
    let (events_tx, events_rx) = mpsc::unbounded_channel();

    let mut renderer =
        TuiRenderer::with_options(theme, color).context("failed to initialize the terminal")?;
    let _input = spawn_input(events_tx);

    let app = App::new(Arc::new(LocalFs::new()), start, events_rx);
    app.run(&mut renderer)
        .await
        .context("the event loop stopped unexpectedly")?;

    Ok(())
}
