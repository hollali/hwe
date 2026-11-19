# Hwɛ

A fast, keyboard-driven terminal file manager written in Rust. The project is
called **Hwɛ**; the installed binary is **`fm`**.

Hwɛ browses directories asynchronously, renders a live preview of the selected
entry, and keeps the UI thread free while work happens in the background.

> Status: early development (`0.1.0`). Navigation, tabs, previews, themes, and
> the command palette work; file operations and search are planned.

## Features

- Vim-style navigation (`hjkl`, arrows, `gg`/`G`) with a command-based core
- Tabs with independent directories, loading, and previews
- Asynchronous, cancellable directory loading and sorting
- Live previews for directories, text files, and binaries, with line numbers,
  scrolling, and lightweight syntax highlighting
- Miller columns on wide terminals: parent, current, and preview panes
- Command palette (`:`) and keyboard help overlay (`?`)
- Four color themes (Tokyo Night, Catppuccin, Gruvbox, Nord), truecolor/256/no-color
  modes, and `NO_COLOR` / `FM_ASCII` fallbacks
- Optional file-type icons (`FM_ICONS` or `I`) and permission columns on wide lists
- Single self-contained binary with no runtime dependencies

## Install

### crates.io

```sh
cargo install hwe
```

This installs the `fm` binary. Rust 1.88 or newer is required.

### Prebuilt binaries

Download the latest archive for your platform from the GitHub or GitLab
releases page, then place `fm` somewhere on your `PATH`.

### From source

From a checkout of this repository:

```sh
cargo install --path crates/cli --locked
```

Or build a release binary directly:

```sh
cargo build --release -p hwe
./target/release/fm
```

## Usage

```sh
fm                      # open the current directory
fm ~/src                # open a specific directory
fm --theme nord ~/src   # pick a theme
fm --color 256          # force a 256-color palette
fm --help
fm --version
```

### Keys

| Key | Action |
| --- | --- |
| `j` / `Down` | Move down |
| `k` / `Up` | Move up |
| `h` / `Left` | Go to parent directory |
| `l` / `Right` / `Enter` | Enter directory |
| `gg` / `Home` | Go to top |
| `G` / `End` | Go to bottom |
| `Tab` | Focus / unfocus the preview pane |
| `j` / `k`, `PgUp` / `PgDn` | Scroll the focused preview |
| `Ctrl+T` | Open a new tab at the current directory |
| `Ctrl+W` | Close the current tab |
| `gt` / `gT` | Next / previous tab |
| `:` | Open the command palette |
| `?` | Toggle the keyboard help overlay |
| `T` | Cycle color themes |
| `I` | Toggle file-type icons |
| `F5` | Refresh |
| `q` / `Ctrl+C` | Quit |

Directories sort before files, case-insensitively. Leaving a directory with `h`
re-selects that directory in the parent.

When the terminal is at least 76 columns wide, the right pane previews the
selected entry:

- Directories: their contents, capped at 1000 entries
- Text files: up to 400 lines or 64 KiB, with control characters sanitized,
  line numbers, and syntax highlighting for common languages
- Binary files: a size notice instead of raw bytes

At 110 columns or more, a parent-directory column appears on the left, giving a
Miller-column layout: parent, current directory, preview. Preview focus is
indicated by the highlighted pane border; the footer shows the active keys.

### Environment

| Variable | Effect |
| --- | --- |
| `NO_COLOR` | Disable all colors |
| `FM_ASCII` | Use ASCII glyphs instead of Unicode box drawing |
| `FM_THEME` | `tokyo-night`, `catppuccin`, `gruvbox`, or `nord` |
| `FM_COLOR` | `auto`, `truecolor`, `256`, or `none` |
| `FM_ICONS` | `1`/`0` to enable or disable Nerd Font file icons |
| `COLORTERM` / `TERM` | Detected to enable truecolor when supported |

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Layout

| Crate | Package | Purpose |
| --- | --- | --- |
| `crates/vfs` | `hwe-vfs` | Filesystem trait, local backend, streaming directory reads |
| `crates/core` | `hwe-core` | App state, commands, keymap, event loop |
| `crates/tui` | `hwe-tui` | Ratatui renderer, theme, terminal input |
| `crates/cli` | `hwe` | Clap entry point; installs the `fm` binary |

## Releasing

1. Bump the version in `[workspace.package]` **and** in the three
   `[workspace.dependencies]` entries in the root `Cargo.toml`.
2. Commit, tag, and push:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

   The GitHub `Release` workflow builds Linux, macOS, and Windows archives.
   The GitLab pipeline publishes a Linux binary and creates a GitLab Release.

3. Publish the crates (requires a `CARGO_REGISTRY_TOKEN` repository secret).
   Run the **Publish crates** workflow manually with `dry_run` disabled, or
   publish in dependency order locally:

   ```sh
   cargo publish -p hwe-vfs
   cargo publish -p hwe-core
   cargo publish -p hwe-tui
   cargo publish -p hwe
   ```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
