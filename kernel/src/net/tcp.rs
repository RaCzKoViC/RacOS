// RaCore — TCP (RFC 793 minimum)
//
// Scope: single-host MVP suitable for client connections (active open),
// in-order receive, timer-driven retransmission, graceful close via FIN.
//
// Out of scope (post-MVP): SACK, ECN, window scaling, fast retransmit,
// out-of-order reassembly, slow start / congestion avoidance, listen/accept,
// keep-alive, RTT-based RTO smoothing.

extern crate alloc;

use alloc::vec::Vec;

use crate::drivers;
use crate::interrupts::pit;
use crate::sync::SpinLock;

use super::{arp, eth, ipv4};

pub const TCP_HDR_LEN: usize = 20;

// --- Flag bits ---
pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;

const DEFAULT_MSS: u16 = 1460;
const DEFAULT_WINDOW: u16 = 8192;
const RTO_INITIAL_TICKS: u64 = 200;  // 200 ms at 1 kHz PIT
const RTO_MAX_TICKS: u64 = 3200;
const MAX_RETRANSMITS: u32 = 5;
const MAX_CONNS: usize = 8;
const RECV_BUF_LIMIT: usize = 16 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct TcpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub data_offset: u8, // 32-bit words
    pub flags: u8,
    pub window: u16,
    pub checksum: u16,
    pub urgent: u16,
}

impl TcpHeader {
    /// Parse a TCP header. Returns the header plus the slice of options + payload.
    /// `payload_after_options` will be empty if options span the whole remainder.
    pub fn parse(data: &[u8]) -> Option<(Self, &[u8] /* options */, &[u8] /* payload */)> {
        if data.len() < TCP_HDR_LEN { return None; }
        let src_port = u16::from_be_bytes([data[0], data[1]]);
        let dst_port = u16::from_be_bytes([data[2], data[3]]);
        let seq = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let ack = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let data_offset = (data[12] >> 4) & 0x0F;
        let flags = data[13];
        let window = u16::from_be_bytes([data[14], data[15]]);
        let checksum = u16::from_be_bytes([data[16], data[17]]);
        let urgent = u16::from_be_bytes([data[18], data[19]]);

        let header_bytes = (data_offset as usize) * 4;
        if header_bytes < TCP_HDR_LEN || header_bytes > data.len() { return None; }

        let options = &data[TCP_HDR_LEN..header_bytes];
        let payload = &data[header_bytes..];
        Some((
            TcpHeader { src_port, dst_port, seq, ack, data_offset, flags, window, checksum, urgent },
            options,
            payload,
        ))
    }
}

/// Parse TCP options for MSS. Returns advertised MSS or DEFAULT_MSS if absent.
fn parse_mss(options: &[u8]) -> u16 {
    let mut i = 0;
    while i < options.len() {
        let kind = options[i];
        match kind {
            0 => return DEFAULT_MSS,           // End of options
            1 => { i += 1; }                   // NOP
            _ => {
                if i + 1 >= options.len() { break; }
                let len = options[i + 1] as usize;
                if len < 2 || i + len > options.len() { break; }
                if kind == 2 && len == 4 {
                    return u16::from_be_bytes([options[i + 2], options[i + 3]]);
                }
                i += len;
            }
        }
    }
    DEFAULT_MSS
}

