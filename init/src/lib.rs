#![no_std]
/// RacInit — Init/Service Manager for RacOS (ADR-011, ADR-012)
///
/// This crate provides the service management logic used by the
/// kernel-side init task (PID 1) during early development.
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub mod engine;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Service unit state (SERVICE_MODEL.md §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    Loaded,
    Starting,
    Active,
    Reloading,
    Stopping,
    Stopped,
    Failed,
}

/// Service restart policy (SERVICE_MODEL.md §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    No,
    OnFailure,
    OnAbnormal,
    Always,
}

/// Service type (SERVICE_MODEL.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceType {
    Simple,
    Oneshot,
    Forking,
}

/// Unit type (SERVICE_MODEL.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitType {
    Service,
    Target,
    Timer,
    Mount,
    Device,
}

/// A parsed unit file.
#[derive(Debug, Clone)]
pub struct Unit {
    pub name: String,
    pub unit_type: UnitType,
    pub description: String,
    pub state: UnitState,

    // [Dependencies]
    pub requires: Vec<String>,
    pub wants: Vec<String>,
    pub after: Vec<String>,
    pub before: Vec<String>,

    // [Service] (only for Service units)
    pub service_type: ServiceType,
    pub exec_start: String,
    pub restart: RestartPolicy,
    pub restart_delay_sec: u32,
    pub timeout_start_sec: u32,
    pub timeout_stop_sec: u32,

    // [Install]
    pub wanted_by: Vec<String>,
}

impl Unit {
    pub fn new(name: &str, unit_type: UnitType) -> Self {
        Unit {
            name: String::from(name),
            unit_type,
            description: String::new(),
            state: UnitState::Loaded,
            requires: Vec::new(),
            wants: Vec::new(),
            after: Vec::new(),
            before: Vec::new(),
            service_type: ServiceType::Simple,
            exec_start: String::new(),
            restart: RestartPolicy::No,
            restart_delay_sec: 5,
            timeout_start_sec: 30,
            timeout_stop_sec: 10,
            wanted_by: Vec::new(),
        }
    }
}

/// Simple INI-style unit file parser (ADR-012).
pub fn parse_unit(name: &str, content: &str) -> Result<Unit, &'static str> {
    let unit_type = if name.ends_with(".service") {
        UnitType::Service
    } else if name.ends_with(".target") {
        UnitType::Target
    } else if name.ends_with(".timer") {
        UnitType::Timer
    } else if name.ends_with(".mount") {
        UnitType::Mount
    } else if name.ends_with(".device") {
        UnitType::Device
    } else {
        return Err("Unknown unit type");
    };

    let mut unit = Unit::new(name, unit_type);
    let mut current_section = "";

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header
        if line.starts_with('[') && line.ends_with(']') {
            current_section = &line[1..line.len() - 1];
            continue;
        }

        // Key=Value
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            match (current_section, key) {
                ("Unit", "Description") => unit.description = String::from(value),
                ("Dependencies", "Requires") => {
                    for dep in value.split_whitespace() {
                        unit.requires.push(String::from(dep));
                    }
                }
                ("Dependencies", "Wants") => {
                    for dep in value.split_whitespace() {
                        unit.wants.push(String::from(dep));
                    }
                }
                ("Dependencies", "After") => {
                    for dep in value.split_whitespace() {
                        unit.after.push(String::from(dep));
                    }
                }
                ("Dependencies", "Before") => {
                    for dep in value.split_whitespace() {
                        unit.before.push(String::from(dep));
                    }
                }
                ("Service", "Type") => {
                    unit.service_type = match value {
                        "simple" => ServiceType::Simple,
                        "oneshot" => ServiceType::Oneshot,
                        "forking" => ServiceType::Forking,
                        _ => return Err("Unknown service type"),
                    };
                }
                ("Service", "ExecStart") => unit.exec_start = String::from(value),
                ("Service", "Restart") => {
                    unit.restart = match value {
                        "no" => RestartPolicy::No,
                        "on-failure" => RestartPolicy::OnFailure,
                        "on-abnormal" => RestartPolicy::OnAbnormal,
                        "always" => RestartPolicy::Always,
                        _ => return Err("Unknown restart policy"),
                    };
                }
                ("Service", "RestartDelaySec") => {
                    unit.restart_delay_sec = value.parse().unwrap_or(5);
                }
                ("Service", "TimeoutStartSec") => {
                    unit.timeout_start_sec = value.parse().unwrap_or(30);
                }
                ("Service", "TimeoutStopSec") => {
                    unit.timeout_stop_sec = value.parse().unwrap_or(10);
                }
                ("Install", "WantedBy") => {
                    for target in value.split_whitespace() {
                        unit.wanted_by.push(String::from(target));
                    }
                }
                _ => {} // Unknown keys: silently ignore (forward compatibility)
            }
        }
    }

    Ok(unit)
}

/// Boot targets used during system startup.
pub const DEFAULT_TARGET: &str = "base.target";

/// The base.target unit (always created).
pub fn base_target() -> Unit {
    let mut u = Unit::new("base.target", UnitType::Target);
    u.description = String::from("Base System Target");
    u
}
