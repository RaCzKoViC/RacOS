// racsh — Execution engine
//
// Walks the AST and executes commands using libc-lite syscalls.
// Handles simple commands, pipelines, sequences, and/or lists,
// redirections, and control flow (if/while/for).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use crate::ast::{AstNode, Assignment, Redirect, RedirectOp, SequenceOp, Word};
use crate::expand::{self, Env};
use crate::builtin::{self, BuiltinResult};

// ─────────────────────────────────────────────────
// Job table
// ─────────────────────────────────────────────────

/// State of a background job.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
}

/// A background job entry.
#[derive(Clone)]
pub struct Job {
    pub id: u32,
    pub pid: u32,
    pub state: JobState,
    pub cmd: String,
}

/// Global job table (simple Vec, max ~32 jobs).
static mut JOBS: Option<Vec<Job>> = None;

fn jobs_table() -> &'static mut Vec<Job> {
    unsafe {
        let ptr = core::ptr::addr_of_mut!(JOBS);
        if (*ptr).is_none() {
            *ptr = Some(Vec::new());
        }
        (*ptr).as_mut().unwrap()
    }
}

/// Add a background job, return job id.
pub fn add_job(pid: u32, cmd: &str) -> u32 {
    let jobs = jobs_table();
    let id = jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
    jobs.push(Job {
        id,
        pid,
        state: JobState::Running,
        cmd: String::from(cmd),
    });
    id
}

/// Get a reference to the job list.
pub fn get_jobs() -> &'static [Job] {
    jobs_table()
}

/// Remove completed jobs from the table.
pub fn reap_jobs() {
    let jobs = jobs_table();
    jobs.retain(|j| j.state != JobState::Done);
}

/// Mark a job as done by PID.
pub fn mark_job_done(pid: u32) {
    let jobs = jobs_table();
    for j in jobs.iter_mut() {
        if j.pid == pid {
            j.state = JobState::Done;
        }
    }
}

/// Find a job by id, return its PID.
pub fn find_job_pid(job_id: u32) -> Option<u32> {
    jobs_table().iter().find(|j| j.id == job_id).map(|j| j.pid)
}

/// Execute an AST node, returning exit status.
pub fn execute(node: &AstNode, env: &mut Env) -> i32 {
    match node {
        AstNode::Program { commands } => {
            let mut status = 0;
            for cmd in commands {
                status = execute(cmd, env);
                env.last_status = status;
            }
            status
        }

        AstNode::SimpleCommand { assignments, words, redirects } => {
            exec_simple(assignments, words, redirects, env)
        }

        AstNode::Pipeline { commands, negated } => {
            let status = exec_pipeline(commands, env);
            if *negated {
                if status == 0 { 1 } else { 0 }
            } else {
                status
            }
        }

        AstNode::Sequence { left, right, op } => {
            match op {
                SequenceOp::Semi => {
                    let left_status = execute(left, env);
                    env.last_status = left_status;
                    let s = execute(right, env);
                    env.last_status = s;
                    s
                }
                SequenceOp::Background => {
                    // Spawn left command in background, then run right
                    exec_background(left, env);
                    let s = execute(right, env);
                    env.last_status = s;
                    s
                }
            }
        }

        AstNode::And { left, right } => {
            let s = execute(left, env);
            env.last_status = s;
            if s == 0 {
                let s2 = execute(right, env);
                env.last_status = s2;
                s2
            } else {
                s
            }
        }

        AstNode::Or { left, right } => {
            let s = execute(left, env);
            env.last_status = s;
            if s != 0 {
                let s2 = execute(right, env);
                env.last_status = s2;
                s2
            } else {
                s
            }
        }

        AstNode::If { condition, then_body, elif_parts, else_body } => {
            let cond = execute(condition, env);
            env.last_status = cond;
            if cond == 0 {
                return execute(then_body, env);
            }
            for (elif_cond, elif_body) in elif_parts {
                let c = execute(elif_cond, env);
                env.last_status = c;
                if c == 0 {
                    return execute(elif_body, env);
                }
            }
            if let Some(else_body) = else_body {
                execute(else_body, env)
            } else {
                0
            }
        }

        AstNode::While { condition, body } => {
            let mut status = 0;
            loop {
                let c = execute(condition, env);
                env.last_status = c;
                if c != 0 {
                    break;
                }
                status = execute(body, env);
                env.last_status = status;
            }
            status
        }

        AstNode::For { var, words, body } => {
            let items: Vec<String> = if let Some(word_list) = words {
                word_list.iter()
                    .flat_map(|w| expand::expand_word_list(w, env))
                    .collect()
            } else {
                // No word list — use positional parameters (not implemented)
                Vec::new()
            };
            let mut status = 0;
            for item in &items {
                env.set(var.clone(), item.clone());
                status = execute(body, env);
                env.last_status = status;
            }
            status
        }

        AstNode::Case { word, items } => {
            let value = expand::expand_word(word, env);
            for item in items {
                for pattern in &item.patterns {
                    let pat = expand::expand_word(pattern, env);
                    if pat == value || pat == "*" {
                        if let Some(body) = &item.body {
                            let s = execute(body, env);
                            env.last_status = s;
                            return s;
                        }
                        return 0;
                    }
                }
            }
            0
        }

        AstNode::Subshell { body, redirects: _ } => {
            // True subshell requires fork — for MVP, execute in-process
            execute(body, env)
        }

        AstNode::BraceGroup { body, redirects: _ } => {
            execute(body, env)
        }

        AstNode::FunctionDef { name: _, body: _ } => {
            // TODO: function table
            0
        }
    }
}

