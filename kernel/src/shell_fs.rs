// RaCore — Shell filesystem API with pluggable backends.
//
// The emergency shell uses this module instead of reaching directly into
// storage details. Current backends:
// - memfs (read/write)
// - optional FAT12/16 read-only mount exposed at /disk

extern crate alloc;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::drivers::block::{self, BlockDevice, SECTOR_SIZE};
use crate::sync::SpinLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeType {
    File,
    Directory,
}

#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

// =====================
// MEMFS BACKEND
// =====================

struct MemNode {
    name: String,
    node_type: NodeType,
    children: Vec<usize>,
    parent: Option<usize>,
    content: Vec<u8>,
}

impl MemNode {
    fn dir(name: &str, parent: Option<usize>) -> Self {
        MemNode {
            name: String::from(name),
            node_type: NodeType::Directory,
            children: Vec::new(),
            parent,
            content: Vec::new(),
        }
    }

    fn file(name: &str, parent: Option<usize>, content: &[u8]) -> Self {
        MemNode {
            name: String::from(name),
            node_type: NodeType::File,
            children: Vec::new(),
            parent,
            content: content.to_vec(),
        }
    }
}

struct MemFs {
    nodes: Vec<MemNode>,
    debug: bool,
}

impl MemFs {
    fn new(debug: bool) -> Self {
        let mut fs = MemFs {
            nodes: Vec::new(),
            debug,
        };

        // /
        fs.nodes.push(MemNode::dir("/", None));

        // /bin, /home/user, /etc
        let bin = fs.add_node(0, "bin", NodeType::Directory, b"").unwrap_or(0);
        let home = fs.add_node(0, "home", NodeType::Directory, b"").unwrap_or(0);
        let user = fs.add_node(home, "user", NodeType::Directory, b"").unwrap_or(home);
        let etc = fs.add_node(0, "etc", NodeType::Directory, b"").unwrap_or(0);

        let _ = fs.add_node(bin, "ls", NodeType::File, b"builtin");
        let _ = fs.add_node(bin, "pwd", NodeType::File, b"builtin");
        let _ = fs.add_node(bin, "cat", NodeType::File, b"builtin");
        let _ = fs.add_node(
            etc,
            "config.txt",
            NodeType::File,
            b"RacOS shell memfs config\nmode=debug\n",
        );
        let _ = fs.add_node(
            user,
            "readme.txt",
            NodeType::File,
            b"Welcome to RacOS memfs.\nTry: ls, pwd, cd, cat.\n",
        );

        fs
    }

