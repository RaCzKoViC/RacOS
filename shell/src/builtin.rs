// racsh — Built-in commands
//
// Commands that must run in the shell process (not spawned):
// cd, pwd, exit, export, unset

extern crate alloc;

use crate::expand::Env;
use alloc::string::String;
use alloc::vec::Vec;

/// Result of a builtin execution.
pub enum BuiltinResult {
    /// Command completed with exit status.
    Ok(i32),
    /// Shell should exit with this status.
    Exit(i32),
    /// Not a builtin — caller should try external command.
    NotBuiltin,
}

/// Check if a command name is a builtin.
pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "cd" | "pwd"
            | "exit"
            | "export"
            | "unset"
            | "set"
            | "true"
            | "false"
            | "jobs"
            | "fg"
            | "bg"
            | "test"
            | "["
            | "read"
            | "type"
            | "kill"
            | "wait"
    )
}

/// Execute a builtin command.
///
/// `args` includes the command name as args[0].
/// `write_fn` is called with output bytes (for stdout).
pub fn run_builtin(args: &[String], env: &mut Env, write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::NotBuiltin;
    }

    match args[0].as_str() {
        "cd" => builtin_cd(args, env, write_fn),
        "pwd" => builtin_pwd(args, env, write_fn),
        "exit" => builtin_exit(args),
        "export" => builtin_export(args, env),
        "unset" => builtin_unset(args, env),
        "set" => builtin_set(env, write_fn),
        "true" => BuiltinResult::Ok(0),
        "false" => BuiltinResult::Ok(1),
        "jobs" => builtin_jobs(write_fn),
        "fg" => builtin_fg(args, write_fn),
        "bg" => builtin_bg(args, write_fn),
        "test" | "[" => builtin_test(args, write_fn),
        "read" => builtin_read(args, env, write_fn),
        "type" => builtin_type(args, env, write_fn),
        "kill" => builtin_kill(args, write_fn),
        "wait" => builtin_wait(args, write_fn),
        _ => BuiltinResult::NotBuiltin,
    }
}

fn builtin_cd(args: &[String], env: &mut Env, write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    let path = if args.len() > 1 {
        args[1].as_str()
    } else {
        env.get("HOME").unwrap_or("/")
    };

    // Build null-terminated path
    let mut path_buf = Vec::with_capacity(path.len() + 1);
    path_buf.extend_from_slice(path.as_bytes());
    path_buf.push(0);

    match libc_lite::chdir(&path_buf) {
        Ok(()) => {
            // Update PWD
            let mut cwd_buf = [0u8; 256];
            if let Ok(n) = libc_lite::getcwd(&mut cwd_buf) {
                let cwd = core::str::from_utf8(&cwd_buf[..n]).unwrap_or("/");
                env.set(String::from("PWD"), String::from(cwd));
            }
            BuiltinResult::Ok(0)
        }
        Err(_) => {
            write_fn(b"racsh: cd: ");
            write_fn(path.as_bytes());
            write_fn(b": No such file or directory\n");
            BuiltinResult::Ok(1)
        }
    }
}

fn builtin_pwd(_args: &[String], env: &mut Env, write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    let mut buf = [0u8; 256];
    match libc_lite::getcwd(&mut buf) {
        Ok(n) => {
            write_fn(&buf[..n]);
            write_fn(b"\n");
            // Update PWD while we're at it
            if let Ok(cwd) = core::str::from_utf8(&buf[..n]) {
                env.set(String::from("PWD"), String::from(cwd));
            }
            BuiltinResult::Ok(0)
        }
        Err(_) => {
            write_fn(b"/\n");
            BuiltinResult::Ok(0)
        }
    }
}

fn builtin_exit(args: &[String]) -> BuiltinResult {
    let code = if args.len() > 1 {
        parse_int(&args[1]).unwrap_or(0)
    } else {
        0
    };
    BuiltinResult::Exit(code)
}

