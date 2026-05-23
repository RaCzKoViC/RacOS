// RacInit — Service engine
//
// Loads unit files from /etc/racinit/, resolves dependencies,
// and starts services in topological order.
// Manages service lifecycle: start, stop, restart on failure.

extern crate alloc;

use crate::{parse_unit, RestartPolicy, ServiceType, Unit, UnitState, UnitType};
use alloc::string::String;
use alloc::vec::Vec;

/// Maximum number of units the engine can manage.
const _MAX_UNITS: usize = 32;

/// The init engine — holds all loaded units and manages their lifecycle.
pub struct Engine {
    units: Vec<Unit>,
    /// PIDs of running services: (unit_index, pid).
    pids: Vec<(usize, i32)>,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            units: Vec::new(),
            pids: Vec::new(),
        }
    }

    /// Load unit files from a directory path.
    /// Reads all files matching *.service, *.target from the directory.
    pub fn load_units_from(&mut self, dir: &str) {
        // Read directory entries via VFS
        // For MVP: try to open known unit files
        let known_units = ["console.service", "shell.service", "base.target"];

        for name in &known_units {
            let mut path = String::with_capacity(dir.len() + 1 + name.len() + 1);
            path.push_str(dir);
            if !dir.ends_with('/') {
                path.push('/');
            }
            path.push_str(name);
            path.push('\0');

            if let Some(content) = read_file_to_string(path.as_bytes()) {
                match parse_unit(name, &content) {
                    Ok(unit) => {
                        self.units.push(unit);
                    }
                    Err(e) => {
                        log("racinit: parse error for ");
                        log(name);
                        log(": ");
                        log(e);
                        log("\n");
                    }
                }
            }
        }
    }

    /// Add a unit directly (for built-in/fallback units).
    pub fn add_unit(&mut self, unit: Unit) {
        self.units.push(unit);
    }

    /// Get the number of loaded units.
    pub fn unit_count(&self) -> usize {
        self.units.len()
    }

    /// Resolve dependencies and return a start order (topological sort).
    /// Returns indices into self.units in the order they should be started.
    pub fn resolve_start_order(&self) -> Vec<usize> {
        let n = self.units.len();
        if n == 0 {
            return Vec::new();
        }

        // Build adjacency: after[i] = set of indices that unit i must start after
        let mut after_deps: Vec<Vec<usize>> = Vec::with_capacity(n);
        for _ in 0..n {
            after_deps.push(Vec::new());
        }

        for (i, unit) in self.units.iter().enumerate() {
            for dep_name in &unit.after {
                if let Some(j) = self.find_unit(dep_name) {
                    after_deps[i].push(j);
                }
            }
            // Requires implies After (if not explicitly listed)
            for dep_name in &unit.requires {
                if let Some(j) = self.find_unit(dep_name) {
                    if !after_deps[i].contains(&j) {
                        after_deps[i].push(j);
                    }
                }
            }
        }

        // Topological sort (Kahn's algorithm)
        let mut in_degree = Vec::with_capacity(n);
        for _ in 0..n {
            in_degree.push(0usize);
        }

        for _deps in &after_deps {
            // This unit depends on deps → deps must come first
            // In-degree counts how many things must come before
        }

        // Build reverse edges: for each (i depends on j), unit j has an "enables" edge to i
        let mut enables: Vec<Vec<usize>> = Vec::with_capacity(n);
        for _ in 0..n {
            enables.push(Vec::new());
        }

        for (i, deps) in after_deps.iter().enumerate() {
            in_degree.push(0); // extra safety
            for &j in deps {
                enables[j].push(i);
            }
        }

        // Recalculate in_degree
        for i in 0..n {
            in_degree[i] = after_deps[i].len();
        }

        let mut queue: Vec<usize> = Vec::new();
        for i in 0..n {
            if in_degree[i] == 0 {
                queue.push(i);
            }
        }

        let mut order: Vec<usize> = Vec::with_capacity(n);
        while let Some(idx) = queue.pop() {
            order.push(idx);
            for &next in &enables[idx] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push(next);
                }
            }
        }

        // If order.len() < n, there's a cycle — append remaining units anyway
        if order.len() < n {
            for i in 0..n {
                if !order.contains(&i) {
                    order.push(i);
                }
            }
        }

        order
    }

    /// Start all units in dependency order.
    pub fn start_all(&mut self) {
        let order = self.resolve_start_order();

        for &idx in &order {
            self.start_unit(idx);
        }
    }

    /// Start a single unit by index.
    fn start_unit(&mut self, idx: usize) {
        let unit = &mut self.units[idx];

        match unit.unit_type {
            UnitType::Target => {
                // Targets are just milestones — mark as active
                log("racinit: reached target ");
                log(&unit.name);
                log("\n");
                unit.state = UnitState::Active;
            }
            UnitType::Service => {
                if unit.exec_start.is_empty() {
                    unit.state = UnitState::Active;
                    return;
                }

                log("racinit: starting ");
                log(&unit.name);
                log(" -> ");
                log(&unit.exec_start);
                log("\n");

                unit.state = UnitState::Starting;

                // Build null-terminated path
                let mut path_buf = Vec::with_capacity(unit.exec_start.len() + 1);
                path_buf.extend_from_slice(unit.exec_start.as_bytes());
                path_buf.push(0);

                match libc_lite::spawn(&path_buf) {
                    Ok(pid) => {
                        unit.state = UnitState::Active;
                        self.pids.push((idx, pid));
                        log("racinit: started PID ");
                        log_i32(pid);
                        log("\n");

                        // For oneshot: wait immediately
                        if unit.service_type == ServiceType::Oneshot {
                            let mut status: i32 = 0;
                            let _ = libc_lite::wait(&mut status);
                            if status != 0 {
                                self.units[idx].state = UnitState::Failed;
                            } else {
                                self.units[idx].state = UnitState::Active;
                            }
                            // Remove from pid list
                            self.pids.retain(|&(i, _)| i != idx);
                        }
                    }
                    Err(_) => {
                        unit.state = UnitState::Failed;
                        log("racinit: FAILED to start ");
                        log(&unit.name);
                        log("\n");
                    }
                }
            }
            _ => {
                // Mount, Timer, Device — not yet implemented
                unit.state = UnitState::Active;
            }
        }
    }

    /// Main loop: wait for child processes and handle restarts.
    /// This never returns (PID 1 runs forever).
    pub fn supervise(&mut self) -> ! {
        loop {
            let mut status: i32 = 0;
            match libc_lite::wait(&mut status) {
                Ok(pid) => {
                    // Find which unit this PID belongs to
                    if let Some(pos) = self.pids.iter().position(|&(_, p)| p == pid) {
                        let (unit_idx, _) = self.pids[pos];
                        self.pids.remove(pos);

                        let unit = &mut self.units[unit_idx];
                        let should_restart = match unit.restart {
                            RestartPolicy::Always => true,
                            RestartPolicy::OnFailure => status != 0,
                            RestartPolicy::OnAbnormal => status != 0,
                            RestartPolicy::No => false,
                        };

                        if should_restart {
                            log("racinit: restarting ");
                            log(&unit.name);
                            log("\n");
                            unit.state = UnitState::Stopped;
                            self.start_unit(unit_idx);
                        } else {
                            if status == 0 {
                                unit.state = UnitState::Stopped;
                            } else {
                                unit.state = UnitState::Failed;
                            }
                            log("racinit: ");
                            log(&unit.name);
                            log(" exited with status ");
                            log_i32(status);
                            log("\n");
                        }
                    }
                }
                Err(_) => {
                    // No children or error — yield CPU
                    // In a real system we'd use a blocking wait syscall
                    // For now, just loop slowly
                }
            }
        }
    }

    fn find_unit(&self, name: &str) -> Option<usize> {
        self.units.iter().position(|u| u.name == name)
    }
}

/// Read a file from VFS into a String.
fn read_file_to_string(path: &[u8]) -> Option<String> {
    let fd = libc_lite::open(path, 0, 0).ok()?;
    let mut buf = [0u8; 2048];
    let mut content = String::new();
    loop {
        match libc_lite::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                    content.push_str(s);
                }
            }
            Err(_) => break,
        }
    }
    let _ = libc_lite::close(fd);
    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

fn log(s: &str) {
    let _ = libc_lite::write(1, s.as_bytes());
}

fn log_i32(val: i32) {
    let mut buf = [0u8; 12];
    let s = format_i32(val, &mut buf);
    log(s);
}

fn format_i32(val: i32, buf: &mut [u8; 12]) -> &str {
    let (negative, mut v) = if val < 0 {
        (true, (-(val as i64)) as u32)
    } else {
        (false, val as u32)
    };

    let mut pos = 12;
    if v == 0 {
        pos -= 1;
        buf[pos] = b'0';
    } else {
        while v > 0 {
            pos -= 1;
            buf[pos] = b'0' + (v % 10) as u8;
            v /= 10;
        }
    }
    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }

    core::str::from_utf8(&buf[pos..]).unwrap_or("?")
}