/// Execute a simple command (assignments + words + redirects).
fn exec_simple(
    assignments: &[Assignment],
    words: &[Word],
    redirects: &[Redirect],
    env: &mut Env,
) -> i32 {
    // If no words — just set variables
    if words.is_empty() {
        for a in assignments {
            let val = expand::expand_word(&a.value, env);
            env.set(a.name.clone(), val);
        }
        return 0;
    }

    // Expand all words with glob expansion support
    let expanded: Vec<String> = words.iter()
        .flat_map(|w| expand::expand_word_list(w, env))
        .collect();

    if expanded.is_empty() || expanded[0].is_empty() {
        return 0;
    }

    // Check builtins first
    match builtin::run_builtin(&expanded, env, &|data| {
        let _ = libc_lite::write(1, data);
    }) {
        BuiltinResult::Ok(status) => return status,
        BuiltinResult::Exit(code) => libc_lite::exit(code),
        BuiltinResult::NotBuiltin => {}
    }

    // External command — resolve path and spawn
    let cmd_name = &expanded[0];
    let path = resolve_command(cmd_name, env);

    match path {
        Some(full_path) => {
            // Set up redirections, spawn, wait
            exec_external(&full_path, &expanded, redirects, env)
        }
        None => {
            let _ = libc_lite::write(2, b"racsh: ");
            let _ = libc_lite::write(2, cmd_name.as_bytes());
            let _ = libc_lite::write(2, b": command not found\n");
            127
        }
    }
}

/// Resolve a command name to a full path by searching PATH.
fn resolve_command(name: &str, env: &Env) -> Option<String> {
    // If name contains '/', use as-is
    if name.contains('/') {
        return Some(String::from(name));
    }

    // Search PATH directories
    for dir in env.path_dirs() {
        let mut path = String::with_capacity(dir.len() + 1 + name.len());
        path.push_str(dir);
        if !dir.ends_with('/') {
            path.push('/');
        }
        path.push_str(name);

        // Check if file exists via stat
        let mut path_buf = Vec::with_capacity(path.len() + 1);
        path_buf.extend_from_slice(path.as_bytes());
        path_buf.push(0);

        let mut stat_buf = [0u8; 80];
        if libc_lite::stat(&path_buf, &mut stat_buf).is_ok() {
            return Some(path);
        }
    }

    None
}