/// Build a TCP segment at `out[start..]`. `payload_len` is the length of the
/// already-populated payload at `out[start + TCP_HDR_LEN + options_len..]`.
/// `mss_option` adds the 4-byte MSS option (only used on SYN).
/// Returns total segment length.
fn write_segment(
    out: &mut [u8],
    start: usize,
    src_ip: &[u8; 4],
    dst_ip: &[u8; 4],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    window: u16,
    mss_option: Option<u16>,
    payload_len: usize,
) -> usize {
    let opt_len = if mss_option.is_some() { 4 } else { 0 };
    let header_len = TCP_HDR_LEN + opt_len;
    let total = header_len + payload_len;
    debug_assert!(start + total <= out.len());
    let hdr = &mut out[start..start + header_len];

    hdr[0..2].copy_from_slice(&src_port.to_be_bytes());
    hdr[2..4].copy_from_slice(&dst_port.to_be_bytes());
    hdr[4..8].copy_from_slice(&seq.to_be_bytes());
    hdr[8..12].copy_from_slice(&ack.to_be_bytes());
    hdr[12] = ((header_len as u8) / 4) << 4;
    hdr[13] = flags;
    hdr[14..16].copy_from_slice(&window.to_be_bytes());
    hdr[16..18].copy_from_slice(&0u16.to_be_bytes()); // checksum placeholder
    hdr[18..20].copy_from_slice(&0u16.to_be_bytes()); // urgent ptr

    if let Some(mss) = mss_option {
        hdr[20] = 2;     // kind: MSS
        hdr[21] = 4;     // length
        hdr[22..24].copy_from_slice(&mss.to_be_bytes());
    }

    // Checksum over pseudo-header + TCP header + payload.
    let mut pseudo = [0u8; 12];
    pseudo[0..4].copy_from_slice(src_ip);
    pseudo[4..8].copy_from_slice(dst_ip);
    pseudo[8] = 0;
    pseudo[9] = ipv4::PROTO_TCP;
    pseudo[10..12].copy_from_slice(&(total as u16).to_be_bytes());

    let mut sum: u32 = 0;
    sum = add_ones_complement(sum, &pseudo);
    sum = add_ones_complement(sum, &out[start..start + total]);
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let csum = !(sum as u16);
    out[start + 16..start + 18].copy_from_slice(&csum.to_be_bytes());

    total
}

fn add_ones_complement(mut sum: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    sum
}

// --- Connection state ---

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Closed,
    SynSent,
    Established,
    FinWait1,    // sent FIN, waiting for ACK of FIN (or peer FIN)
    FinWait2,    // got ACK of our FIN, waiting for peer FIN
    CloseWait,   // peer sent FIN, app needs to call close()
    LastAck,     // closed after CloseWait, waiting for ACK of our FIN
    TimeWait,    // graceful close complete; lingers until reaped
}

#[derive(Clone)]
struct PendingSegment {
    seq: u32,
    /// Payload bytes (empty for pure control segments like SYN/FIN).
    data: Vec<u8>,
    /// Flags carried by this segment (SYN, FIN, PSH, ACK).
    flags: u8,
    /// `data.len()` + 1 if SYN, +1 if FIN (segments consuming sequence space).
    seq_len: u32,
    /// Last time we transmitted this segment, in PIT ticks.
    last_tx_tick: u64,
    /// Number of retransmissions performed.
    retries: u32,
}

pub struct TcpConn {
    state: State,
    local_port: u16,
    remote_ip: [u8; 4],
    remote_port: u16,
    peer_mac: [u8; 6],

    // Send sequence space.
    snd_una: u32,   // oldest unacknowledged seq
    snd_nxt: u32,   // next seq to use
    snd_wnd: u16,   // last advertised window from peer
    iss: u32,       // initial send seq

    // Receive sequence space.
    rcv_nxt: u32,   // next expected seq
    rcv_wnd: u16,   // window we advertise
    irs: u32,       // initial receive seq

    mss: u16,

    pending: Vec<PendingSegment>,
    recv_buf: Vec<u8>,
    /// Set when the peer's FIN has been seen and queued for delivery.
    peer_fin_seen: bool,
}

impl TcpConn {
    fn new(local_port: u16, remote_ip: [u8; 4], remote_port: u16, peer_mac: [u8; 6], iss: u32) -> Self {
        TcpConn {
            state: State::Closed,
            local_port,
            remote_ip,
            remote_port,
            peer_mac,
            snd_una: iss,
            snd_nxt: iss,
            snd_wnd: DEFAULT_WINDOW,
            iss,
            rcv_nxt: 0,
            rcv_wnd: DEFAULT_WINDOW,
            irs: 0,
            mss: DEFAULT_MSS,
            pending: Vec::new(),
            recv_buf: Vec::new(),
            peer_fin_seen: false,
        }
    }

    fn matches(&self, local_port: u16, remote_ip: [u8; 4], remote_port: u16) -> bool {
        self.local_port == local_port
            && self.remote_ip == remote_ip
            && self.remote_port == remote_port
    }