    fn add_node(
        &mut self,
        parent: usize,
        name: &str,
        node_type: NodeType,
        content: &[u8],
    ) -> Result<usize, &'static str> {
        if parent >= self.nodes.len() {
            return Err("invalid parent");
        }
        if self.nodes[parent].node_type != NodeType::Directory {
            return Err("parent is not a directory");
        }
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err("invalid name");
        }
        if self.find_child(parent, name).is_some() {
            return Err("already exists");
        }

        let idx = self.nodes.len();
        let node = match node_type {
            NodeType::Directory => MemNode::dir(name, Some(parent)),
            NodeType::File => MemNode::file(name, Some(parent), content),
        };

        self.nodes.push(node);
        self.nodes[parent].children.push(idx);

        if self.debug {
            crate::serial::serial_println!(
                "[ SFS-MEM ] add {:?} '{}' under {}",
                node_type,
                name,
                parent
            );
        }

        Ok(idx)
    }

    fn find_child(&self, parent: usize, name: &str) -> Option<usize> {
        self.nodes[parent]
            .children
            .iter()
            .copied()
            .find(|idx| self.nodes[*idx].name == name)
    }

    fn resolve_absolute(&self, abs_path: &str) -> Result<usize, &'static str> {
        if abs_path == "/" {
            return Ok(0);
        }
        if !abs_path.starts_with('/') {
            return Err("path must be absolute");
        }

        let mut current = 0usize;
        for part in abs_path.split('/') {
            if part.is_empty() {
                continue;
            }
            if self.nodes[current].node_type != NodeType::Directory {
                return Err("not a directory");
            }
            current = self.find_child(current, part).ok_or("path not found")?;
        }
        Ok(current)
    }

    fn path_for_node(&self, mut idx: usize) -> String {
        if idx == 0 {
            return String::from("/");
        }

        let mut parts: Vec<&str> = Vec::new();
        while idx != 0 {
            let n = &self.nodes[idx];
            parts.push(n.name.as_str());
            idx = n.parent.unwrap_or(0);
        }

        let mut out = String::from("/");
        for (i, p) in parts.iter().rev().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(p);
        }
        out
    }

    fn list_dir(&self, idx: usize) -> Result<Vec<DirEntry>, &'static str> {
        let n = self.nodes.get(idx).ok_or("not found")?;
        if n.node_type != NodeType::Directory {
            return Err("not a directory");
        }

        let mut out = Vec::new();
        for child_idx in &n.children {
            let child = &self.nodes[*child_idx];
            out.push(DirEntry {
                name: child.name.clone(),
                is_dir: child.node_type == NodeType::Directory,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn read_file(&self, idx: usize) -> Result<Vec<u8>, &'static str> {
        let n = self.nodes.get(idx).ok_or("not found")?;
        if n.node_type != NodeType::File {
            return Err("is a directory");
        }
        Ok(n.content.clone())
    }

    fn mkdir_absolute(&mut self, abs_path: &str) -> Result<(), &'static str> {
        let (parent, leaf) = split_parent_leaf(abs_path)?;
        let parent_idx = self.resolve_absolute(parent)?;
        self.add_node(parent_idx, leaf, NodeType::Directory, b"")?;
        Ok(())
    }

    fn touch_absolute(&mut self, abs_path: &str) -> Result<(), &'static str> {
        let (parent, leaf) = split_parent_leaf(abs_path)?;
        let parent_idx = self.resolve_absolute(parent)?;
        self.add_node(parent_idx, leaf, NodeType::File, b"")?;
        Ok(())
    }
}

// =====================
// FAT12/16 BACKEND (READ-ONLY)
// =====================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FatType {
    Fat12,
    Fat16,
}

#[derive(Clone)]
struct FatEntry {
    name: String,
    is_dir: bool,
    first_cluster: u16,
    size: u32,
}

enum FatResolved {
    Root,
    Entry(FatEntry),
}

struct FatFs {
    device: Arc<dyn BlockDevice>,
    fat_type: FatType,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    first_fat_sector: u32,
    first_root_sector: u32,
    first_data_sector: u32,
    root_dir_sectors: u32,
    cluster_count: u32,
}

impl FatFs {
    fn mount(device: Arc<dyn BlockDevice>) -> Result<Self, &'static str> {
        let mut boot = [0u8; SECTOR_SIZE];
        device
            .read_sector(0, &mut boot)
            .map_err(|_| "fat: boot read failed")?;

        if boot[510] != 0x55 || boot[511] != 0xAA {
            return Err("fat: missing 0x55AA signature");
        }

        let bytes_per_sector = le16(&boot, 11);
        let sectors_per_cluster = boot[13];
        let reserved_sectors = le16(&boot, 14);
        let fat_count = boot[16];
        let root_entry_count = le16(&boot, 17);
        let total_sectors_16 = le16(&boot, 19);
        let sectors_per_fat_16 = le16(&boot, 22);
        let total_sectors_32 = le32(&boot, 32);