/// Execute an external command with redirections.
fn exec_external(path: &str, args: &[String], redirects: &[Redirect], env: &Env) -> i32 {
    // Build null-terminated path
    let mut path_buf = Vec::with_capacity(path.len() + 1);
    path_buf.extend_from_slice(path.as_bytes());
    path_buf.push(0);

    // Build argv: array of null-terminated strings
    let mut arg_bufs: Vec<Vec<u8>> = Vec::with_capacity(args.len());
    for arg in args {
        let mut buf = Vec::with_capacity(arg.len() + 1);
        buf.extend_from_slice(arg.as_bytes());
        buf.push(0); // null terminator
        arg_bufs.push(buf);
    }
    // Build pointer array (null-terminated)
    let mut argv_ptrs: Vec<*const u8> = Vec::with_capacity(arg_bufs.len() + 1);
    for buf in &arg_bufs {
        argv_ptrs.push(buf.as_ptr());
    }
    argv_ptrs.push(core::ptr::null()); // NULL terminator

    // Set up redirections before spawn. If any redirect fails to open we
    // refuse to run the command — otherwise the redirect quietly evaporates
    // and the user gets the false impression their `>` write succeeded.
    let saved_fds = match apply_redirects(redirects, env) {
        Ok(s) => s,
        Err(s) => {
            restore_fds(&s);
            return 1;
        }
    };

    let result = match libc_lite::spawn_args(&path_buf, &argv_ptrs) {
        Ok(_child_pid) => {
            // Wait for child
            let mut status: i32 = 0;
            let _ = libc_lite::wait(&mut status);
            status
        }
        Err(_e) => {
            let _ = libc_lite::write(2, b"racsh: ");
            let _ = libc_lite::write(2, path.as_bytes());
            let _ = libc_lite::write(2, b": cannot execute\n");
            126
        }
    };

    // Restore original fds
    restore_fds(&saved_fds);

    result
}