fn builtin_export(args: &[String], env: &mut Env) -> BuiltinResult {
    // export NAME=VALUE or export NAME
    for arg in &args[1..] {
        if let Some(eq_pos) = arg.find('=') {
            let name = String::from(&arg[..eq_pos]);
            let value = String::from(&arg[eq_pos + 1..]);
            env.set(name, value);
        }
        // export NAME without value: mark for export (no-op for MVP)
    }
    BuiltinResult::Ok(0)
}

fn builtin_unset(args: &[String], env: &mut Env) -> BuiltinResult {
    for arg in &args[1..] {
        env.unset(arg);
    }
    BuiltinResult::Ok(0)
}

fn builtin_set(env: &Env, write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    // Print all variables
    // Access vars through the public API — iterate by checking known names
    // For now, just print a placeholder
    write_fn(b"(set: variable listing not yet implemented)\n");
    let _ = env;
    BuiltinResult::Ok(0)
}

fn parse_int(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (negative, start) = if bytes[0] == b'-' {
        (true, 1)
    } else {
        (false, 0)
    };
    let mut result: i32 = 0;
    for &b in &bytes[start..] {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.wrapping_mul(10).wrapping_add((b - b'0') as i32);
    }
    if negative {
        Some(-result)
    } else {
        Some(result)
    }
}

// ─────────────────────────────────────────────────
// Job control builtins
// ─────────────────────────────────────────────────

fn builtin_jobs(write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    use crate::exec;

    // First, poll completed jobs
    for job in exec::get_jobs() {
        if job.state == exec::JobState::Running {
            write_fn(b"[");
            write_u32(job.id, write_fn);
            write_fn(b"]  Running  ");
            write_fn(job.cmd.as_bytes());
            write_fn(b" &\n");
        }
    }
    BuiltinResult::Ok(0)
}

fn builtin_fg(args: &[String], write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    use crate::exec;

    let job_id = if args.len() > 1 {
        // Parse %N or N
        let s = args[1].as_str();
        let s = if s.starts_with('%') { &s[1..] } else { s };
        match parse_int(s) {
            Some(n) if n > 0 => n as u32,
            _ => {
                write_fn(b"fg: invalid job spec\n");
                return BuiltinResult::Ok(1);
            }
        }
    } else {
        // Default: last job
        match exec::get_jobs().last() {
            Some(j) => j.id,
            None => {
                write_fn(b"fg: no current job\n");
                return BuiltinResult::Ok(1);
            }
        }
    };

    match exec::find_job_pid(job_id) {
        Some(_pid) => {
            // Wait for the job's process
            let mut status: i32 = 0;
            let _ = libc_lite::wait(&mut status);
            exec::mark_job_done(job_id);
            exec::reap_jobs();
            BuiltinResult::Ok(status)
        }
        None => {
            write_fn(b"fg: no such job\n");
            BuiltinResult::Ok(1)
        }
    }
}

fn builtin_bg(args: &[String], write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    use crate::exec;

    let job_id = if args.len() > 1 {
        let s = args[1].as_str();
        let s = if s.starts_with('%') { &s[1..] } else { s };
        match parse_int(s) {
            Some(n) if n > 0 => n as u32,
            _ => {
                write_fn(b"bg: invalid job spec\n");
                return BuiltinResult::Ok(1);
            }
        }
    } else {
        match exec::get_jobs().last() {
            Some(j) => j.id,
            None => {
                write_fn(b"bg: no current job\n");
                return BuiltinResult::Ok(1);
            }
        }
    };

    match exec::find_job_pid(job_id) {
        Some(_pid) => {
            // Job already running in background
            write_fn(b"[");
            write_u32(job_id, write_fn);
            write_fn(b"]  Running\n");
            BuiltinResult::Ok(0)
        }
        None => {
            write_fn(b"bg: no such job\n");
            BuiltinResult::Ok(1)
        }
    }
}

