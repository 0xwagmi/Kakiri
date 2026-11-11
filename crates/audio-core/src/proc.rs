//! Spawning the tools we shell out to (yt-dlp, ffmpeg) without a console window.
//!
//! On Windows a child process inherits the parent's console unless told otherwise,
//! so every yt-dlp run popped a black terminal over the app. `CREATE_NO_WINDOW`
//! suppresses that; on other platforms this is only the stdin guard.

use std::process::{Command, Stdio};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Mark a command as an internal helper: no console window, no stdin.
pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null());
    cmd
}