        if bytes_per_sector != 512 {
            return Err("fat: only 512-byte sectors supported");
        }
        if sectors_per_cluster == 0 || fat_count == 0 || sectors_per_fat_16 == 0 {
            return Err("fat: invalid BPB fields");
        }
        if root_entry_count == 0 {
            return Err("fat: likely FAT32, not FAT12/16");
        }

        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16 as u32
        } else {
            total_sectors_32
        };
        if total_sectors == 0 {
            return Err("fat: invalid total sectors");
        }

        let root_dir_sectors =
            ((root_entry_count as u32 * 32) + (bytes_per_sector as u32 - 1)) / bytes_per_sector as u32;
        let first_fat_sector = reserved_sectors as u32;
        let first_root_sector = first_fat_sector + fat_count as u32 * sectors_per_fat_16 as u32;
        let first_data_sector = first_root_sector + root_dir_sectors;
        let data_sectors = total_sectors.saturating_sub(
            reserved_sectors as u32 + fat_count as u32 * sectors_per_fat_16 as u32 + root_dir_sectors,
        );
        let cluster_count = data_sectors / sectors_per_cluster as u32;

        let fat_type = if cluster_count < 4085 {
            FatType::Fat12
        } else if cluster_count < 65525 {
            FatType::Fat16
        } else {
            return Err("fat: FAT32 is not handled by this backend");
        };

        crate::serial::serial_println!(
            "[ SFS-FAT ] mounted {:?} (clusters={}, root_sector={}, data_sector={})",
            fat_type,
            cluster_count,
            first_root_sector,
            first_data_sector
        );

        Ok(FatFs {
            device,
            fat_type,
            bytes_per_sector,
            sectors_per_cluster,
            first_fat_sector,
            first_root_sector,
            first_data_sector,
            root_dir_sectors,
            cluster_count,
        })
    }

    fn read_sector(&self, lba: u32, out: &mut [u8; SECTOR_SIZE]) -> Result<(), &'static str> {
        self.device
            .read_sector(lba as u64, out)
            .map_err(|_| "fat: read sector failed")
    }

    fn is_eoc(&self, value: u16) -> bool {
        match self.fat_type {
            FatType::Fat12 => value >= 0x0FF8,
            FatType::Fat16 => value >= 0xFFF8,
        }
    }

    fn read_fat_entry(&self, cluster: u16) -> Result<u16, &'static str> {
        match self.fat_type {
            FatType::Fat16 => {
                let fat_offset = cluster as u32 * 2;
                let sector = self.first_fat_sector + fat_offset / self.bytes_per_sector as u32;
                let off = (fat_offset % self.bytes_per_sector as u32) as usize;

                let mut buf = [0u8; SECTOR_SIZE];
                self.read_sector(sector, &mut buf)?;
                Ok(le16(&buf, off))
            }
            FatType::Fat12 => {
                let fat_offset = cluster as u32 + (cluster as u32 / 2);
                let sector = self.first_fat_sector + fat_offset / self.bytes_per_sector as u32;
                let off = (fat_offset % self.bytes_per_sector as u32) as usize;

                let mut a = [0u8; SECTOR_SIZE];
                self.read_sector(sector, &mut a)?;
                let b0 = a[off];
                let b1 = if off + 1 < SECTOR_SIZE {
                    a[off + 1]
                } else {
                    let mut b = [0u8; SECTOR_SIZE];
                    self.read_sector(sector + 1, &mut b)?;
                    b[0]
                };

                let value = if (cluster & 1) == 0 {
                    ((b1 as u16 & 0x0F) << 8) | b0 as u16
                } else {
                    ((b1 as u16) << 4) | ((b0 as u16) >> 4)
                };
                Ok(value)
            }
        }
    }

    fn cluster_first_sector(&self, cluster: u16) -> u32 {
        self.first_data_sector + (cluster as u32 - 2) * self.sectors_per_cluster as u32
    }

    fn parse_dir_entries_from_sectors(
        &self,
        first_sector: u32,
        sector_count: u32,
    ) -> Result<Vec<FatEntry>, &'static str> {
        let mut out = Vec::new();

        for s in 0..sector_count {
            let mut sec = [0u8; SECTOR_SIZE];
            self.read_sector(first_sector + s, &mut sec)?;

            for off in (0..SECTOR_SIZE).step_by(32) {
                let e = &sec[off..off + 32];
                let first = e[0];
                if first == 0x00 {
                    return Ok(out);
                }
                if first == 0xE5 {
                    continue;
                }

                let attr = e[11];
                if attr == 0x0F || (attr & 0x08) != 0 {
                    continue;
                }

                let name = parse_short_name_8_3(&e[0..11]);
                if name.is_empty() || name == "." || name == ".." {
                    continue;
                }

                let first_cluster = le16(e, 26);
                let size = le32(e, 28);
                out.push(FatEntry {
                    name,
                    is_dir: (attr & 0x10) != 0,
                    first_cluster,
                    size,
                });
            }
        }

        Ok(out)
    }

    fn read_root_entries(&self) -> Result<Vec<FatEntry>, &'static str> {
        self.parse_dir_entries_from_sectors(self.first_root_sector, self.root_dir_sectors)
    }

    fn read_dir_entries_cluster(&self, start_cluster: u16) -> Result<Vec<FatEntry>, &'static str> {
        if start_cluster < 2 {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        let mut current = start_cluster;
        let mut guard = 0u32;

        while guard < self.cluster_count + 2 {
            guard += 1;

            let first_sector = self.cluster_first_sector(current);
            let mut chunk =
                self.parse_dir_entries_from_sectors(first_sector, self.sectors_per_cluster as u32)?;
            out.append(&mut chunk);

            let next = self.read_fat_entry(current)?;
            if self.is_eoc(next) || next < 2 {
                break;
            }
            current = next;
        }

        Ok(out)
    }

    fn resolve_path(&self, abs_path: &str) -> Result<FatResolved, &'static str> {
        if abs_path == "/" {
            return Ok(FatResolved::Root);
        }
        if !abs_path.starts_with('/') {
            return Err("fat: path must be absolute");
        }

        let mut current = FatResolved::Root;
        for part in abs_path.split('/') {
            if part.is_empty() {
                continue;
            }

            let entries = match &current {
                FatResolved::Root => self.read_root_entries()?,
                FatResolved::Entry(e) => {
                    if !e.is_dir {
                        return Err("fat: not a directory");
                    }
                    self.read_dir_entries_cluster(e.first_cluster)?
                }
            };

            let mut found = None;
            for e in entries {
                if name_eq_case_insensitive(&e.name, part) {
                    found = Some(e);
                    break;
                }
            }

            current = FatResolved::Entry(found.ok_or("fat: path not found")?);
        }

        Ok(current)
    }

    fn is_dir_path(&self, abs_path: &str) -> Result<bool, &'static str> {
        match self.resolve_path(abs_path)? {
            FatResolved::Root => Ok(true),
            FatResolved::Entry(e) => Ok(e.is_dir),
        }
    }

    fn list_dir(&self, abs_path: &str) -> Result<Vec<DirEntry>, &'static str> {
        let entries = match self.resolve_path(abs_path)? {
            FatResolved::Root => self.read_root_entries()?,
            FatResolved::Entry(e) => {
                if !e.is_dir {
                    return Err("not a directory");
                }
                self.read_dir_entries_cluster(e.first_cluster)?
            }
        };

        let mut out = Vec::new();
        for e in entries {
            out.push(DirEntry {
                name: e.name,
                is_dir: e.is_dir,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn read_file(&self, abs_path: &str) -> Result<Vec<u8>, &'static str> {
        let entry = match self.resolve_path(abs_path)? {
            FatResolved::Root => return Err("is a directory"),
            FatResolved::Entry(e) => e,
        };

        if entry.is_dir {
            return Err("is a directory");
        }
        if entry.size == 0 {
            return Ok(Vec::new());
        }
        if entry.first_cluster < 2 {
            return Err("fat: invalid file cluster");
        }

        let mut out = Vec::new();
        out.reserve(entry.size as usize);

        let mut remaining = entry.size as usize;
        let mut current = entry.first_cluster;
        let mut guard = 0u32;

        while remaining > 0 && guard < self.cluster_count + 2 {
            guard += 1;

            let first_sector = self.cluster_first_sector(current);
            for s in 0..self.sectors_per_cluster as u32 {
                let mut sec = [0u8; SECTOR_SIZE];
                self.read_sector(first_sector + s, &mut sec)?;

                let take = core::cmp::min(remaining, SECTOR_SIZE);
                out.extend_from_slice(&sec[..take]);
                remaining -= take;

                if remaining == 0 {
                    break;
                }
            }

            if remaining == 0 {
                break;
            }

            let next = self.read_fat_entry(current)?;
            if self.is_eoc(next) {
                break;
            }
            if next < 2 {
                return Err("fat: broken cluster chain");
            }
            current = next;
        }

        if out.len() > entry.size as usize {
            out.truncate(entry.size as usize);
        }

        Ok(out)
    }
}

fn parse_short_name_8_3(raw: &[u8]) -> String {
    let mut base = String::new();
    for &b in &raw[0..8] {
        if b == b' ' {
            break;
        }
        base.push(byte_upper_ascii(b) as char);
    }

    let mut ext = String::new();
    for &b in &raw[8..11] {
        if b == b' ' {
            break;
        }
        ext.push(byte_upper_ascii(b) as char);
    }

    if ext.is_empty() {
        base
    } else {
        let mut name = base;
        name.push('.');
        name.push_str(&ext);
        name
    }
}

fn name_eq_case_insensitive(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (ca, cb) in a.bytes().zip(b.bytes()) {
        if byte_upper_ascii(ca) != byte_upper_ascii(cb) {
            return false;
        }
    }
    true
}

fn byte_upper_ascii(b: u8) -> u8 {
    if (b'a'..=b'z').contains(&b) {
        b - 32
    } else {
        b
    }
}

fn le16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

fn le32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn split_parent_leaf(abs_path: &str) -> Result<(&str, &str), &'static str> {
    if !abs_path.starts_with('/') || abs_path == "/" {
        return Err("invalid path");
    }

    let trimmed = abs_path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return Err("invalid path");
    }

    if let Some(pos) = trimmed.rfind('/') {
        let leaf = &trimmed[pos + 1..];
        if leaf.is_empty() {
            return Err("invalid path");
        }
        let parent = if pos == 0 { "/" } else { &trimmed[..pos] };
        Ok((parent, leaf))
    } else {
        Err("invalid path")
    }
}

