//! `hyprctl`: a drop-in shim that translates the handful of `hyprctl`
//! invocations `Velo-shell` makes into `velo-de` IPC requests, so
//! `Velo-shell` runs under `velo-de` with zero source changes.
//!
//! `velo-de` builds this binary as literally `hyprctl` and prepends its
//! directory to `PATH` for spawned children. Anything not recognized below
//! (or any error talking to `velo-de`) falls back to an empty JSON
//! value/no-op so callers that don't care about the result keep working.

use serde_json::json;
use velo_de_ipc::{Client, Command, Request, Response};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    print!("{}", run(&args).unwrap_or_else(|| fallback(&args)));
}

fn run(args: &[String]) -> Option<String> {
    let mut client = Client::connect().ok()?;

    match args {
        [flag, query] if flag == "-j" && query == "workspaces" => {
            let Response::Spaces(spaces) = client.request(&Request::GetSpaces).ok()? else { return None };
            let spaces: Vec<_> = spaces.into_iter().map(|s| json!({"id": s.id, "windows": s.windows})).collect();
            serde_json::to_string(&spaces).ok()
        }
        [flag, query] if flag == "-j" && query == "activeworkspace" => {
            let Response::ActiveSpace(space) = client.request(&Request::GetActiveSpace).ok()? else { return None };
            serde_json::to_string(&json!({"id": space.id})).ok()
        }
        [flag, query] if flag == "-j" && query == "activewindow" => {
            let Response::ActiveWindow(win) = client.request(&Request::GetActiveWindow).ok()? else { return None };
            match win {
                Some(w) => serde_json::to_string(&json!({"title": w.title, "class": w.class})).ok(),
                None => Some("{}".to_string()),
            }
        }
        [cmd, sub, id] if cmd == "dispatch" && sub == "workspace" => {
            let id: i32 = id.parse().ok()?;
            client.request(&Request::Dispatch(Command::FocusSpaceById(id))).ok()?;
            Some(String::new())
        }
        _ => None,
    }
}

/// A safe, exit-0 fallback for unrecognized commands or a `velo-de` that
/// isn't reachable: `-j` queries get an empty JSON value, everything else
/// gets empty output.
fn fallback(args: &[String]) -> String {
    if args.first().map(String::as_str) == Some("-j") {
        match args.get(1).map(String::as_str) {
            Some("workspaces") | Some("clients") | Some("monitors") | Some("layers") => "[]".to_string(),
            _ => "{}".to_string(),
        }
    } else {
        String::new()
    }
}