    /// Drain the recv buffer into `out`, returning bytes copied.
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        if self.recv_buf.is_empty() { return 0; }
        let n = out.len().min(self.recv_buf.len());
        out[..n].copy_from_slice(&self.recv_buf[..n]);
        self.recv_buf.drain(..n);
        n
    }

    pub fn state(&self) -> State { self.state }
    pub fn recv_available(&self) -> usize { self.recv_buf.len() }
    pub fn peer_fin(&self) -> bool { self.peer_fin_seen }
}

// --- Connection table ---

struct Table {
    conns: [Option<TcpConn>; MAX_CONNS],
    our_ip: [u8; 4],
    our_mac: [u8; 6],
    next_ephemeral: u16,
    isn_counter: u32,
}

impl Table {
    const fn new() -> Self {
        const NONE: Option<TcpConn> = None;
        Table {
            conns: [NONE; MAX_CONNS],
            our_ip: [0; 4],
            our_mac: [0; 6],
            next_ephemeral: 49152,
            isn_counter: 0,
        }
    }

    fn alloc(&mut self) -> Option<usize> {
        self.conns.iter().position(|c| c.is_none())
    }

    fn find_mut(&mut self, local_port: u16, remote_ip: [u8; 4], remote_port: u16) -> Option<usize> {
        self.conns.iter().position(|c| c.as_ref().map_or(false, |x| x.matches(local_port, remote_ip, remote_port)))
    }

    fn alloc_port(&mut self) -> u16 {
        for _ in 0..1024 {
            let p = self.next_ephemeral;
            self.next_ephemeral = if self.next_ephemeral >= 65000 { 49152 } else { self.next_ephemeral + 1 };
            let in_use = self.conns.iter().any(|c| c.as_ref().map_or(false, |x| x.local_port == p));
            if !in_use { return p; }
        }
        49152
    }

    fn next_isn(&mut self) -> u32 {
        // RFC 793 recommends ISN advancing ~1/4 microsecond; we use ticks + counter.
        self.isn_counter = self.isn_counter.wrapping_add(0x9E3779B9);
        (pit::ticks() as u32).wrapping_mul(64).wrapping_add(self.isn_counter)
    }
}

static TABLE: SpinLock<Table> = SpinLock::new(Table::new());

pub fn init(our_ip: [u8; 4], our_mac: [u8; 6]) {
    let mut t = TABLE.lock();
    t.our_ip = our_ip;
    t.our_mac = our_mac;
}

/// Identifier returned to callers — currently just the table slot index.
pub type ConnId = usize;

#[derive(Debug)]
pub enum TcpError {
    NoSlot,
    NoRoute,
    NotConnected,
    Closed,
    SendFailed,
}

/// Initiate an active open. Returns a ConnId on success; SYN has been sent.
pub fn connect(remote_ip: [u8; 4], remote_port: u16, peer_mac: [u8; 6]) -> Result<ConnId, TcpError> {
    let (id, our_ip, segment_buf, segment_len) = {
        let mut t = TABLE.lock();
        let our_ip = t.our_ip;
        let local_port = t.alloc_port();
        let iss = t.next_isn();
        let id = t.alloc().ok_or(TcpError::NoSlot)?;

        let mut conn = TcpConn::new(local_port, remote_ip, remote_port, peer_mac, iss);
        conn.state = State::SynSent;
        conn.snd_nxt = iss.wrapping_add(1); // SYN consumes one seq

        // Build SYN segment with MSS option.
        let mut buf = [0u8; eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN + TCP_HDR_LEN + 4];
        eth::write_header(&mut buf, &peer_mac, &t.our_mac, eth::ETHERTYPE_IPV4);
        let tcp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
        let tcp_len = write_segment(
            &mut buf, tcp_off,
            &our_ip, &remote_ip,
            local_port, remote_port,
            iss, 0,
            SYN,
            DEFAULT_WINDOW,
            Some(DEFAULT_MSS),
            0,
        );
        ipv4::Ipv4Header::write(
            &mut buf[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
            &our_ip, &remote_ip, ipv4::PROTO_TCP, tcp_len, (id as u16).wrapping_add(1),
        );

        conn.pending.push(PendingSegment {
            seq: iss,
            data: Vec::new(),
            flags: SYN,
            seq_len: 1,
            last_tx_tick: pit::ticks(),
            retries: 0,
        });

        t.conns[id] = Some(conn);
        (id, our_ip, buf, tcp_off + tcp_len)
    };

    transmit(&segment_buf[..segment_len]);
    crate::serial::serial_println!(
        "[ NETSTACK ] TCP: SYN -> {}.{}.{}.{}:{} (conn={}, ours={}.{}.{}.{}, src_port=?)",
        remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3], remote_port, id,
        our_ip[0], our_ip[1], our_ip[2], our_ip[3],
    );
    Ok(id)
}