// =====================
// SHELL FS API LAYER
// =====================

enum Cwd {
    Mem(usize),
    Fat(String),
}

enum Target {
    Mem(usize),
    Fat(String),
}

struct ShellFsApi {
    mem: MemFs,
    fat: Option<FatFs>,
    cwd: Cwd,
    debug: bool,
}

impl ShellFsApi {
    fn new(debug: bool) -> Self {
        let fat = match block::find("ram0") {
            Some(dev) => match FatFs::mount(dev) {
                Ok(fs) => Some(fs),
                Err(e) => {
                    if debug {
                        crate::serial::serial_println!(
                            "[ SFS-FAT ] no FAT12/16 mounted on ram0: {}",
                            e
                        );
                    }
                    None
                }
            },
            None => None,
        };

        ShellFsApi {
            mem: MemFs::new(debug),
            fat,
            cwd: Cwd::Mem(0),
            debug,
        }
    }

    fn cwd_virtual_path(&self) -> String {
        match &self.cwd {
            Cwd::Mem(idx) => self.mem.path_for_node(*idx),
            Cwd::Fat(path) => {
                if path == "/" {
                    String::from("/disk")
                } else {
                    let mut out = String::from("/disk");
                    out.push_str(path);
                    out
                }
            }
        }
    }

    fn canonicalize(&self, input: &str) -> String {
        let mut components: Vec<String> = Vec::new();

        if !input.starts_with('/') {
            let cwd = self.cwd_virtual_path();
            for p in cwd.split('/') {
                if !p.is_empty() {
                    components.push(String::from(p));
                }
            }
        }

        for p in input.split('/') {
            if p.is_empty() || p == "." {
                continue;
            }
            if p == ".." {
                if !components.is_empty() {
                    components.pop();
                }
                continue;
            }
            components.push(String::from(p));
        }

        let mut out = String::from("/");
        for (i, p) in components.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            out.push_str(p);
        }

