//! The context connector for Claude Code (`memory::connector`, README
//! "Context connector"): what Claude Code may read of Freebyrd's memory,
//! with the person's consent per project, a receipt for every answer and a
//! daily cap.
//!
//! `freebyrd-connector session-start` is Claude Code's SessionStart hook: it
//! reads the hook's input on stdin and prints the live threads' index as
//! context, or that Freebyrd waits for the person's OK, or nothing.
//! `freebyrd-connector mcp` is a stdio MCP server with three read-only
//! tools: `index`, `thread` and `references`. `freebyrd-connector check`
//! says what a call would see, for the person.
//!
//! Both answer only Claude Code (by their parent processes), open the memory
//! read-only (`FREEBYRD_MEMORY_DB`, or the default), and write only
//! `connector.db` and `connector.log` beside it (`FREEBYRD_CONNECTOR_DB`).
//! Neither ever fails a session: what goes wrong is a log line.

use std::io::{BufRead, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use memory::Store;
use memory::brief::Setup;
use memory::connector::{self, Connector};
use serde_json::{Value, json};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let code = match mode.as_str() {
        "session-start" => session_start(),
        "mcp" => mcp(),
        "check" => check(),
        _ => {
            eprintln!("usage: freebyrd-connector session-start | mcp | check");
            2
        }
    };
    std::process::exit(code);
}

fn memory() -> Result<Store> {
    let path = match std::env::var_os("FREEBYRD_MEMORY_DB") {
        Some(path) => PathBuf::from(path),
        None => Store::default_path().context("HOME is not set")?,
    };
    Store::open_read_only(&path)
}

/// The session's folder: the hook's `cwd`, else Claude Code's project
/// folder, else where this runs.
fn folder(hook_cwd: Option<&str>) -> String {
    hook_cwd
        .map(str::to_owned)
        .or_else(|| std::env::var("CLAUDE_PROJECT_DIR").ok())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|dir| dir.display().to_string())
        })
        .unwrap_or_default()
}

fn session_start() -> i32 {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let Ok(input) = serde_json::from_str::<Value>(&input) else {
        connector::log("session-start: the hook's input could not be read");
        return 0;
    };
    if !connector::started_by_claude_code() {
        connector::log("session-start: refused: not started by Claude Code");
        return 0;
    }
    let folder = folder(input["cwd"].as_str());
    let session = input["session_id"].as_str();
    let source = input["source"].as_str().unwrap_or("startup");
    let answered = (|| -> Result<Option<String>> {
        let store = memory()?;
        let connector = Connector::open_default()?;
        connector::session_start(
            &store,
            &connector,
            &Setup::load(),
            &folder,
            session,
            source,
            chrono::Local::now(),
        )
    })();
    match answered {
        Ok(Some(text)) => println!("{}", connector::hook_output(&text)),
        Ok(None) => {}
        Err(e) => connector::log(&format!("session-start: {e:#}")),
    }
    0
}

fn mcp() -> i32 {
    let caller = connector::started_by_claude_code();
    if !caller {
        connector::log("mcp: started by something other than Claude Code; it gets nothing");
    }
    let folder = folder(None);
    let session = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    let setup = Setup::load();
    let mut answer = |tool: &str, arguments: &Value| -> (String, bool) {
        if !caller {
            return ("Freebyrd answers only Claude Code.".to_owned(), true);
        }
        let answered = (|| -> Result<(String, bool)> {
            let store = memory()?;
            let connector = Connector::open_default()?;
            connector::call(
                &store,
                &connector,
                &setup,
                &folder,
                session.as_deref(),
                tool,
                arguments,
                chrono::Local::now(),
            )
        })();
        answered.unwrap_or_else(|e| {
            connector::log(&format!("mcp {tool}: {e:#}"));
            (
                "Freebyrd could not read its memory just now.".to_owned(),
                true,
            )
        })
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(message) => connector::handle(&message, &mut answer),
            Err(_) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": "Parse error" }
            })),
        };
        if let Some(reply) = reply
            && writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
    0
}

/// What a call from here would see: who started it, the memory, the
/// records, and the standing answers. Prints no memory content.
fn check() -> i32 {
    println!(
        "started by Claude Code: {}",
        connector::started_by_claude_code()
    );
    match memory() {
        Ok(_) => println!("memory: readable, at the version this build reads"),
        Err(e) => println!("memory: {e:#}"),
    }
    match Connector::open_default() {
        Ok(connector) => {
            println!(
                "records: {}",
                Connector::default_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            );
            for (tool, project, kind, _) in connector.consents().unwrap_or_default() {
                println!("  {tool}: {project}: {kind}");
            }
        }
        Err(e) => println!("records: {e:#}"),
    }
    0
}
