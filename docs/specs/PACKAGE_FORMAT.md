# RacOS — Package Format Specification (rpkg + rapt)

> Version: 0.1.0 | Status: Draft | Components: rpkg, rapt

## 1. Two-Layer Architecture

```
┌─────────────────────────────────────┐
│  rapt (high-level)                  │
│  - Repository management            │
│  - Dependency resolution             │
│  - Channel policy                    │
│  - Signature verification            │
│  - System upgrades                   │
├─────────────────────────────────────┤
│  rpkg (low-level)                   │
│  - Package install/remove            │
│  - File database                     │
│  - Hook execution                    │
│  - Integrity verification            │
│  - Rollback metadata                 │
└─────────────────────────────────────┘
```

rpkg never resolves dependencies or contacts repositories. rapt never touches files directly.

## 2. Package Format (.rpk)

### 2.1 Archive Structure

An `.rpk` file is a custom archive with magic header:

```
Bytes 0-3:    Magic "RPK\x01"
Bytes 4-7:    Format version (u32 LE)
Bytes 8-15:   Manifest offset (u64 LE)
Bytes 16-23:  Manifest size (u64 LE)
Bytes 24-31:  Signature offset (u64 LE)
Bytes 32-39:  Signature size (u64 LE)
Bytes 40-47:  Data offset (u64 LE)
Bytes 48-55:  Data size (u64 LE)
Bytes 56-...: Sections (manifest, checksums, hooks, data)
```

### 2.2 Sections

| Section | Format | Purpose |
|---------|--------|---------|
| MANIFEST | TOML | Package metadata |
| CHECKSUMS | text (sha256 per file) | Integrity verification |
| SIGNATURE | binary (ed25519) | Authenticity |
| HOOKS | tar archive | pre-install, post-install, pre-remove, post-remove scripts |
| DATA | tar archive | Filesystem payload (preserving paths and permissions) |

### 2.3 Manifest Format (TOML)

```toml
[package]
name = "example-tool"
version = "1.2.3"
arch = "x86_64"
description = "An example command-line tool"
maintainer = "RacOS Team"
homepage = ""
license = "MIT"

[dependencies]
libc-lite = ">= 0.1.0"
libfoo = ">= 2.0.0, < 3.0.0"

[conflicts]
old-example-tool = "*"

[provides]
example = "1.2.3"

[services]
# Declares associated service units
install = ["example-tool.service"]

[files]
config = ["/etc/example-tool/config.toml"]
```

### 2.4 Checksum File

```
sha256:a1b2c3...  /usr/bin/example-tool
sha256:d4e5f6...  /usr/lib/libexample.so
sha256:789abc...  /etc/example-tool/config.toml
```

## 3. rpkg — Low-Level Package Tool

### 3.1 CLI

| Command | Description |
|---------|-------------|
| `rpkg install <file.rpk>` | Install package from local file |
| `rpkg remove <package>` | Remove package (keep config) |
| `rpkg purge <package>` | Remove package + config |
| `rpkg list [--installed]` | List packages |
| `rpkg info <package>` | Show package info |
| `rpkg verify <package>` | Verify installed file checksums |
| `rpkg files <package>` | List files owned by package |
| `rpkg owner <file>` | Which package owns this file |

### 3.2 Package Database

Location: `/var/lib/rpkg/`

```
/var/lib/rpkg/
  status              — installed packages index (name, version, state)
  info/
    <package>/
      manifest.toml   — copy of package manifest
      checksums       — file checksums
      files           — list of installed files
      conffiles       — configuration files (protected on upgrade)
```

### 3.3 Installation Process

1. Validate .rpk magic and format version
2. Extract and parse MANIFEST
3. Verify SIGNATURE against trusted keys
4. Verify CHECKSUMS
5. Check for file conflicts with installed packages
6. Run pre-install hook (if present)
7. Extract DATA to filesystem
8. Update package database
9. Run post-install hook (if present)
10. If service declared: notify RacInit to reload units

### 3.4 Removal Process

1. Run pre-remove hook
2. Remove files listed in database (skip conffiles for `remove`, include for `purge`)
3. Remove empty directories
4. Update package database
5. Run post-remove hook
6. If service declared: stop and remove unit

### 3.5 Rollback Metadata

Before overwriting files during upgrade, rpkg saves:
- Previous file checksums
- Previous manifest
- Backup of modified conffiles

On failure: restore previous files from backup, revert database entry.

## 4. rapt — High-Level Package Tool

### 4.1 CLI

| Command | Description |
|---------|-------------|
| `rapt update` | Refresh repository indices |
| `rapt install <package> [...]` | Install package(s) with dependencies |
| `rapt remove <package> [...]` | Remove package(s) |
| `rapt upgrade` | Upgrade all packages to latest versions |
| `rapt search <query>` | Search available packages |
| `rapt show <package>` | Show package details from repo |
| `rapt list [--installed\|--upgradable]` | List packages |
| `rapt clean` | Remove cached .rpk files |

### 4.2 Repository Configuration

Location: `/etc/rapt/sources.toml`

```toml
[[repository]]
name = "racos-stable"
url = "https://repo.racos.example/stable"
channel = "stable"
priority = 100
key = "/etc/rapt/keys/racos-stable.pub"

[[repository]]
name = "racos-testing"
url = "https://repo.racos.example/testing"
channel = "testing"
priority = 200
enabled = false
key = "/etc/rapt/keys/racos-testing.pub"
```

### 4.3 Repository Index Format

```
/repo/
  index.toml           — package list with versions, checksums, sizes
  index.toml.sig       — signature of index
  packages/
    example-tool-1.2.3-x86_64.rpk
    ...
```

Index entry:
```toml
[[package]]
name = "example-tool"
version = "1.2.3"
arch = "x86_64"
size = 45678
sha256 = "abc123..."
depends = ["libc-lite >= 0.1.0"]
filename = "packages/example-tool-1.2.3-x86_64.rpk"
```

### 4.4 Dependency Resolution Algorithm

1. Build a graph of requested packages + all transitive dependencies
2. For each dependency, find the best matching version from enabled repositories
3. Detect conflicts
4. Topological sort for installation order
5. Present plan to user for confirmation
6. Download .rpk files to `/var/cache/rapt/`
7. Call rpkg for each package in order

### 4.5 Channels

| Channel | Purpose | Update frequency |
|---------|---------|-----------------|
| stable | Production releases | On release |
| testing | Pre-release validation | Weekly |
| dev | Development builds | On commit |

### 4.6 Upgrade Safety

1. `rapt upgrade` downloads all new .rpk files first
2. Verifies all signatures and checksums
3. Plans the upgrade order (dependencies first)
4. Applies upgrades via rpkg
5. If any step fails: rpkg rollback for that package, report partial failure

## 5. Signing

- Algorithm: Ed25519
- Keys stored in `/etc/rapt/keys/`
- Package signatures cover: MANIFEST + CHECKSUMS + DATA hash
- Repository index signatures cover: entire index.toml
- Unsigned packages are rejected by default (`allow_unsigned = false`)

## 6. Exit Criteria

- [ ] rpkg can install, remove, purge, verify packages
- [ ] rapt can fetch index, resolve dependencies, install with deps
- [ ] rapt upgrade works end-to-end
- [ ] Rollback works when install fails mid-way
- [ ] Signature verification rejects tampered packages
