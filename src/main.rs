mod app;
mod browser;
mod clipboard;
mod commands;
mod csv_transfer;
mod i18n;
mod input;
mod sensitive;
mod settings;
mod theme;
mod updates;
mod vault;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::{io::IsTerminal, path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(
    name = "tvault",
    version,
    about = "Local terminal password vault — experimental MVP, keep backups"
)]
struct Cli {
    /// Existing vault path. Default: setup's selected vault, or local-data/TermVault/vault.kdbx
    path: Option<PathBuf>,
    /// Explicitly create a NEW vault (never overwrite an existing file)
    #[arg(long)]
    create: bool,
    /// Inactivity timeout in seconds; unsaved entry drafts are discarded on auto-lock
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(15..=3600))]
    lock_after: u64,
    /// Internal browser approval mode; never accepts a master password
    #[arg(long, hide = true, conflicts_with = "create")]
    browser_approve: Option<String>,
    /// Explicit isolated settings file (used by terminal process tests).
    #[arg(long, hide = true, conflicts_with = "browser_approve")]
    settings_file: Option<PathBuf>,
}

fn settings_file(cli: &Cli) -> Result<PathBuf> {
    match &cli.settings_file {
        Some(path) if path.is_absolute() => Ok(path.clone()),
        Some(_) => bail!("--settings-file must be an absolute file path"),
        None => settings::settings_file(),
    }
}

fn path(cli: &Cli) -> Result<PathBuf> {
    let path = match &cli.path {
        Some(path) => path.clone(),
        None => settings::default_vault_file(&settings_file(cli)?)?,
    };
    let path = if let Ok(suffix) = path.strip_prefix("~") {
        dirs::home_dir()
            .context("Cannot locate the home directory")?
            .join(suffix)
    } else {
        path
    };
    Ok(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    })
}

fn run() -> Result<()> {
    let first = std::env::args().nth(1);
    if first.as_deref() == Some("--native-host")
        || (first.as_deref() != Some("--browser-approve")
            && std::env::current_exe()?
                .file_stem()
                .is_some_and(|name| name == "tvault-browser-host"))
    {
        return browser::native_host();
    }
    let cli = Cli::parse();
    if let Some(origin) = &cli.browser_approve {
        return browser::approve(path(&cli)?, origin);
    }
    // No password flags, environment variables or redirected secret input.
    // Plaintext CSV export requires masked reauthentication and visible in-app consent.
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!("An interactive terminal is required. Master passwords are entered only in the masked UI.");
    }
    let path = path(&cli)?;
    if cli.create {
        if std::fs::symlink_metadata(&path).is_ok() {
            bail!("Destination already exists; --create never overwrites a vault");
        }
    } else {
        vault::inspect(&path)?;
    }
    app::run(
        path,
        cli.create,
        Duration::from_secs(cli.lock_after),
        settings_file(&cli)?,
    )
}

fn main() {
    if let Err(error) = run() {
        eprintln!("tvault: {}", input::display(&error.to_string()));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_settings_injection_is_absolute_and_never_falls_back() {
        let cli = Cli::try_parse_from(["tvault", "--settings-file", "relative"]).unwrap();
        assert!(settings_file(&cli).is_err());
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("isolated.json");
        let cli = Cli::try_parse_from([
            std::ffi::OsStr::new("tvault"),
            std::ffi::OsStr::new("--settings-file"),
            file.as_os_str(),
        ])
        .unwrap();
        assert_eq!(settings_file(&cli).unwrap(), file);
        assert!(!root.path().join("TermVault").exists());
    }
    #[test]
    fn cli_never_accepts_password_arguments_and_bounds_timeout() {
        assert!(Cli::try_parse_from(["tvault", "--password", "secret"]).is_err());
        assert!(Cli::try_parse_from(["tvault", "--lock-after", "0"]).is_err());
        assert!(
            Cli::try_parse_from(["tvault", "--create", "test.kdbx"])
                .unwrap()
                .create
        );
    }
}