/// Apply redirections. Returns `Ok(saved_fds)` on success; `Err(saved_fds)`
/// if any open/dup failed so the caller can restore what we already redirected
/// and abort the command instead of silently dropping the redirect.
fn apply_redirects(redirects: &[Redirect], env: &Env) -> Result<Vec<(i32, i32)>, Vec<(i32, i32)>> {
    let mut saved = Vec::new();

    fn report_open_fail(target: &str, errno: i64) {
        let _ = libc_lite::write(2, b"racsh: ");
        let _ = libc_lite::write(2, target.as_bytes());
        let _ = libc_lite::write(2, b": cannot open (errno ");
        // Errors come back negative from libc_lite; print the absolute value.
        let v = if errno < 0 { (-errno) as u64 } else { errno as u64 };
        let mut digits = [0u8; 8];
        let mut i = 0usize;
        let mut t = v;
        if t == 0 { digits[0] = b'0'; i = 1; }
        while t > 0 { digits[i] = b'0' + (t % 10) as u8; t /= 10; i += 1; }
        let mut buf = [0u8; 8];
        for j in 0..i { buf[j] = digits[i - 1 - j]; }
        let _ = libc_lite::write(2, &buf[..i]);
        let _ = libc_lite::write(2, b")\n");
    }

    for redir in redirects {
        let target = expand::expand_word(&redir.target, env);

        match redir.op {
            RedirectOp::Output | RedirectOp::Append => {
                let fd = redir.fd.unwrap_or(1); // default stdout
                let flags = if matches!(redir.op, RedirectOp::Append) {
                    0x441 // O_WRONLY | O_CREAT | O_APPEND
                } else {
                    0x241 // O_WRONLY | O_CREAT | O_TRUNC
                };

                let mut path_buf = Vec::with_capacity(target.len() + 1);
                path_buf.extend_from_slice(target.as_bytes());
                path_buf.push(0);

                match libc_lite::open(&path_buf, flags, 0o644) {
                    Ok(new_fd) => {
                        if let Ok(saved_fd) = libc_lite::dup(fd) {
                            saved.push((fd, saved_fd));
                        }
                        let _ = libc_lite::dup2(new_fd, fd);
                        let _ = libc_lite::close(new_fd);
                    }
                    Err(e) => {
                        report_open_fail(&target, e);
                        return Err(saved);
                    }
                }
            }
            RedirectOp::Input => {
                let fd = redir.fd.unwrap_or(0); // default stdin
                let mut path_buf = Vec::with_capacity(target.len() + 1);
                path_buf.extend_from_slice(target.as_bytes());
                path_buf.push(0);

                match libc_lite::open(&path_buf, 0, 0) { // O_RDONLY
                    Ok(new_fd) => {
                        if let Ok(saved_fd) = libc_lite::dup(fd) {
                            saved.push((fd, saved_fd));
                        }
                        let _ = libc_lite::dup2(new_fd, fd);
                        let _ = libc_lite::close(new_fd);
                    }
                    Err(e) => {
                        report_open_fail(&target, e);
                        return Err(saved);
                    }
                }
            }
            RedirectOp::DupOutput => {
                let fd = redir.fd.unwrap_or(1);
                if let Some(src_fd) = parse_fd(&target) {
                    if let Ok(saved_fd) = libc_lite::dup(fd) {
                        saved.push((fd, saved_fd));
                    }
                    let _ = libc_lite::dup2(src_fd, fd);
                }
            }
            RedirectOp::DupInput => {
                let fd = redir.fd.unwrap_or(0);
                if let Some(src_fd) = parse_fd(&target) {
                    if let Ok(saved_fd) = libc_lite::dup(fd) {
                        saved.push((fd, saved_fd));
                    }
                    let _ = libc_lite::dup2(src_fd, fd);
                }
            }
        }
    }

    Ok(saved)
}

/// Restore saved file descriptors.
fn restore_fds(saved: &[(i32, i32)]) {
    for &(fd, saved_fd) in saved.iter().rev() {
        let _ = libc_lite::dup2(saved_fd, fd);
        let _ = libc_lite::close(saved_fd);
    }
}

/// Parse a string as a file descriptor number.
fn parse_fd(s: &str) -> Option<i32> {
    let mut result: i32 = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result * 10 + (b - b'0') as i32;
    }
    Some(result)
}

/// Execute a command in the background (spawn without waiting).
fn exec_background(node: &AstNode, env: &mut Env) {
    // For SimpleCommand with external commands, spawn without waiting
    if let AstNode::SimpleCommand { assignments: _, words, redirects: _ } = node {
        let expanded: Vec<String> = words.iter().map(|w| expand::expand_word(w, env)).collect();
        if expanded.is_empty() || expanded[0].is_empty() {
            return;
        }
        if let Some(path) = resolve_command(&expanded[0], env) {
            let mut path_buf = Vec::with_capacity(path.len() + 1);
            path_buf.extend_from_slice(path.as_bytes());
            path_buf.push(0);
            // Build argv
            let mut arg_bufs: Vec<Vec<u8>> = Vec::with_capacity(expanded.len());
            for arg in &expanded {
                let mut buf = Vec::with_capacity(arg.len() + 1);
                buf.extend_from_slice(arg.as_bytes());
                buf.push(0);
                arg_bufs.push(buf);
            }
            let mut argv_ptrs: Vec<*const u8> = Vec::with_capacity(arg_bufs.len() + 1);
            for buf in &arg_bufs {
                argv_ptrs.push(buf.as_ptr());
            }
            argv_ptrs.push(core::ptr::null());
            match libc_lite::spawn_args(&path_buf, &argv_ptrs) {
                Ok(pid) => {
                    let job_id = add_job(pid as u32, &expanded[0]);
                    let _ = libc_lite::write(1, b"[");
                    print_u32(job_id);
                    let _ = libc_lite::write(1, b"] ");
                    print_u32(pid as u32);
                    let _ = libc_lite::write(1, b"\n");
                }
                Err(_) => {
                    let _ = libc_lite::write(2, b"racsh: cannot spawn background job\n");
                }
            }
        } else {
            let _ = libc_lite::write(2, b"racsh: ");
            let _ = libc_lite::write(2, expanded[0].as_bytes());
            let _ = libc_lite::write(2, b": command not found\n");
        }
    } else {
        // For non-simple commands, just execute synchronously as fallback
        let _ = execute(node, env);
    }
}

