//! rpkg — Low-level package installer for RacOS
//!
//! Phase E MVP in this crate provides:
//! - `.rpk` header parser and validator
//! - section extraction helpers
//! - lightweight manifest metadata extraction

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const RPK_MAGIC: [u8; 4] = [b'R', b'P', b'K', 0x01];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageHeader {
    pub format_version: u32,
    pub manifest_offset: u64,
    pub manifest_size: u64,
    pub signature_offset: u64,
    pub signature_size: u64,
    pub data_offset: u64,
    pub data_size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Manifest,
    Signature,
    Data,
}

#[cfg(not(feature = "std"))]
use alloc::string::String;
#[cfg(feature = "std")]
use std::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSummary {
    pub name: Option<String>,
    pub version: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub package_name: Option<String>,
    pub package_version: Option<String>,
    pub source_file: String,
    pub db_root: String,
    pub info_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooShort,
    InvalidMagic,
    InvalidBounds,
    InvalidUtf8,
    MissingSection,
}

fn read_u32_le(buf: &[u8], off: usize) -> Result<u32, Error> {
    let b = buf.get(off..off + 4).ok_or(Error::TooShort)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u64_le(buf: &[u8], off: usize) -> Result<u64, Error> {
    let b = buf.get(off..off + 8).ok_or(Error::TooShort)?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

pub fn parse_header(rpk: &[u8]) -> Result<PackageHeader, Error> {
    if rpk.len() < 56 {
        return Err(Error::TooShort);
    }
    if rpk[0..4] != RPK_MAGIC {
        return Err(Error::InvalidMagic);
    }

    let header = PackageHeader {
        format_version: read_u32_le(rpk, 4)?,
        manifest_offset: read_u64_le(rpk, 8)?,
        manifest_size: read_u64_le(rpk, 16)?,
        signature_offset: read_u64_le(rpk, 24)?,
        signature_size: read_u64_le(rpk, 32)?,
        data_offset: read_u64_le(rpk, 40)?,
        data_size: read_u64_le(rpk, 48)?,
    };

    validate_header(&header, rpk.len() as u64)?;
    Ok(header)
}

pub fn validate_header(header: &PackageHeader, file_len: u64) -> Result<(), Error> {
    fn checked_end(off: u64, size: u64) -> Option<u64> {
        off.checked_add(size)
    }

    let mend =
        checked_end(header.manifest_offset, header.manifest_size).ok_or(Error::InvalidBounds)?;
    let send =
        checked_end(header.signature_offset, header.signature_size).ok_or(Error::InvalidBounds)?;
    let dend = checked_end(header.data_offset, header.data_size).ok_or(Error::InvalidBounds)?;

    if mend > file_len || send > file_len || dend > file_len {
        return Err(Error::InvalidBounds);
    }
    if header.manifest_size == 0 || header.data_size == 0 {
        return Err(Error::InvalidBounds);
    }
    Ok(())
}

pub fn section<'a>(
    rpk: &'a [u8],
    header: &PackageHeader,
    kind: SectionKind,
) -> Result<&'a [u8], Error> {
    let (off, size) = match kind {
        SectionKind::Manifest => (header.manifest_offset, header.manifest_size),
        SectionKind::Signature => (header.signature_offset, header.signature_size),
        SectionKind::Data => (header.data_offset, header.data_size),
    };
    let start = off as usize;
    let end = off.checked_add(size).ok_or(Error::InvalidBounds)? as usize;
    rpk.get(start..end).ok_or(Error::MissingSection)
}

pub fn manifest_summary(manifest_bytes: &[u8]) -> Result<ManifestSummary, Error> {
    let s = core::str::from_utf8(manifest_bytes).map_err(|_| Error::InvalidUtf8)?;
    let mut in_package = false;
    let mut name = None;
    let mut version = None;
    let mut arch = None;

    for line in s.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if l.starts_with('[') && l.ends_with(']') {
            in_package = l == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            let key = k.trim();
            let val = v.trim().trim_matches('"').to_string();
            match key {
                "name" => name = Some(String::from(val)),
                "version" => version = Some(String::from(val)),
                "arch" => arch = Some(String::from(val)),
                _ => {}
            }
        }
    }

    Ok(ManifestSummary {
        name,
        version,
        arch,
    })
}

pub fn build_install_plan(
    rpk_bytes: &[u8],
    source_file: &str,
    db_root: &str,
) -> Result<InstallPlan, Error> {
    let header = parse_header(rpk_bytes)?;
    let manifest = section(rpk_bytes, &header, SectionKind::Manifest)?;
    let summary = manifest_summary(manifest)?;

    let info_leaf = summary
        .name
        .clone()
        .unwrap_or_else(|| String::from("unknown"));
    let mut info_dir = String::from(db_root.trim_end_matches('/'));
    info_dir.push_str("/info/");
    info_dir.push_str(&info_leaf);

    Ok(InstallPlan {
        package_name: summary.name,
        package_version: summary.version,
        source_file: String::from(source_file),
        db_root: String::from(db_root),
        info_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fake_rpk() -> Vec<u8> {
        let manifest = b"[package]\nname = \"demo\"\nversion = \"1.0.0\"\narch = \"x86_64\"\n";
        let signature = b"sig";
        let data = b"DATA";

        let mo = 56u64;
        let ms = manifest.len() as u64;
        let so = mo + ms;
        let ss = signature.len() as u64;
        let doff = so + ss;
        let ds = data.len() as u64;

        let mut out = vec![0u8; 56];
        out[0..4].copy_from_slice(&RPK_MAGIC);
        out[4..8].copy_from_slice(&1u32.to_le_bytes());
        out[8..16].copy_from_slice(&mo.to_le_bytes());
        out[16..24].copy_from_slice(&ms.to_le_bytes());
        out[24..32].copy_from_slice(&so.to_le_bytes());
        out[32..40].copy_from_slice(&ss.to_le_bytes());
        out[40..48].copy_from_slice(&doff.to_le_bytes());
        out[48..56].copy_from_slice(&ds.to_le_bytes());
        out.extend_from_slice(manifest);
        out.extend_from_slice(signature);
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn parse_header_ok() {
        let rpk = make_fake_rpk();
        let h = parse_header(&rpk).unwrap();
        assert_eq!(h.format_version, 1);
        assert_eq!(h.manifest_offset, 56);
    }

    #[test]
    fn summary_ok() {
        let rpk = make_fake_rpk();
        let h = parse_header(&rpk).unwrap();
        let m = section(&rpk, &h, SectionKind::Manifest).unwrap();
        let s = manifest_summary(m).unwrap();
        assert_eq!(s.name.as_deref(), Some("demo"));
    }
}
