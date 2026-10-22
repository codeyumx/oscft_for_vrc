//! `facebridge-cli`: headless FaceBridge. `run` drives the engine until Ctrl+C; the other commands manage the
//! installed tracking modules without starting anything.

mod commands;
mod run;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "facebridge-cli", version, about = "Run VRChat face tracking modules without the UI")]
struct Cli {
    /// Data directory (config, installed modules, module settings). Default: %APPDATA%\FaceBridge.
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the engine and print notifications, logs, avatar and OSC status until Ctrl+C.
    Run {
        /// Stop by itself after this many seconds (for scripts and smoke tests).
        #[arg(long, value_name = "SECONDS")]
        exit_after: Option<f64>,
    },
    /// List installed modules.
    List,
    /// Install a module from a .zip file or a folder.
    Install { path: PathBuf },
    /// Copy the modules installed for VRCFaceTracking (%APPDATA%\VRCFaceTracking\CustomLibs).
    ImportVrcft,
    /// Uninstall a module (id, or a unique part of its name).
    Remove { module: String },
    /// Enable a module.
    Enable { module: String },
    /// Disable a module.
    Disable { module: String },
    /// Show a module's settings, or change them with KEY=VALUE arguments.
    Settings {
        module: String,
        /// Values are JSON (`true`, `5`, `0.5`, `"text"`); anything else is taken as plain text.
        #[arg(value_name = "KEY=VALUE")]
        assignments: Vec<String>,
    },
    /// Tracking mutation settings: calibration, unified correctors, parameter adjustment, data filter.
    Mutation {
        #[command(subcommand)]
        action: MutationCommand,
    },
    /// Learned calibration data.
    Calibration {
        #[command(subcommand)]
        action: CalibrationCommand,
    },
}

#[derive(Subcommand)]
enum MutationCommand {
    /// Print the tracking mutation settings (calibration, correctors, parameter adjustment, data filter) as JSON.
    Show,
    /// Merge a partial JSON object into the settings, e.g. '{"filter":{"enabled":true}}', and print the result.
    Set { json: String },
}

#[derive(Subcommand)]
enum CalibrationCommand {
    /// List the shapes with learned calibration data and the mean progress.
    Show,
    /// Forget the learned calibration of one shape (an expression name such as JawOpen), or of all shapes.
    Reset { shape: Option<String> },
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(fb_core::default_data_dir);
    match cli.command {
        Command::Run { exit_after } => run::run(data_dir, exit_after),
        Command::List => commands::list(data_dir),
        Command::Install { path } => commands::install(data_dir, &path),
        Command::ImportVrcft => commands::import_vrcft(data_dir),
        Command::Remove { module } => commands::remove(data_dir, &module),
        Command::Enable { module } => commands::set_enabled(data_dir, &module, true),
        Command::Disable { module } => commands::set_enabled(data_dir, &module, false),
        Command::Settings { module, assignments } => commands::settings(data_dir, &module, &assignments),
        Command::Mutation { action } => match action {
            MutationCommand::Show => commands::mutation_show(data_dir),
            MutationCommand::Set { json } => commands::mutation_set(data_dir, &json),
        },
        Command::Calibration { action } => match action {
            CalibrationCommand::Show => commands::calibration_show(data_dir),
            CalibrationCommand::Reset { shape } => commands::calibration_reset(data_dir, shape.as_deref()),
        },
    }
}