fn write_u32(mut n: u32, write_fn: &dyn Fn(&[u8])) {
    if n == 0 {
        write_fn(b"0");
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    buf[..i].reverse();
    write_fn(&buf[..i]);
}

// ─────────────────────────────────────────────────
// test / [ builtin
// ─────────────────────────────────────────────────

fn builtin_test(args: &[String], write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    // Strip command name and optional trailing ']'
    let is_bracket = args[0] == "[";
    let test_args = &args[1..];
    let test_args = if is_bracket {
        if test_args.is_empty() || test_args.last().map(|s| s.as_str()) != Some("]") {
            write_fn(b"[: missing ]\n");
            return BuiltinResult::Ok(2);
        }
        &test_args[..test_args.len() - 1]
    } else {
        test_args
    };

    let result = eval_test(test_args);
    BuiltinResult::Ok(if result { 0 } else { 1 })
}

fn eval_test(args: &[String]) -> bool {
    match args.len() {
        0 => false,
        1 => !args[0].is_empty(), // test STRING — true if non-empty
        2 => {
            let op = args[0].as_str();
            let val = args[1].as_str();
            match op {
                "-z" => val.is_empty(),
                "-n" => !val.is_empty(),
                "-f" | "-e" => file_exists(val),
                "-d" => file_is_dir(val),
                "!" => !eval_test(&args[1..]),
                _ => false,
            }
        }
        3 => {
            let left = args[0].as_str();
            let op = args[1].as_str();
            let right = args[2].as_str();
            match op {
                "=" | "==" => left == right,
                "!=" => left != right,
                "-eq" => parse_i64(left) == parse_i64(right),
                "-ne" => parse_i64(left) != parse_i64(right),
                "-lt" => parse_i64(left) < parse_i64(right),
                "-le" => parse_i64(left) <= parse_i64(right),
                "-gt" => parse_i64(left) > parse_i64(right),
                "-ge" => parse_i64(left) >= parse_i64(right),
                _ => false,
            }
        }
        _ => false,
    }
}

fn file_exists(path: &str) -> bool {
    let mut buf = Vec::with_capacity(path.len() + 1);
    buf.extend_from_slice(path.as_bytes());
    buf.push(0);
    let mut stat_buf = [0u8; 80];
    libc_lite::stat(&buf, &mut stat_buf).is_ok()
}

fn file_is_dir(path: &str) -> bool {
    let mut buf = Vec::with_capacity(path.len() + 1);
    buf.extend_from_slice(path.as_bytes());
    buf.push(0);
    let mut stat_buf = [0u8; 80];
    if libc_lite::stat(&buf, &mut stat_buf).is_ok() {
        // st_mode is at offset 16 (u32), check S_IFDIR bit (0x4000)
        let mode = u32::from_le_bytes([stat_buf[16], stat_buf[17], stat_buf[18], stat_buf[19]]);
        mode & 0xF000 == 0x4000
    } else {
        false
    }
}

fn parse_i64(s: &str) -> i64 {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return 0;
    }
    let (neg, start) = if bytes[0] == b'-' {
        (true, 1)
    } else {
        (false, 0)
    };
    let mut r: i64 = 0;
    for &b in &bytes[start..] {
        if b.is_ascii_digit() {
            r = r.wrapping_mul(10).wrapping_add((b - b'0') as i64);
        }
    }
    if neg {
        -r
    } else {
        r
    }
}

// ─────────────────────────────────────────────────
// read builtin — read a line into a variable
// ─────────────────────────────────────────────────

fn builtin_read(args: &[String], env: &mut Env, write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    if args.len() < 2 {
        write_fn(b"read: missing variable name\n");
        return BuiltinResult::Ok(1);
    }

    let var_name = args[1].clone();
    let mut buf = [0u8; 256];
    match libc_lite::read(0, &mut buf) {
        Ok(0) => BuiltinResult::Ok(1), // EOF
        Ok(n) => {
            // Trim trailing newline
            let mut text = &buf[..n];
            if text.ends_with(b"\n") {
                text = &text[..n - 1];
            }
            if let Ok(s) = core::str::from_utf8(text) {
                env.set(var_name, String::from(s));
                BuiltinResult::Ok(0)
            } else {
                BuiltinResult::Ok(1)
            }
        }
        Err(_) => BuiltinResult::Ok(1),
    }
}

// ─────────────────────────────────────────────────
// type builtin — determine command type
// ─────────────────────────────────────────────────

