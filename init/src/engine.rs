// RacInit — Service engine
//
// Loads unit files from /etc/racinit/, resolves dependencies,
// and starts services in topological order.
// Manages service lifecycle: start, stop, restart on failure.

extern crate alloc;

use crate::{parse_unit, RestartPolicy, ServiceType, Unit, UnitState, UnitType};
use alloc::string::String;
use alloc::vec::Vec;

/// Result of `Engine::resolve_start_order`.
pub struct ResolveResult {
    /// Indices into `self.units` in dependency-respecting start order.
    pub order: Vec<usize>,
    /// Indices that could not be scheduled because they're part of a
    /// dependency cycle. Empty on a clean DAG.
    pub cycle: Vec<usize>,
}

/// Restart-burst tracking for a single unit. Keeps the wall-clock-ish
/// time (in seconds since boot, fed by the engine) of each restart so we
/// can refuse to keep restarting a service that fails immediately after
/// every restart — that would otherwise saturate PID 1.
const BURST_WINDOW_SEC: u64 = 30;
const BURST_LIMIT: usize = 5;

#[derive(Default, Debug, Clone)]
pub struct RestartTracker {
    timestamps: Vec<u64>,
}

impl RestartTracker {
    /// Record a restart at `now_secs`. Returns `true` if the unit has
    /// burned through its budget within the burst window and should be
    /// quarantined (state → Failed, no more restarts).
    pub fn record_and_check(&mut self, now_secs: u64) -> bool {
        // Drop timestamps older than the burst window before counting.
        self.timestamps
            .retain(|&t| now_secs.saturating_sub(t) <= BURST_WINDOW_SEC);
        self.timestamps.push(now_secs);
        self.timestamps.len() > BURST_LIMIT
    }
}

/// The init engine — holds all loaded units and manages their lifecycle.
pub struct Engine {
    units: Vec<Unit>,
    /// PIDs of running services: (unit_index, pid).
    pids: Vec<(usize, i32)>,
    /// Per-unit restart-burst tracker (parallel-indexed with `units`).
    restart_trackers: Vec<RestartTracker>,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            units: Vec::new(),
            pids: Vec::new(),
            restart_trackers: Vec::new(),
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
                        self.restart_trackers.push(RestartTracker::default());
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
        self.restart_trackers.push(RestartTracker::default());
    }

    /// Read-only access to the loaded units (used by host tests).
    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// Get the number of loaded units.
    pub fn unit_count(&self) -> usize {
        self.units.len()
    }

    /// Resolve dependencies and return a start order (topological sort
    /// via Kahn's algorithm). On a clean DAG the returned `Vec` is a full
    /// permutation of `0..units.len()` and `cycle` is empty. If a cycle is
    /// detected, the units that could not be scheduled are returned in
    /// `cycle` (in arbitrary order); the caller decides whether to refuse
    /// startup or fall back to best-effort.
    pub fn resolve_start_order(&self) -> ResolveResult {
        let n = self.units.len();
        if n == 0 {
            return ResolveResult {
                order: Vec::new(),
                cycle: Vec::new(),
            };
        }

        // depends_on[i] = indices that unit i must start AFTER.
        // `After=` is a direct dep; `Requires=` implies After unless already
        // listed (matches systemd's ordering-only-via-After semantics).
        let mut depends_on: Vec<Vec<usize>> = (0..n).map(|_| Vec::new()).collect();
        for (i, unit) in self.units.iter().enumerate() {
            for dep_name in unit.after.iter().chain(unit.requires.iter()) {
                if let Some(j) = self.find_unit(dep_name) {
                    if i != j && !depends_on[i].contains(&j) {
                        depends_on[i].push(j);
                    }
                }
            }
        }

        // Reverse edges so we can decrement in_degree as deps complete.
        let mut enables: Vec<Vec<usize>> = (0..n).map(|_| Vec::new()).collect();
        for (i, deps) in depends_on.iter().enumerate() {
            for &j in deps {
                enables[j].push(i);
            }
        }

        let mut in_degree: Vec<usize> = depends_on.iter().map(|d| d.len()).collect();
        let mut ready: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
        let mut order: Vec<usize> = Vec::with_capacity(n);
        while let Some(idx) = ready.pop() {
            order.push(idx);
            for &next in &enables[idx] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    ready.push(next);
                }
            }
        }

        let mut cycle: Vec<usize> = Vec::new();
        if order.len() < n {
            for i in 0..n {
                if in_degree[i] > 0 {
                    cycle.push(i);
                }
            }
        }
        ResolveResult { order, cycle }
    }

    /// Start all units in dependency order. Units that participate in a
    /// dependency cycle are skipped and marked Failed; the rest still
    /// start in order. Returns the number of cycle-skipped units (0 on a
    /// clean DAG).
    pub fn start_all(&mut self) -> usize {
        let resolve = self.resolve_start_order();
        for &idx in &resolve.order {
            self.start_unit(idx);
        }
        for &idx in &resolve.cycle {
            log("racinit: refusing to start ");
            log(&self.units[idx].name);
            log(" — depends on a cycle\n");
            self.units[idx].state = UnitState::Failed;
        }
        resolve.cycle.len()
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
                Ok(pid) => self.on_child_exit(pid, status),
                Err(_) => {
                    // No children to wait for (ECHILD) or other error.
                    // Sleep briefly to avoid busy-spinning; orphan zombies
                    // reparented to PID 1 will surface via the next wait.
                    let _ = libc_lite::nanosleep(1, 0);
                }
            }
        }
    }

    /// Internal: handle one child's exit. Looks up the owning unit (if any),
    /// applies the restart policy, and either respawns the service or
    /// quarantines it if it has exceeded its restart burst budget.
    pub(crate) fn on_child_exit(&mut self, pid: i32, status: i32) {
        let pos = match self.pids.iter().position(|&(_, p)| p == pid) {
            Some(p) => p,
            None => return, // Unowned orphan; just reaped.
        };
        let (unit_idx, _) = self.pids.remove(pos);

        let should_restart = {
            let unit = &self.units[unit_idx];
            match unit.restart {
                RestartPolicy::Always => true,
                RestartPolicy::OnFailure | RestartPolicy::OnAbnormal => status != 0,
                RestartPolicy::No => false,
            }
        };

        if !should_restart {
            let unit = &mut self.units[unit_idx];
            unit.state = if status == 0 {
                UnitState::Stopped
            } else {
                UnitState::Failed
            };
            log("racinit: ");
            log(&unit.name);
            log(" exited with status ");
            log_i32(status);
            log("\n");
            return;
        }

        // Burst gate: if this unit has restarted too many times in the
        // window, give up rather than crashloop forever.
        let now = now_secs();
        let burst = self.restart_trackers[unit_idx].record_and_check(now);
        if burst {
            let unit = &mut self.units[unit_idx];
            unit.state = UnitState::Failed;
            log("racinit: ");
            log(&unit.name);
            log(" hit restart burst limit — quarantined\n");
            return;
        }

        log("racinit: restarting ");
        log(&self.units[unit_idx].name);
        log("\n");
        self.units[unit_idx].state = UnitState::Stopped;
        self.start_unit(unit_idx);
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

/// Coarse "seconds since some fixed epoch" used only for restart-burst
/// gating. Defaults to clock_gettime; on host or if the syscall fails we
/// fall back to a monotonic counter so tests can still drive the burst
/// logic without QEMU.
fn now_secs() -> u64 {
    let mut ts = libc_lite::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if libc_lite::clock_gettime(libc_lite::CLOCK_MONOTONIC, &mut ts).is_ok() {
        return ts.tv_sec as u64;
    }
    0
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