        if self.debug {
            crate::serial::serial_println!(
                "[ SFS ] canonicalize in='{}' cwd='{}' out='{}'",
                input,
                self.cwd_virtual_path(),
                out
            );
        }

        out
    }

    fn resolve_target(&self, canonical_abs: &str) -> Result<Target, &'static str> {
        if canonical_abs == "/" {
            return Ok(Target::Mem(0));
        }

        if canonical_abs == "/disk" || canonical_abs.starts_with("/disk/") {
            self.fat.as_ref().ok_or("disk not mounted")?;
            let rel = if canonical_abs == "/disk" {
                String::from("/")
            } else {
                String::from(&canonical_abs[5..])
            };
            return Ok(Target::Fat(rel));
        }

        let idx = self.mem.resolve_absolute(canonical_abs)?;
        Ok(Target::Mem(idx))
    }

    fn pwd(&self) -> String {
        self.cwd_virtual_path()
    }

    fn ls(&self, path: Option<&str>) -> Result<Vec<DirEntry>, &'static str> {
        let canonical = match path {
            Some(p) => self.canonicalize(p),
            None => self.cwd_virtual_path(),
        };

        if canonical == "/" {
            let mut root_entries = self.mem.list_dir(0)?;
            if self.fat.is_some() {
                root_entries.push(DirEntry {
                    name: String::from("disk"),
                    is_dir: true,
                });
            }
            root_entries.sort_by(|a, b| a.name.cmp(&b.name));
            return Ok(root_entries);
        }

        match self.resolve_target(&canonical)? {
            Target::Mem(idx) => self.mem.list_dir(idx),
            Target::Fat(rel) => self.fat.as_ref().ok_or("disk not mounted")?.list_dir(&rel),
        }
    }

    fn chdir(&mut self, path: &str) -> Result<(), &'static str> {
        let canonical = self.canonicalize(path);
        match self.resolve_target(&canonical)? {
            Target::Mem(idx) => {
                if self.mem.nodes[idx].node_type != NodeType::Directory {
                    return Err("not a directory");
                }
                self.cwd = Cwd::Mem(idx);
                Ok(())
            }
            Target::Fat(rel) => {
                let fat = self.fat.as_ref().ok_or("disk not mounted")?;
                if !fat.is_dir_path(&rel)? {
                    return Err("not a directory");
                }
                self.cwd = Cwd::Fat(rel);
                Ok(())
            }
        }
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, &'static str> {
        let canonical = self.canonicalize(path);
        match self.resolve_target(&canonical)? {
            Target::Mem(idx) => self.mem.read_file(idx),
            Target::Fat(rel) => self.fat.as_ref().ok_or("disk not mounted")?.read_file(&rel),
        }
    }

    fn mkdir(&mut self, path: &str) -> Result<(), &'static str> {
        let canonical = self.canonicalize(path);
        if canonical == "/" || canonical == "/disk" || canonical.starts_with("/disk/") {
            return Err("read-only or invalid target");
        }
        self.mem.mkdir_absolute(&canonical)
    }

    fn touch(&mut self, path: &str) -> Result<(), &'static str> {
        let canonical = self.canonicalize(path);
        if canonical == "/" || canonical == "/disk" || canonical.starts_with("/disk/") {
            return Err("read-only or invalid target");
        }
        self.mem.touch_absolute(&canonical)
    }
}

