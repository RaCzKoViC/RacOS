// racinit — RacOS Init Process (PID 1)
//
// Boots the system by loading unit files from /etc/racinit/,
// resolving dependencies, starting services, and supervising them.
// Falls back to spawning /bin/sh if no unit files are found.

#![no_std]
#![no_main]

extern crate alloc;
extern crate libc_lite;

use alloc::string::String;
use init::{Unit, UnitType, ServiceType, RestartPolicy};
use init::engine::Engine;

fn fallback_terminal_command() -> &'static str {
    // Keep fallback path robust during PTY/terminal bring-up.
    // A direct shell on /dev/console guarantees interactivity.
    "/bin/sh"
}

#[allow(unreachable_code)]
#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    libc_lite::println("racinit: starting...");

    // Create a new session (PID 1 should be session leader)
    let _ = libc_lite::setsid();

    let mut engine = Engine::new();

    // Try to load unit files from /etc/racinit/
    engine.load_units_from("/etc/racinit");

    if engine.unit_count() == 0 {
        // No unit files found — create fallback units
        libc_lite::println("racinit: no unit files found, using fallback config");

        // base.target — the default target
        let base = Unit::new("base.target", UnitType::Target);
        engine.add_unit(base);

        // shell.service — spawn an interactive shell
        let mut shell = Unit::new("shell.service", UnitType::Service);
        shell.exec_start = String::from(fallback_terminal_command());
        shell.service_type = ServiceType::Simple;
        shell.restart = RestartPolicy::Always;
        shell.after.push(String::from("base.target"));
        shell.wanted_by.push(String::from("base.target"));
        engine.add_unit(shell);
    }

    // Start all units in dependency order
    libc_lite::println("racinit: starting services...");
    engine.start_all();

    // Enter supervisor loop (never returns)
    libc_lite::println("racinit: supervision active");
    engine.supervise();

    0
}
