# RacOS — Service Model Specification (RacInit)

> Version: 0.1.0 | Status: Draft | Component: RacInit

## 1. Overview

RacInit is the init process (PID 1) and service manager for RacOS. Functionally inspired by systemd's organizational model, but entirely original in implementation and format.

## 2. Unit Types

| Type | Extension | Purpose |
|------|-----------|---------|
| service | `.service` | Long-running daemon or one-shot task |
| target | `.target` | Grouping milestone — represents a system state |
| timer | `.timer` | Scheduled/periodic execution of a service |
| mount | `.mount` | Filesystem mount point |
| device | `.device` | Device availability gate |

## 3. Unit File Format

INI-style with defined sections. Stored in `/etc/racinit/`.

### 3.1 Example Service Unit

```ini
[Unit]
Description=Example Daemon
Documentation=/docs/example.md

[Dependencies]
Requires=network.target
After=network.target
Wants=logging.service

[Service]
Type=simple
ExecStart=/usr/sbin/exampled --config /etc/exampled.conf
ExecReload=/usr/sbin/exampled --reload
ExecStop=
Restart=on-failure
RestartDelaySec=5
TimeoutStartSec=30
TimeoutStopSec=10
User=service
Group=services
WorkingDirectory=/var/lib/exampled
Environment=LOG_LEVEL=info
CapabilityBoundingSet=CAP_NET_BIND

[Install]
WantedBy=base.target
```

### 3.2 Example Target Unit

```ini
[Unit]
Description=Base System Target

[Dependencies]
Requires=filesystem.mount logging.service
After=filesystem.mount
```

### 3.3 Example Timer Unit

```ini
[Unit]
Description=Periodic log rotation

[Timer]
OnBootSec=300
OnUnitActiveSec=3600
Unit=logrotate.service

[Install]
WantedBy=timers.target
```

## 4. Service Types

| Type | Behavior |
|------|----------|
| simple | Main process is the service. Started when ExecStart runs. |
| oneshot | Runs and exits. Considered active after successful exit. |
| forking | Forks daemon; parent exits. PID file expected. |

## 5. Dependency Model

### 5.1 Dependency Types

| Keyword | Behavior |
|---------|----------|
| Requires | Hard dependency — failure in required unit stops this unit |
| Wants | Soft dependency — failure does not propagate |
| After | Ordering — start this unit after listed units |
| Before | Ordering — start this unit before listed units |
| Conflicts | Cannot run simultaneously |

### 5.2 Dependency Resolution

1. Parse all unit files
2. Build directed acyclic graph (DAG)
3. Detect cycles (report error, refuse to start circular deps)
4. Topological sort for start order
5. Start units level by level

## 6. Service Lifecycle

### 6.1 States

| State | Description |
|-------|-------------|
| loaded | Unit file parsed, not yet started |
| starting | ExecStart initiated, waiting for readiness |
| active | Running or completed successfully (oneshot) |
| reloading | ExecReload in progress |
| stopping | ExecStop or SIGTERM sent |
| stopped | Explicitly stopped |
| failed | Process exited with error or timed out |

### 6.2 Restart Policies

| Policy | Behavior |
|--------|----------|
| no | Never restart |
| on-failure | Restart only on non-zero exit |
| on-abnormal | Restart on signal/timeout/crash (not clean exit) |
| always | Always restart |

Restart respects `RestartDelaySec` and a burst limit (max 5 restarts in 60s, then mark failed).

### 6.3 Timeout Handling

- `TimeoutStartSec`: If service doesn't become active within this time, kill and fail
- `TimeoutStopSec`: If service doesn't exit within this time after SIGTERM, send SIGKILL

## 7. Healthcheck (Post-MVP)

```ini
[Healthcheck]
ExecCheck=/usr/sbin/exampled --health
IntervalSec=30
TimeoutSec=5
Retries=3
```

## 8. Log Routing

- Services inherit stdout/stderr from RacInit by default
- RacInit captures output and writes to journal: `/var/log/racinit/`
- Each service's output is prefixed with unit name and timestamp:
  ```
  [2026-04-04T12:00:00.000Z] [exampled.service] Started successfully
  ```

## 9. Admin CLI: servicectl

### 9.1 Commands

| Command | Description |
|---------|-------------|
| `servicectl start <unit>` | Start a unit and its dependencies |
| `servicectl stop <unit>` | Stop a unit (SIGTERM → timeout → SIGKILL) |
| `servicectl restart <unit>` | Stop then start |
| `servicectl status [unit]` | Show state, PID, uptime, last log lines |
| `servicectl enable <unit>` | Create symlink in target's wants directory |
| `servicectl disable <unit>` | Remove symlink |
| `servicectl list [--all]` | List units filtered by state |
| `servicectl log <unit> [--lines N]` | Show journal entries |

### 9.2 Status Output Format

```
● exampled.service - Example Daemon
     State: active (running)
     PID: 142
     Started: 2026-04-04 12:00:00 UTC
     Uptime: 2h 30m
     Restarts: 0
     Memory: 4.2 MiB

  Last 5 log lines:
  [12:00:00] Started successfully
  [12:00:01] Listening on port 8080
```

## 10. Boot Sequence

```
RacInit (PID 1) starts
  ↓
Parse /etc/racinit/*.{service,target,timer,mount}
  ↓
Build dependency graph
  ↓
Start root target: default.target
  ↓
  ├── filesystem.mount
  ├── logging.service
  ├── network.target (later)
  ├── ...
  ↓
base.target reached → system booted
```

## 11. Shutdown Sequence

1. `servicectl stop default.target` (or shutdown command)
2. Stop services in reverse dependency order
3. SIGTERM to all remaining processes, wait TimeoutStopSec
4. SIGKILL to all remaining
5. Unmount filesystems
6. Sync
7. Kernel: `sys_reboot` or halt

## 12. Error Handling

| Scenario | Behavior |
|----------|----------|
| Unit file parse error | Log error, skip unit, continue boot |
| Circular dependency | Log error, refuse to start cycle, continue with rest |
| Start timeout | Kill process, mark failed, apply restart policy |
| ExecStart not found | Mark failed immediately |
| Permission denied | Mark failed, log security error |

## 13. File Locations

| Path | Purpose |
|------|---------|
| `/etc/racinit/` | Unit files |
| `/var/log/racinit/` | Service journals |
| `/run/racinit/` | Runtime state (PIDs, sockets) |
