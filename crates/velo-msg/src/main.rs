//! `velo-msg`: a small debug/scripting CLI over `velo-de`'s native IPC
//! socket (see `velo-de-ipc`).

use clap::{Parser, Subcommand, ValueEnum};
use velo_de_ipc::{Client, Command, Direction, Request, Response};

#[derive(Parser)]
#[command(name = "velo-msg", about = "Debug/scripting CLI for velo-de's IPC socket")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List all Spaces (Hyprland `workspaces`-shaped).
    Spaces,
    /// The currently-focused Space.
    ActiveSpace,
    /// The currently-focused window.
    ActiveWindow,
    /// Apply a command to the focused output's Spaces grid.
    Dispatch {
        #[command(subcommand)]
        command: DispatchCmd,
    },
}

#[derive(ValueEnum, Clone, Copy)]
enum DirArg {
    Left,
    Right,
    Up,
    Down,
}

impl From<DirArg> for Direction {
    fn from(dir: DirArg) -> Direction {
        match dir {
            DirArg::Left => Direction::Left,
            DirArg::Right => Direction::Right,
            DirArg::Up => Direction::Up,
            DirArg::Down => Direction::Down,
        }
    }
}

#[derive(Subcommand)]
enum DispatchCmd {
    FocusColumn { dir: DirArg },
    MoveColumn { dir: DirArg },
    SwitchSpace { dir: DirArg },
    MoveWindowToSpace { dir: DirArg },
    CycleWindow,
    ToggleColumnLayout,
    ToggleFullscreen,
    CloseFocused,
    /// Multiply the focused column's width by this factor (e.g. `1.1`).
    ResizeColumn { factor: f64 },
    ToggleOverview,
    OverviewMove { dir: DirArg },
    OverviewConfirm,
    OverviewCancel,
    /// Jump to (or create) the Space with this Hyprland-shaped id.
    FocusSpaceById { id: i32 },
}

impl DispatchCmd {
    fn into_command(self) -> Command {
        match self {
            DispatchCmd::FocusColumn { dir } => Command::FocusColumn(dir.into()),
            DispatchCmd::MoveColumn { dir } => Command::MoveColumn(dir.into()),
            DispatchCmd::SwitchSpace { dir } => Command::SwitchSpace(dir.into()),
            DispatchCmd::MoveWindowToSpace { dir } => Command::MoveWindowToSpace(dir.into()),
            DispatchCmd::CycleWindow => Command::CycleWindow,
            DispatchCmd::ToggleColumnLayout => Command::ToggleColumnLayout,
            DispatchCmd::ToggleFullscreen => Command::ToggleFullscreen,
            DispatchCmd::CloseFocused => Command::CloseFocused,
            DispatchCmd::ResizeColumn { factor } => Command::ResizeColumn(factor),
            DispatchCmd::ToggleOverview => Command::ToggleOverview,
            DispatchCmd::OverviewMove { dir } => Command::OverviewMove(dir.into()),
            DispatchCmd::OverviewConfirm => Command::OverviewConfirm,
            DispatchCmd::OverviewCancel => Command::OverviewCancel,
            DispatchCmd::FocusSpaceById { id } => Command::FocusSpaceById(id),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let req = match cli.cmd {
        Cmd::Spaces => Request::GetSpaces,
        Cmd::ActiveSpace => Request::GetActiveSpace,
        Cmd::ActiveWindow => Request::GetActiveWindow,
        Cmd::Dispatch { command } => Request::Dispatch(command.into_command()),
    };

    let mut client = Client::connect().map_err(|e| format!("connecting to velo-de: {e}"))?;
    let resp = client.request(&req)?;

    match resp {
        Response::Err(msg) => {
            eprintln!("velo-de: {msg}");
            std::process::exit(1);
        }
        Response::Ok => println!("ok"),
        other => println!("{}", serde_json::to_string_pretty(&other)?),
    }

    Ok(())
}