fn builtin_type(args: &[String], env: &Env, write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    if args.len() < 2 {
        write_fn(b"type: missing command name\n");
        return BuiltinResult::Ok(1);
    }

    let cmd = args[1].as_str();

    // Check if builtin
    if is_builtin(cmd) {
        write_fn(cmd.as_bytes());
        write_fn(b" is a shell builtin\n");
        return BuiltinResult::Ok(0);
    }

    // Check if in PATH
    for dir in env.path_dirs() {
        let mut path = String::from(dir);
        path.push('/');
        path.push_str(cmd);

        let mut path_buf = alloc::vec::Vec::new();
        path_buf.extend_from_slice(path.as_bytes());
        path_buf.push(0);

        let mut stat_buf = [0u8; 80];
        if libc_lite::stat(&path_buf, &mut stat_buf).is_ok() {
            write_fn(cmd.as_bytes());
            write_fn(b" is ");
            write_fn(path.as_bytes());
            write_fn(b"\n");
            return BuiltinResult::Ok(0);
        }
    }

    write_fn(cmd.as_bytes());
    write_fn(b": not found\n");
    BuiltinResult::Ok(1)
}

// ─────────────────────────────────────────────────
// kill builtin — send signal to process
// ─────────────────────────────────────────────────

fn builtin_kill(args: &[String], write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    if args.len() < 2 {
        write_fn(b"kill: missing pid\n");
        return BuiltinResult::Ok(1);
    }

    let mut sig = 15i32; // SIGTERM default
    let mut pid_str = args[1].as_str();

    // Check for signal flag (-N or -SIGname)
    if args[1].starts_with('-') && args[1].len() > 1 {
        let sig_part = &args[1][1..];
        if let Some(sig_num) = parse_int(sig_part) {
            sig = sig_num;
            if args.len() > 2 {
                pid_str = args[2].as_str();
            } else {
                write_fn(b"kill: missing pid\n");
                return BuiltinResult::Ok(1);
            }
        } else {
            // Try to parse signal name (basic)
            sig = match sig_part {
                "TERM" | "term" => 15,
                "KILL" | "kill" => 9,
                "INT" | "int" => 2,
                "STOP" | "stop" => 19,
                "CONT" | "cont" => 18,
                "HUP" | "hup" => 1,
                _ => {
                    write_fn(b"kill: unknown signal: ");
                    write_fn(sig_part.as_bytes());
                    write_fn(b"\n");
                    return BuiltinResult::Ok(1);
                }
            };
            if args.len() > 2 {
                pid_str = args[2].as_str();
            } else {
                write_fn(b"kill: missing pid\n");
                return BuiltinResult::Ok(1);
            }
        }
    }

    // Parse PID
    if let Some(pid) = parse_int(pid_str) {
        match libc_lite::kill(pid, sig) {
            Ok(()) => BuiltinResult::Ok(0),
            Err(_) => {
                write_fn(b"kill: no such process\n");
                BuiltinResult::Ok(1)
            }
        }
    } else {
        write_fn(b"kill: invalid pid: ");
        write_fn(pid_str.as_bytes());
        write_fn(b"\n");
        BuiltinResult::Ok(1)
    }
}

// ─────────────────────────────────────────────────
// wait builtin — wait for background jobs
// ─────────────────────────────────────────────────

fn builtin_wait(args: &[String], write_fn: &dyn Fn(&[u8])) -> BuiltinResult {
    if args.len() < 2 {
        // No PID specified — wait for all background jobs (not implemented)
        write_fn(b"wait: not implemented without pid\n");
        return BuiltinResult::Ok(1);
    }

    // Parse PID
    if let Some(pid) = parse_int(args[1].as_str()) {
        let mut status = 0i32;
        match libc_lite::wait(&mut status) {
            Ok(returned_pid) => {
                if returned_pid == pid {
                    BuiltinResult::Ok(0)
                } else {
                    // Some other process exited
                    BuiltinResult::Ok(0)
                }
            }
            Err(_) => BuiltinResult::Ok(1),
        }
    } else {
        write_fn(b"wait: invalid pid\n");
        BuiltinResult::Ok(1)
    }
}