/// Queue `data` for transmission. Splits into MSS-sized chunks if needed.
pub fn send(id: ConnId, data: &[u8]) -> Result<(), TcpError> {
    if data.is_empty() { return Ok(()); }
    let segments: Vec<(Vec<u8>, u32, [u8; 1600], usize)> = {
        let mut t = TABLE.lock();
        let our_ip = t.our_ip;
        let our_mac = t.our_mac;
        let conn = t.conns.get_mut(id).and_then(|s| s.as_mut()).ok_or(TcpError::NotConnected)?;
        if conn.state != State::Established { return Err(TcpError::NotConnected); }

        let mss = conn.mss as usize;
        let mut out_segments = Vec::new();
        let mut offset = 0;
        while offset < data.len() {
            let chunk_len = (data.len() - offset).min(mss);
            let seq = conn.snd_nxt;
            let chunk = data[offset..offset + chunk_len].to_vec();

            let mut buf = [0u8; 1600];
            eth::write_header(&mut buf, &conn.peer_mac, &our_mac, eth::ETHERTYPE_IPV4);
            let tcp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
            // Payload first.
            buf[tcp_off + TCP_HDR_LEN..tcp_off + TCP_HDR_LEN + chunk_len].copy_from_slice(&chunk);
            let tcp_len = write_segment(
                &mut buf, tcp_off,
                &our_ip, &conn.remote_ip,
                conn.local_port, conn.remote_port,
                seq, conn.rcv_nxt,
                PSH | ACK,
                conn.rcv_wnd,
                None,
                chunk_len,
            );
            ipv4::Ipv4Header::write(
                &mut buf[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
                &our_ip, &conn.remote_ip, ipv4::PROTO_TCP, tcp_len,
                conn.snd_nxt as u16,
            );

            conn.pending.push(PendingSegment {
                seq,
                data: chunk.clone(),
                flags: PSH | ACK,
                seq_len: chunk_len as u32,
                last_tx_tick: pit::ticks(),
                retries: 0,
            });
            conn.snd_nxt = conn.snd_nxt.wrapping_add(chunk_len as u32);
            out_segments.push((chunk, seq, buf, tcp_off + tcp_len));
            offset += chunk_len;
        }
        out_segments
    };

    for (_chunk, _seq, buf, total_len) in segments {
        transmit(&buf[..total_len]);
    }
    Ok(())
}

/// Close the connection (send FIN). Transitions Established → FinWait1.
pub fn close(id: ConnId) -> Result<(), TcpError> {
    let (our_ip, our_mac, peer_mac, remote_ip, remote_port, local_port, seq, ack, win) = {
        let mut t = TABLE.lock();
        let our_ip = t.our_ip;
        let our_mac = t.our_mac;
        let conn = t.conns.get_mut(id).and_then(|s| s.as_mut()).ok_or(TcpError::NotConnected)?;
        if conn.state != State::Established && conn.state != State::CloseWait {
            return Err(TcpError::Closed);
        }
        let seq = conn.snd_nxt;
        let ack = conn.rcv_nxt;
        let win = conn.rcv_wnd;
        conn.pending.push(PendingSegment {
            seq,
            data: Vec::new(),
            flags: FIN | ACK,
            seq_len: 1,
            last_tx_tick: pit::ticks(),
            retries: 0,
        });
        conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
        conn.state = if conn.state == State::Established { State::FinWait1 } else { State::LastAck };
        (our_ip, our_mac, conn.peer_mac, conn.remote_ip, conn.remote_port, conn.local_port, seq, ack, win)
    };
    send_control(&our_ip, &our_mac, &peer_mac, &remote_ip, remote_port, local_port, seq, ack, FIN | ACK, win);
    Ok(())
}

fn send_control(our_ip: &[u8; 4], our_mac: &[u8; 6], peer_mac: &[u8; 6],
                remote_ip: &[u8; 4], remote_port: u16, local_port: u16,
                seq: u32, ack: u32, flags: u8, window: u16) {
    let mut buf = [0u8; eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN + TCP_HDR_LEN];
    eth::write_header(&mut buf, peer_mac, our_mac, eth::ETHERTYPE_IPV4);
    let tcp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
    let tcp_len = write_segment(
        &mut buf, tcp_off,
        our_ip, remote_ip,
        local_port, remote_port,
        seq, ack, flags, window, None, 0,
    );
    ipv4::Ipv4Header::write(
        &mut buf[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
        our_ip, remote_ip, ipv4::PROTO_TCP, tcp_len, seq as u16,
    );
    transmit(&buf[..tcp_off + tcp_len]);
}

fn transmit(frame: &[u8]) {
    let mut nic = drivers::NIC.lock();
    if let Some(n) = nic.as_mut() {
        let _ = n.send_frame(frame);
    }
}

/// Sequence number "less than" with wraparound (a - b interpreted as i32 < 0).
#[inline]
fn seq_lt(a: u32, b: u32) -> bool { ((a.wrapping_sub(b)) as i32) < 0 }
#[inline]
fn seq_le(a: u32, b: u32) -> bool { ((a.wrapping_sub(b)) as i32) <= 0 }

/// Dispatch an inbound TCP segment carried in an IPv4 packet.
pub fn handle_segment(ip: &ipv4::Ipv4Header, payload: &[u8]) {
    let Some((hdr, options, data)) = TcpHeader::parse(payload) else { return; };

    let outcome = {
        let mut t = TABLE.lock();
        let our_ip = t.our_ip;
        let our_mac = t.our_mac;
        let Some(id) = t.find_mut(hdr.dst_port, ip.src, hdr.src_port) else {
            return; // no connection — drop silently (we don't listen)
        };
        let conn = t.conns[id].as_mut().unwrap();
        let result = process_segment(conn, &hdr, options, data);
        Outcome { id, our_ip, our_mac, result }
    };

    // Issue any TX *after* the table lock has been released to keep lock order
    // table → NIC (never the other way).
    if let Some(ack) = outcome.result.ack {
        send_control(&outcome.our_ip, &outcome.our_mac, &ack.peer_mac,
                     &ack.remote_ip, ack.remote_port, ack.local_port,
                     ack.seq, ack.ack, ack.flags, ack.win);
    }

    match outcome.result.notify {
        Notify::None => {}
        Notify::Established { remote_ip, remote_port } => {
            crate::serial::serial_println!(
                "[ NETSTACK ] TCP: ESTABLISHED with {}.{}.{}.{}:{} (conn={})",
                remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3], remote_port, outcome.id,
            );
            super::stack::on_tcp_established(outcome.id);
        }
        Notify::DataReady { remote_ip, remote_port, bytes } => {
            crate::serial::serial_println!(
                "[ NETSTACK ] TCP: +{} B from {}.{}.{}.{}:{} (conn={})",
                bytes, remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3], remote_port, outcome.id,
            );
            super::stack::on_tcp_data(outcome.id);
        }
        Notify::Closed { remote_ip, remote_port } => {
            crate::serial::serial_println!(
                "[ NETSTACK ] TCP: CLOSED with {}.{}.{}.{}:{} (conn={})",
                remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3], remote_port, outcome.id,
            );
            super::stack::on_tcp_closed(outcome.id);
        }
    }
}

