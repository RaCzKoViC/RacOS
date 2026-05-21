// RaCore — Minimal DNS client (RFC 1035)
//
// Builds a single-question A-record query and extracts the first A answer
// from the response. Supports message compression in NAME fields.

pub const PORT: u16 = 53;
pub const HEADER_LEN: usize = 12;

pub const TYPE_A: u16 = 1;
pub const CLASS_IN: u16 = 1;

#[derive(Debug)]
pub enum DnsError {
    NameTooLong,
    LabelTooLong,
    Truncated,
    BadFormat,
    Loop,
    NotFound,
}

/// Encode a hostname into DNS wire format (label-prefixed). Returns bytes written.
fn encode_name(out: &mut [u8], name: &str) -> Result<usize, DnsError> {
    let mut pos = 0;
    for label in name.split('.') {
        if label.is_empty() { continue; }
        if label.len() > 63 { return Err(DnsError::LabelTooLong); }
        if pos + 1 + label.len() > out.len() { return Err(DnsError::NameTooLong); }
        out[pos] = label.len() as u8;
        out[pos + 1..pos + 1 + label.len()].copy_from_slice(label.as_bytes());
        pos += 1 + label.len();
    }
    if pos >= out.len() { return Err(DnsError::NameTooLong); }
    out[pos] = 0; // root label
    Ok(pos + 1)
}

/// Build a DNS query for `name` (A record). Returns total bytes written.
/// `out` must be at least 12 + len(name)+2 + 4 bytes.
pub fn build_query_a(out: &mut [u8], id: u16, name: &str) -> Result<usize, DnsError> {
    if out.len() < HEADER_LEN { return Err(DnsError::NameTooLong); }

    // Header: id, flags=0x0100 (standard query, recursion desired), qd=1
    out[0..2].copy_from_slice(&id.to_be_bytes());
    out[2..4].copy_from_slice(&0x0100u16.to_be_bytes());
    out[4..6].copy_from_slice(&1u16.to_be_bytes());   // QDCOUNT
    out[6..8].copy_from_slice(&0u16.to_be_bytes());
    out[8..10].copy_from_slice(&0u16.to_be_bytes());
    out[10..12].copy_from_slice(&0u16.to_be_bytes());

    let name_len = encode_name(&mut out[HEADER_LEN..], name)?;
    let qtype_off = HEADER_LEN + name_len;
    if qtype_off + 4 > out.len() { return Err(DnsError::NameTooLong); }
    out[qtype_off..qtype_off + 2].copy_from_slice(&TYPE_A.to_be_bytes());
    out[qtype_off + 2..qtype_off + 4].copy_from_slice(&CLASS_IN.to_be_bytes());
    Ok(qtype_off + 4)
}

/// Skip a DNS name starting at offset `off` in `msg`, honouring compression
/// pointers. Returns the offset just past the name (in the *outer* message).
fn skip_name(msg: &[u8], mut off: usize) -> Result<usize, DnsError> {
    let mut hops = 0;
    loop {
        if off >= msg.len() { return Err(DnsError::Truncated); }
        let len = msg[off];
        if len == 0 { return Ok(off + 1); }
        if (len & 0xC0) == 0xC0 {
            // Pointer — name continues at the target, but for *skip* we stop.
            if off + 1 >= msg.len() { return Err(DnsError::Truncated); }
            return Ok(off + 2);
        }
        if (len & 0xC0) != 0 { return Err(DnsError::BadFormat); }
        off += 1 + len as usize;
        hops += 1;
        if hops > 128 { return Err(DnsError::Loop); }
    }
}

/// Parse a DNS response and return the first A record (4-byte IPv4 address)
/// matching the original query.
pub fn parse_first_a(msg: &[u8]) -> Result<[u8; 4], DnsError> {
    if msg.len() < HEADER_LEN { return Err(DnsError::Truncated); }
    let flags = u16::from_be_bytes([msg[2], msg[3]]);
    let qd = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let an = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    if flags & 0x8000 == 0 { return Err(DnsError::BadFormat); } // not a response
    let rcode = (flags & 0x000F) as u8;
    if rcode != 0 || an == 0 { return Err(DnsError::NotFound); }

    // Skip question section.
    let mut off = HEADER_LEN;
    for _ in 0..qd {
        off = skip_name(msg, off)?;
        if off + 4 > msg.len() { return Err(DnsError::Truncated); }
        off += 4; // qtype + qclass
    }

    // Walk answer section.
    for _ in 0..an {
        off = skip_name(msg, off)?;
        if off + 10 > msg.len() { return Err(DnsError::Truncated); }
        let rtype = u16::from_be_bytes([msg[off], msg[off + 1]]);
        let _rclass = u16::from_be_bytes([msg[off + 2], msg[off + 3]]);
        let _ttl = u32::from_be_bytes([msg[off + 4], msg[off + 5], msg[off + 6], msg[off + 7]]);
        let rdlength = u16::from_be_bytes([msg[off + 8], msg[off + 9]]) as usize;
        let rdata = off + 10;
        if rdata + rdlength > msg.len() { return Err(DnsError::Truncated); }
        if rtype == TYPE_A && rdlength == 4 {
            return Ok([msg[rdata], msg[rdata + 1], msg[rdata + 2], msg[rdata + 3]]);
        }
        off = rdata + rdlength;
    }
    Err(DnsError::NotFound)
}