fn print_u32(mut n: u32) {
    if n == 0 {
        let _ = libc_lite::write(1, b"0");
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
    let _ = libc_lite::write(1, &buf[..i]);
}

/// Execute a pipeline of commands.
fn exec_pipeline(commands: &[AstNode], env: &mut Env) -> i32 {
    if commands.len() == 1 {
        return execute(&commands[0], env);
    }

    // For a proper pipeline, spawn all commands concurrently:
    //   cmd1 | cmd2 | cmd3
    // Each pair is connected by a pipe. All commands run in parallel
    // and we wait for all of them. The exit status is from the last command.

    let mut child_pids: Vec<i32> = Vec::new();
    let mut prev_read_fd: Option<i32> = None;

    for (i, cmd) in commands.iter().enumerate() {
        let is_last = i == commands.len() - 1;

        // Create pipe for non-last commands
        let mut pipe_fds = [0i32; 2];
        if !is_last {
            if libc_lite::pipe(&mut pipe_fds).is_err() {
                let _ = libc_lite::write(2, b"racsh: pipe failed\n");
                return 1;
            }
        }

        // For external commands in the pipeline, we need to set up
        // redirections and spawn the process
        if let AstNode::SimpleCommand { assignments: _, words, redirects } = cmd {
            let expanded: Vec<String> = words.iter()
                .flat_map(|w| expand::expand_word_list(w, env))
                .collect();

            if expanded.is_empty() || expanded[0].is_empty() {
                if let Some(rd) = prev_read_fd { let _ = libc_lite::close(rd); }
                prev_read_fd = if !is_last { Some(pipe_fds[0]) } else { None };
                if !is_last { let _ = libc_lite::close(pipe_fds[1]); }
                continue;
            }

            // Check builtins — run in-process with redirected fds
            let is_builtin = builtin::is_builtin(&expanded[0]);

            if is_builtin {
                // Set up stdin from previous pipe
                let saved_stdin = if let Some(read_fd) = prev_read_fd.take() {
                    let saved = libc_lite::dup(0).ok();
                    let _ = libc_lite::dup2(read_fd, 0);
                    let _ = libc_lite::close(read_fd);
                    saved
                } else { None };

                // Set up stdout to current pipe
                let saved_stdout = if !is_last {
                    let saved = libc_lite::dup(1).ok();
                    let _ = libc_lite::dup2(pipe_fds[1], 1);
                    let _ = libc_lite::close(pipe_fds[1]);
                    saved
                } else { None };

                let _ = execute(cmd, env);

                if let Some(fd) = saved_stdout { let _ = libc_lite::dup2(fd, 1); let _ = libc_lite::close(fd); }
                if let Some(fd) = saved_stdin { let _ = libc_lite::dup2(fd, 0); let _ = libc_lite::close(fd); }
            } else {
                // External command — spawn with pipes
                let cmd_name = &expanded[0];
                if let Some(path) = resolve_command(cmd_name, env) {
                    let mut path_buf = Vec::with_capacity(path.len() + 1);
                    path_buf.extend_from_slice(path.as_bytes());
                    path_buf.push(0);

                    let mut arg_bufs: Vec<Vec<u8>> = Vec::with_capacity(expanded.len());
                    for arg in &expanded {
                        let mut buf = Vec::with_capacity(arg.len() + 1);
                        buf.extend_from_slice(arg.as_bytes());
                        buf.push(0);
                        arg_bufs.push(buf);
                    }
                    let mut argv_ptrs: Vec<*const u8> = Vec::with_capacity(arg_bufs.len() + 1);
                    for buf in &arg_bufs {
                        argv_ptrs.push(buf.as_ptr());
                    }
                    argv_ptrs.push(core::ptr::null());

                    // Redirect stdin from previous pipe
                    let saved_stdin = if let Some(read_fd) = prev_read_fd.take() {
                        let saved = libc_lite::dup(0).ok();
                        let _ = libc_lite::dup2(read_fd, 0);
                        let _ = libc_lite::close(read_fd);
                        saved
                    } else { None };

                    // Redirect stdout to current pipe
                    let saved_stdout = if !is_last {
                        let saved = libc_lite::dup(1).ok();
                        let _ = libc_lite::dup2(pipe_fds[1], 1);
                        let _ = libc_lite::close(pipe_fds[1]);
                        saved
                    } else { None };

                    // Apply redirects from the command. If they fail, skip
                    // the spawn — silent drop would let a failed `>` look
                    // like success.
                    let saved_redirects = match apply_redirects(redirects, env) {
                        Ok(s) => Some(s),
                        Err(s) => { restore_fds(&s); None }
                    };

                    if let Some(saved_redirects) = saved_redirects {
                        match libc_lite::spawn_args(&path_buf, &argv_ptrs) {
                            Ok(pid) => { child_pids.push(pid); }
                            Err(_) => {
                                let _ = libc_lite::write(2, b"racsh: ");
                                let _ = libc_lite::write(2, cmd_name.as_bytes());
                                let _ = libc_lite::write(2, b": cannot execute\n");
                            }
                        }
                        restore_fds(&saved_redirects);
                    }
                    if let Some(fd) = saved_stdout { let _ = libc_lite::dup2(fd, 1); let _ = libc_lite::close(fd); }
                    if let Some(fd) = saved_stdin { let _ = libc_lite::dup2(fd, 0); let _ = libc_lite::close(fd); }
                } else {
                    let _ = libc_lite::write(2, b"racsh: ");
                    let _ = libc_lite::write(2, cmd_name.as_bytes());
                    let _ = libc_lite::write(2, b": command not found\n");
                    if let Some(rd) = prev_read_fd.take() { let _ = libc_lite::close(rd); }
                    if !is_last { let _ = libc_lite::close(pipe_fds[1]); }
                }
            }
        } else {
            // Non-simple commands in pipeline: execute sequentially as fallback
            let saved_stdin = if let Some(read_fd) = prev_read_fd.take() {
                let saved = libc_lite::dup(0).ok();
                let _ = libc_lite::dup2(read_fd, 0);
                let _ = libc_lite::close(read_fd);
                saved
            } else { None };

            let saved_stdout = if !is_last {
                let saved = libc_lite::dup(1).ok();
                let _ = libc_lite::dup2(pipe_fds[1], 1);
                let _ = libc_lite::close(pipe_fds[1]);
                saved
            } else { None };

            let _ = execute(cmd, env);

            if let Some(fd) = saved_stdout { let _ = libc_lite::dup2(fd, 1); let _ = libc_lite::close(fd); }
            if let Some(fd) = saved_stdin { let _ = libc_lite::dup2(fd, 0); let _ = libc_lite::close(fd); }
        }

        // Save the read end for the next command
        if !is_last {
            prev_read_fd = Some(pipe_fds[0]);
        }
    }

    // Wait for all spawned children
    let mut last_status = 0i32;
    for _pid in &child_pids {
        let mut status: i32 = 0;
        let _ = libc_lite::wait(&mut status);
        last_status = status;
    }

    last_status
}