struct Outcome {
    id: ConnId,
    our_ip: [u8; 4],
    our_mac: [u8; 6],
    result: SegmentResult,
}

struct SegmentResult {
    notify: Notify,
    ack: Option<AckParams>,
}

#[derive(Clone, Copy)]
struct AckParams {
    seq: u32, ack: u32, win: u16, flags: u8,
    peer_mac: [u8; 6], remote_ip: [u8; 4], remote_port: u16, local_port: u16,
}

enum Notify {
    None,
    Established { remote_ip: [u8; 4], remote_port: u16 },
    DataReady { remote_ip: [u8; 4], remote_port: u16, bytes: usize },
    Closed { remote_ip: [u8; 4], remote_port: u16 },
}

/// Pure state-machine step: mutates `conn` and returns notification + an
/// optional ACK segment to emit (caller does the actual transmit).
fn process_segment(conn: &mut TcpConn, hdr: &TcpHeader, options: &[u8], data: &[u8]) -> SegmentResult {
    if hdr.flags & RST != 0 {
        conn.state = State::Closed;
        return SegmentResult {
            notify: Notify::Closed { remote_ip: conn.remote_ip, remote_port: conn.remote_port },
            ack: None,
        };
    }

    match conn.state {
        State::SynSent => {
            if hdr.flags & SYN == 0 || hdr.flags & ACK == 0 {
                return SegmentResult { notify: Notify::None, ack: None };
            }
            if hdr.ack != conn.iss.wrapping_add(1) {
                return SegmentResult {
                    notify: Notify::None,
                    ack: Some(AckParams {
                        seq: hdr.ack, ack: 0, win: 0, flags: RST | ACK,
                        peer_mac: conn.peer_mac, remote_ip: conn.remote_ip,
                        remote_port: conn.remote_port, local_port: conn.local_port,
                    }),
                };
            }
            conn.irs = hdr.seq;
            conn.rcv_nxt = hdr.seq.wrapping_add(1);
            conn.snd_una = hdr.ack;
            conn.snd_wnd = hdr.window;
            conn.mss = parse_mss(options).max(536);
            conn.state = State::Established;
            conn.pending.retain(|p| p.flags & SYN == 0); // our SYN was ACKed

            SegmentResult {
                notify: Notify::Established { remote_ip: conn.remote_ip, remote_port: conn.remote_port },
                ack: Some(ack_for(conn, ACK)),
            }
        }

        State::Established | State::FinWait1 | State::FinWait2 | State::CloseWait => {
            // Process ACK field.
            if hdr.flags & ACK != 0
                && seq_lt(conn.snd_una, hdr.ack)
                && seq_le(hdr.ack, conn.snd_nxt)
            {
                conn.snd_una = hdr.ack;
                conn.snd_wnd = hdr.window;
                conn.pending.retain(|p| {
                    let end = p.seq.wrapping_add(p.seq_len);
                    seq_lt(hdr.ack, end)
                });
                if conn.state == State::FinWait1 && conn.pending.iter().all(|p| p.flags & FIN == 0) {
                    conn.state = State::FinWait2;
                }
            }

            // Process payload (in-order only).
            let mut bytes_delivered = 0;
            if !data.is_empty() && hdr.seq == conn.rcv_nxt {
                let remaining_cap = RECV_BUF_LIMIT.saturating_sub(conn.recv_buf.len());
                let n = data.len().min(remaining_cap);
                conn.recv_buf.extend_from_slice(&data[..n]);
                conn.rcv_nxt = conn.rcv_nxt.wrapping_add(n as u32);
                bytes_delivered = n;
            }

            // Process FIN if its seq lines up with the contiguous receive stream.
            let mut peer_just_closed = false;
            if hdr.flags & FIN != 0 && hdr.seq.wrapping_add(data.len() as u32) == conn.rcv_nxt {
                conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                conn.peer_fin_seen = true;
                peer_just_closed = true;
                conn.state = match conn.state {
                    State::Established => State::CloseWait,
                    State::FinWait1    => State::TimeWait,
                    State::FinWait2    => State::TimeWait,
                    s => s,
                };
            }

            let need_ack = bytes_delivered > 0 || peer_just_closed || (hdr.flags & SYN) != 0;
            let ack = if need_ack { Some(ack_for(conn, ACK)) } else { None };

            let notify = if peer_just_closed && bytes_delivered == 0 {
                Notify::Closed { remote_ip: conn.remote_ip, remote_port: conn.remote_port }
            } else if bytes_delivered > 0 {
                Notify::DataReady {
                    remote_ip: conn.remote_ip,
                    remote_port: conn.remote_port,
                    bytes: bytes_delivered,
                }
            } else {
                Notify::None
            };
            SegmentResult { notify, ack }
        }

        State::LastAck => {
            if hdr.flags & ACK != 0 && hdr.ack == conn.snd_nxt {
                conn.state = State::Closed;
                SegmentResult {
                    notify: Notify::Closed { remote_ip: conn.remote_ip, remote_port: conn.remote_port },
                    ack: None,
                }
            } else {
                SegmentResult { notify: Notify::None, ack: None }
            }
        }

        State::TimeWait | State::Closed => SegmentResult { notify: Notify::None, ack: None },
    }
}

