mod app;
mod buttondown;
mod clipboard;
mod config;
mod keys;
mod ui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "mnml-msg-buttondown",
    version,
    about = "Buttondown newsletter browser for mnml"
)]
struct Cli {
    /// Print the resolved config + auth state and exit.
    #[arg(long)]
    check: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.check {
        let cfg = config::load();
        let auth = buttondown::Auth::from_env();

        println!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        println!("config: {}", config::config_path().display());
        match &cfg {
            Ok(cfg) => {
                println!("tabs:");
                for (i, t) in cfg.tabs.iter().enumerate() {
                    println!("  {} ({}): kind={}", i + 1, t.name, t.kind);
                }
            }
            Err(e) => println!("config: ERROR — {e}"),
        }

        println!();
        println!("env: BUTTONDOWN_API_KEY={}", mask_env("BUTTONDOWN_API_KEY"));

        match &auth {
            Ok(a) => {
                println!();
                println!("api base: {}", a.api_base());
                println!("web base: {}", a.web_base());
                println!("auth: ok");
            }
            Err(e) => {
                println!();
                println!("auth: ERROR — {e}");
                std::process::exit(2);
            }
        }
        // If config errored, still exit non-zero so callers can
        // chain `&&` safely.
        if cfg.is_err() {
            std::process::exit(2);
        }
        return Ok(());
    }

    let cfg = config::load()?;
    let auth = match buttondown::Auth::from_env() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!();
            eprintln!("setup:");
            eprintln!(
                "  export BUTTONDOWN_API_KEY=...   (from Buttondown: Settings → Programming)"
            );
            eprintln!();
            eprintln!("then re-run, or `mnml-msg-buttondown --check` to confirm.");
            std::process::exit(2);
        }
    };

    let mut app = app::App::new(cfg, auth)?;
    ui::run(&mut app)
}

fn mask_env(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => {
            if v.len() > 6 {
                format!("set ({} chars, ends …{})", v.len(), &v[v.len() - 4..])
            } else {
                format!("set ({} chars)", v.len())
            }
        }
        _ => "(unset)".into(),
    }
}
