//! The student-device agent binary.
//!
//! Subcommands (Phase 1.4a — the ones that need no elevation are live; the SYSTEM service and the
//! session-helper spawn arrive in 1.4b, validated in a VM):
//!
//! - `sessions`   list the machine's login sessions (works unelevated)
//! - `supervise`  run a program and keep it alive with backoff (the engine the service will use)
//! - `version`    print name and version
//!
//! `install` / `uninstall` / `run` (as a Windows service) and `helper` are declared but not yet
//! implemented; they land in 1.4b.

mod supervisor;

use std::{
    process::{Command, ExitCode},
    time::Duration,
};

use supervisor::{RestartPolicy, supervise};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("sessions") => cmd_sessions(),
        Some("supervise") => cmd_supervise(&args[1..]),
        Some("version") => {
            println!(
                "{} agent {}",
                proto::PRODUCT_NAME,
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Some("install" | "uninstall" | "run" | "helper") => {
            eprintln!(
                "'{}' arrives in Phase 1.4b (Windows service + session helper).",
                args[0]
            );
            ExitCode::FAILURE
        }
        _ => {
            eprintln!("usage: cowatcher-agent <sessions|supervise <program> [args...]|version>");
            ExitCode::FAILURE
        }
    }
}

fn cmd_sessions() -> ExitCode {
    match platform::session::list_sessions() {
        Ok(sessions) => {
            println!("{} session(s):", sessions.len());
            for s in sessions {
                println!(
                    "  id={:<3} {:<14} {}",
                    s.id,
                    s.station,
                    if s.active { "ACTIVE" } else { "-" }
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_supervise(args: &[String]) -> ExitCode {
    let Some((program, rest)) = args.split_first() else {
        eprintln!("usage: cowatcher-agent supervise <program> [args...]");
        return ExitCode::FAILURE;
    };
    let program = program.clone();
    let rest = rest.to_vec();
    let make = move || {
        let mut c = Command::new(&program);
        c.args(&rest);
        c
    };
    // The CLI runs until the process is killed (Ctrl+C), like a service; the SCM stop control drives
    // a real stop signal in 1.4b. `|| false` = never stop on its own.
    let starts = supervise(
        make,
        RestartPolicy::default(),
        &(|| false),
        Duration::from_millis(200),
    );
    println!("supervisor stopped after {starts} start(s)");
    ExitCode::SUCCESS
}
