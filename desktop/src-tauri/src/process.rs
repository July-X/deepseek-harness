//! Suppressing the console window Windows allocates for child processes.
//!
//! The shell is a GUI-subsystem app: every `Command` spawned without
//! `CREATE_NO_WINDOW` makes Windows briefly allocate a console window, which
//! the user sees as a flashing terminal. All helper-process spawns in this
//! crate go through `quiet`.

use std::process::Command;

/// Hide the console window Windows would otherwise flash for the child.
/// No-op on other platforms.
pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