fn ack_for(conn: &TcpConn, flags: u8) -> AckParams {
    AckParams {
        seq: conn.snd_nxt,
        ack: conn.rcv_nxt,
        win: conn.rcv_wnd,
        flags,
        peer_mac: conn.peer_mac,
        remote_ip: conn.remote_ip,
        remote_port: conn.remote_port,
        local_port: conn.local_port,
    }
}

/// Drain `id`'s receive buffer into `out`. Returns bytes read.
pub fn read(id: ConnId, out: &mut [u8]) -> usize {
    let mut t = TABLE.lock();
    match t.conns.get_mut(id).and_then(|s| s.as_mut()) {
        Some(c) => c.read(out),
        None => 0,
    }
}

/// Inspect connection state.
pub fn state(id: ConnId) -> Option<State> {
    let t = TABLE.lock();
    t.conns.get(id).and_then(|s| s.as_ref()).map(|c| c.state)
}

/// Called from the timer IRQ to drive retransmissions and TIME_WAIT cleanup.
/// Uses try_lock; if the table is busy we just skip — segments will be
/// retransmitted on the next tick.
pub fn tick() {
    let Some(mut t) = TABLE.try_lock() else { return; };
    let now = pit::ticks();
    let our_ip = t.our_ip;
    let our_mac = t.our_mac;

    // Collect TX work to perform after dropping the lock.
    let mut to_send: Vec<([u8; 1600], usize)> = Vec::new();
    let mut to_drop: Vec<usize> = Vec::new();

    for (idx, slot) in t.conns.iter_mut().enumerate() {
        let Some(conn) = slot.as_mut() else { continue; };
        if conn.state == State::Closed {
            to_drop.push(idx);
            continue;
        }
        for seg in conn.pending.iter_mut() {
            let elapsed = now.saturating_sub(seg.last_tx_tick);
            let rto = (RTO_INITIAL_TICKS << (seg.retries.min(4) as u64)).min(RTO_MAX_TICKS);
            if elapsed < rto { continue; }
            if seg.retries >= MAX_RETRANSMITS {
                // Give up: signal closed.
                conn.state = State::Closed;
                continue;
            }

            // Rebuild and queue retransmission.
            let mut buf = [0u8; 1600];
            eth::write_header(&mut buf, &conn.peer_mac, &our_mac, eth::ETHERTYPE_IPV4);
            let tcp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
            let mss_opt = if seg.flags & SYN != 0 { Some(DEFAULT_MSS) } else { None };
            if !seg.data.is_empty() {
                buf[tcp_off + TCP_HDR_LEN..tcp_off + TCP_HDR_LEN + seg.data.len()]
                    .copy_from_slice(&seg.data);
            }
            let tcp_len = write_segment(
                &mut buf, tcp_off,
                &our_ip, &conn.remote_ip,
                conn.local_port, conn.remote_port,
                seg.seq, conn.rcv_nxt,
                seg.flags | ACK,
                conn.rcv_wnd,
                mss_opt,
                seg.data.len(),
            );
            ipv4::Ipv4Header::write(
                &mut buf[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
                &our_ip, &conn.remote_ip, ipv4::PROTO_TCP, tcp_len,
                seg.seq as u16,
            );

            seg.last_tx_tick = now;
            seg.retries += 1;
            to_send.push((buf, tcp_off + tcp_len));
        }
    }

    for idx in to_drop {
        t.conns[idx] = None;
    }

    drop(t);

    for (buf, len) in to_send {
        transmit(&buf[..len]);
    }
}