static FS: SpinLock<Option<ShellFsApi>> = SpinLock::new(None);

pub fn init(debug: bool) {
    let mut guard = FS.lock();
    if guard.is_some() {
        return;
    }
    *guard = Some(ShellFsApi::new(debug));
    crate::serial::serial_println!("[ SFS ] shell filesystem API initialized");
}

pub fn pwd() -> Result<String, &'static str> {
    let guard = FS.lock();
    let fs = guard.as_ref().ok_or("filesystem not initialized")?;
    Ok(fs.pwd())
}

pub fn ls(path: Option<&str>) -> Result<Vec<DirEntry>, &'static str> {
    let guard = FS.lock();
    let fs = guard.as_ref().ok_or("filesystem not initialized")?;
    fs.ls(path)
}

pub fn chdir(path: &str) -> Result<(), &'static str> {
    let mut guard = FS.lock();
    let fs = guard.as_mut().ok_or("filesystem not initialized")?;
    fs.chdir(path)
}

pub fn read_file(path: &str) -> Result<Vec<u8>, &'static str> {
    let guard = FS.lock();
    let fs = guard.as_ref().ok_or("filesystem not initialized")?;
    fs.read_file(path)
}

pub fn mkdir(path: &str) -> Result<(), &'static str> {
    let mut guard = FS.lock();
    let fs = guard.as_mut().ok_or("filesystem not initialized")?;
    fs.mkdir(path)
}

pub fn touch(path: &str) -> Result<(), &'static str> {
    let mut guard = FS.lock();
    let fs = guard.as_mut().ok_or("filesystem not initialized")?;
    fs.touch(path)
}