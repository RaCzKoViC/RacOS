// RaCore — IP stack orchestrator (Phase E step 2)
//
// Glues virtio-net (link layer) to the new Ethernet/ARP/IPv4/ICMP modules.
// Static configuration matches QEMU user-mode networking:
//
//     RacOS IP  = 10.0.2.15
//     Netmask   = 255.255.255.0
//     Gateway   = 10.0.2.2  (QEMU emulates this host)
//     DNS       = 10.0.2.3
//
// The stack does not yet do interrupt-driven RX; `poll()` is called from the
// PIT tick handler (~1 kHz). That is sufficient for ARP/ICMP exchange.

use crate::drivers;
use crate::sync::SpinLock;

use super::{arp, dns, eth, icmp, ipv4, tcp, udp};

pub const OUR_IP: [u8; 4] = [10, 0, 2, 15];
pub const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];
pub const DNS_IP: [u8; 4] = [10, 0, 2, 3];
pub const NETMASK: [u8; 4] = [255, 255, 255, 0];

/// What the demo flow is currently doing — drives the state machine that the
/// ARP/ICMP/UDP receive handlers consult to decide what to send next.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DemoStage {
    Idle,
    AwaitGatewayArp,     // sent ARP for 10.0.2.2; waiting on reply
    AwaitIcmpReply,      // sent ICMP echo; waiting on reply
    AwaitDnsArp,         // sent ARP for 10.0.2.3; waiting on reply
    AwaitDnsReply,       // sent DNS query; waiting on UDP response
    AwaitTcpEstablished, // sent SYN; waiting on SYN+ACK
    AwaitHttpResponse,   // sent HTTP GET; waiting on bytes
    Done,
}

pub struct Stack {
    mac: [u8; 6],
    arp_cache: arp::ArpCache,
    ip_id: u16,
    next_icmp_seq: u16,
    /// Counter incremented on every ICMP echo reply we receive.
    pub echo_replies: u32,
    /// Demo state machine (Phase E krok 1–3 self-test).
    demo: DemoStage,
    /// Ephemeral source port used by the DNS demo. Matched against incoming UDP dst port.
    dns_src_port: u16,
    /// Last DNS transaction id sent.
    dns_id: u16,
    /// Hostname we asked about (static literal kept here for logging only).
    dns_query: &'static str,
    /// IP address learned from DNS (set when AwaitDnsReply transitions to TCP).
    resolved_ip: [u8; 4],
    /// Active TCP connection used by the HTTP demo.
    http_conn: Option<tcp::ConnId>,
    /// Userland gethostbyname() request in flight. Independent of the demo
    /// flow — both can coexist because the pending state is matched by the
    /// ephemeral source port carried in the UDP reply.
    query_pending: Option<DnsQueryState>,
}

#[derive(Clone, Copy)]
struct DnsQueryState {
    id: u16,
    src_port: u16,
    completed: bool,
    resolved_ip: [u8; 4],
    failed: bool,
}

impl Stack {
    pub const fn new() -> Self {
        Stack {
            mac: [0; 6],
            arp_cache: arp::ArpCache::new(),
            ip_id: 1,
            next_icmp_seq: 1,
            echo_replies: 0,
            demo: DemoStage::Idle,
            dns_src_port: 0,
            dns_id: 0,
            dns_query: "",
            resolved_ip: [0; 4],
            http_conn: None,
            query_pending: None,
        }
    }
}

static STACK: SpinLock<Stack> = SpinLock::new(Stack::new());

pub fn init() {
    // Snapshot the NIC's MAC into the stack state.
    let mac = {
        let nic = drivers::NIC.lock();
        match nic.as_ref() {
            Some(n) => n.mac,
            None => {
                crate::serial::serial_println!("[ NETSTACK ] no NIC, skipping init");
                return;
            }
        }
    };

    {
        let mut s = STACK.lock();
        s.mac = mac;
    }
    tcp::init(OUR_IP, mac);
    crate::serial::serial_println!(
        "[ NETSTACK ] up: ip={}.{}.{}.{}, gw={}.{}.{}.{}, mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        OUR_IP[0], OUR_IP[1], OUR_IP[2], OUR_IP[3],
        GATEWAY_IP[0], GATEWAY_IP[1], GATEWAY_IP[2], GATEWAY_IP[3],
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
    );
}

/// Return the L2 MAC to use when sending to `ip` — direct ARP lookup for
/// hosts in our /24, gateway MAC for everything else.
pub fn next_hop_mac(ip: [u8; 4]) -> Option<[u8; 6]> {
    let on_subnet = ip[0] == OUR_IP[0] && ip[1] == OUR_IP[1] && ip[2] == OUR_IP[2];
    let target = if on_subnet { ip } else { GATEWAY_IP };
    let s = STACK.lock();
    s.arp_cache.lookup(target)
}

/// Synchronous DNS A-record resolution. Blocks (with interrupts on, so the
/// PIT keeps pumping the NIC) until a reply arrives or the deadline passes.
/// `name` must be a valid DNS hostname; returns the first A record found.
pub fn resolve(name: &str) -> Option<[u8; 4]> {
    // Need the DNS server's MAC already in the ARP cache. The demo populates
    // it on boot; on a clean reset the user could trigger an explicit ARP.
    let dns_mac = next_hop_mac(DNS_IP)?;
    let id: u16 = (crate::interrupts::pit::ticks() as u16) ^ 0xC0DE;
    // Pick an ephemeral source port distinct from the demo's 52015.
    let src_port: u16 = 60000u16.wrapping_add((crate::interrupts::pit::ticks() as u16) & 0x0FFF);

    // Build query payload.
    let mut payload = [0u8; 256];
    let qlen = match dns::build_query_a(&mut payload, id, name) {
        Ok(n) => n,
        Err(_) => return None,
    };
    {
        let mut s = STACK.lock();
        s.query_pending = Some(DnsQueryState {
            id,
            src_port,
            completed: false,
            resolved_ip: [0; 4],
            failed: false,
        });
    }

    // Build + send the UDP datagram. We mirror send_udp's logic but use
    // `dns_mac` directly to avoid an extra arp_cache lookup.
    let udp_len = udp::UDP_HDR_LEN + qlen;
    let ip_len = ipv4::IPV4_HDR_LEN + udp_len;
    let frame_len = eth::ETH_HDR_LEN + ip_len;
    let mut buf = [0u8; 512];
    let (our_mac, ip_id) = {
        let mut s = STACK.lock();
        s.ip_id = s.ip_id.wrapping_add(1);
        (s.mac, s.ip_id)
    };
    eth::write_header(&mut buf, &dns_mac, &our_mac, eth::ETHERTYPE_IPV4);
    let udp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
    buf[udp_off + udp::UDP_HDR_LEN..udp_off + udp_len].copy_from_slice(&payload[..qlen]);
    udp::write(
        &mut buf[udp_off..udp_off + udp_len],
        &OUR_IP,
        &DNS_IP,
        src_port,
        dns::PORT,
        qlen,
    );
    ipv4::Ipv4Header::write(
        &mut buf[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
        &OUR_IP,
        &DNS_IP,
        ipv4::PROTO_UDP,
        udp_len,
        ip_id,
    );
    transmit(&buf[..frame_len]);

    // Wait up to 3 seconds for the reply (or 1 retransmission).
    // The SYSCALL path enters with IF=0 (SFMASK clears it). Enable interrupts
    // for the duration of the wait so the PIT can fire and poll the NIC.
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
    }
    let start = crate::interrupts::pit::ticks();
    let mut retransmitted = false;
    let result = loop {
        let now = crate::interrupts::pit::ticks();
        let elapsed = now.saturating_sub(start);

        let snapshot = {
            let s = STACK.lock();
            s.query_pending
        };
        let Some(q) = snapshot else {
            break None;
        };
        if q.completed && !q.failed {
            break Some(q.resolved_ip);
        }
        if q.failed {
            break None;
        }

        if elapsed > 3000 {
            break None;
        }
        if !retransmitted && elapsed > 1000 {
            retransmitted = true;
            transmit(&buf[..frame_len]);
        }
        // Drain any RX frames now (no IRQ-driven poll while we wait).
        poll();
        core::hint::spin_loop();
    };
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    // Clear pending state on timeout.
    if result.is_none() {
        let mut s = STACK.lock();
        s.query_pending = None;
    }
    result
}

/// Send an ARP request for `target_ip`. Drops the lock before TX.
pub fn send_arp_request(target_ip: [u8; 4]) {
    let mut buf = [0u8; arp::ARP_FRAME_LEN];
    {
        let s = STACK.lock();
        arp::build_request(&mut buf, &s.mac, &OUR_IP, &target_ip);
    }
    crate::serial::serial_println!(
        "[ NETSTACK ] ARP: who-has {}.{}.{}.{}? request out ({} B)",
        target_ip[0],
        target_ip[1],
        target_ip[2],
        target_ip[3],
        buf.len(),
    );
    transmit(&buf);
}

/// Send an ICMP echo to `dst_ip`. The frame uses the cached MAC for `dst_ip`
/// (or the gateway, if `dst_ip` is off-subnet — but for MVP we assume LAN).
pub fn send_icmp_echo(dst_ip: [u8; 4], payload: &[u8]) -> bool {
    // Resolve destination MAC.
    let (dst_mac, src_mac, ip_id, seq) = {
        let mut s = STACK.lock();
        let mac = match s.arp_cache.lookup(dst_ip) {
            Some(m) => m,
            None => return false,
        };
        let src_mac = s.mac;
        s.ip_id = s.ip_id.wrapping_add(1);
        let id = s.ip_id;
        let seq = s.next_icmp_seq;
        s.next_icmp_seq = s.next_icmp_seq.wrapping_add(1);
        (mac, src_mac, id, seq)
    };

    let icmp_len = icmp::ICMP_HDR_LEN + payload.len();
    let ip_len = ipv4::IPV4_HDR_LEN + icmp_len;
    let frame_len = eth::ETH_HDR_LEN + ip_len;
    if frame_len > 1500 {
        return false;
    }

    let mut buf = [0u8; 128];
    eth::write_header(&mut buf, &dst_mac, &src_mac, eth::ETHERTYPE_IPV4);

    // ICMP first (payload then header — write_echo needs payload in place for checksum).
    let icmp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
    buf[icmp_off + icmp::ICMP_HDR_LEN..icmp_off + icmp_len].copy_from_slice(payload);
    icmp::write_echo(
        &mut buf[icmp_off..icmp_off + icmp_len],
        icmp::TYPE_ECHO_REQUEST,
        0x1234,
        seq,
        payload.len(),
    );

    // IPv4 last (checksum covers header only).
    ipv4::Ipv4Header::write(
        &mut buf[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
        &OUR_IP,
        &dst_ip,
        ipv4::PROTO_ICMP,
        icmp_len,
        ip_id,
    );

    transmit(&buf[..frame_len]);
    true
}

/// Kick off the Phase-E demo: ARP gateway → ICMP echo → ARP DNS → DNS A query.
/// `query` is the hostname to resolve (must outlive the call — typically a literal).
pub fn start_demo(query: &'static str) {
    {
        let mut s = STACK.lock();
        s.demo = DemoStage::AwaitGatewayArp;
        s.dns_query = query;
    }
    send_arp_request(GATEWAY_IP);
}

/// Send a UDP datagram to `dst_ip:dst_port` from `src_port`. Resolves dst MAC
/// via the ARP cache; returns false if MAC is unknown or payload too large.
pub fn send_udp(dst_ip: [u8; 4], src_port: u16, dst_port: u16, payload: &[u8]) -> bool {
    let udp_len = udp::UDP_HDR_LEN + payload.len();
    let ip_len = ipv4::IPV4_HDR_LEN + udp_len;
    let frame_len = eth::ETH_HDR_LEN + ip_len;
    if frame_len > 1500 {
        return false;
    }

    let (dst_mac, src_mac, ip_id) = {
        let mut s = STACK.lock();
        let mac = match s.arp_cache.lookup(dst_ip) {
            Some(m) => m,
            None => return false,
        };
        s.ip_id = s.ip_id.wrapping_add(1);
        (mac, s.mac, s.ip_id)
    };

    let mut buf = [0u8; 1514];
    eth::write_header(&mut buf, &dst_mac, &src_mac, eth::ETHERTYPE_IPV4);

    // UDP (payload first so checksum covers it).
    let udp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
    buf[udp_off + udp::UDP_HDR_LEN..udp_off + udp_len].copy_from_slice(payload);
    udp::write(
        &mut buf[udp_off..udp_off + udp_len],
        &OUR_IP,
        &dst_ip,
        src_port,
        dst_port,
        payload.len(),
    );

    // IPv4 header.
    ipv4::Ipv4Header::write(
        &mut buf[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
        &OUR_IP,
        &dst_ip,
        ipv4::PROTO_UDP,
        udp_len,
        ip_id,
    );

    transmit(&buf[..frame_len]);
    true
}

/// Issue a DNS A query and arm the demo to expect the response. The transaction
/// id and ephemeral source port are recorded so we can match the reply.
fn send_dns_query(name: &'static str) -> bool {
    let id: u16 = 0xABCD;
    let src_port: u16 = 52015; // arbitrary ephemeral

    let mut payload = [0u8; 128];
    let qlen = match dns::build_query_a(&mut payload, id, name) {
        Ok(n) => n,
        Err(_) => return false,
    };
    {
        let mut s = STACK.lock();
        s.dns_src_port = src_port;
        s.dns_id = id;
    }
    if !send_udp(DNS_IP, src_port, dns::PORT, &payload[..qlen]) {
        return false;
    }
    crate::serial::serial_println!(
        "[ NETSTACK ] DNS: query A {} -> {}.{}.{}.{}:{} ({} B)",
        name,
        DNS_IP[0],
        DNS_IP[1],
        DNS_IP[2],
        DNS_IP[3],
        dns::PORT,
        qlen,
    );
    true
}

fn transmit(frame: &[u8]) {
    let mut nic = drivers::NIC.lock();
    if let Some(n) = nic.as_mut() {
        let _ = n.send_frame(frame);
    }
}

/// Pull all available frames from the NIC and dispatch them.
/// Uses try_lock so calling from an interrupt handler is safe.
pub fn poll() {
    let mut buf = [0u8; 1600];
    loop {
        let n = {
            let Some(mut nic) = drivers::NIC.try_lock() else {
                return;
            };
            match nic.as_mut() {
                Some(n) => n.poll_rx(&mut buf),
                None => return,
            }
        };
        let Some(len) = n else {
            return;
        };
        if len == 0 {
            return;
        }
        on_frame(&buf[..len]);
    }
}

fn on_frame(frame: &[u8]) {
    let Some((eth_hdr, payload)) = eth::EthHeader::parse(frame) else {
        return;
    };
    crate::serial::serial_println!(
        "[ NETSTACK ] RX frame {} B, ethertype=0x{:04X}, src={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        frame.len(), eth_hdr.ethertype,
        eth_hdr.src[0], eth_hdr.src[1], eth_hdr.src[2],
        eth_hdr.src[3], eth_hdr.src[4], eth_hdr.src[5],
    );
    match eth_hdr.ethertype {
        eth::ETHERTYPE_ARP => handle_arp(payload),
        eth::ETHERTYPE_IPV4 => handle_ipv4(payload),
        _ => {}
    }
}

fn handle_arp(payload: &[u8]) {
    let Some(pkt) = arp::ArpPacket::parse(payload) else {
        return;
    };

    // Learn from any ARP we see. Decide demo follow-up based on which address resolved.
    let next: DemoFollowup = {
        let mut s = STACK.lock();
        s.arp_cache.insert(pkt.sender_ip, pkt.sender_mac);
        match (s.demo, pkt.sender_ip) {
            (DemoStage::AwaitGatewayArp, ip) if ip == GATEWAY_IP => {
                crate::serial::serial_println!(
                    "[ NETSTACK ] ARP: gateway {}.{}.{}.{} is at {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    ip[0], ip[1], ip[2], ip[3],
                    pkt.sender_mac[0], pkt.sender_mac[1], pkt.sender_mac[2],
                    pkt.sender_mac[3], pkt.sender_mac[4], pkt.sender_mac[5],
                );
                s.demo = DemoStage::AwaitIcmpReply;
                DemoFollowup::PingGateway
            }
            (DemoStage::AwaitDnsArp, ip) if ip == DNS_IP => {
                crate::serial::serial_println!(
                    "[ NETSTACK ] ARP: dns {}.{}.{}.{} is at {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    ip[0], ip[1], ip[2], ip[3],
                    pkt.sender_mac[0], pkt.sender_mac[1], pkt.sender_mac[2],
                    pkt.sender_mac[3], pkt.sender_mac[4], pkt.sender_mac[5],
                );
                s.demo = DemoStage::AwaitDnsReply;
                DemoFollowup::SendDns(s.dns_query)
            }
            _ => DemoFollowup::None,
        }
    };

    // Reply to ARP requests directed at us.
    if pkt.op == arp::OP_REQUEST && pkt.target_ip == OUR_IP {
        let (our_mac, peer_mac, peer_ip) = {
            let s = STACK.lock();
            (s.mac, pkt.sender_mac, pkt.sender_ip)
        };
        let mut buf = [0u8; arp::ARP_FRAME_LEN];
        arp::build_reply(&mut buf, &our_mac, &OUR_IP, &peer_mac, &peer_ip);
        transmit(&buf);
    }

    match next {
        DemoFollowup::PingGateway => {
            let payload = b"RacOS ICMP echo Phase E.2";
            if send_icmp_echo(GATEWAY_IP, payload) {
                crate::serial::serial_println!("[ NETSTACK ] ICMP: sent echo request to gateway");
            }
        }
        DemoFollowup::SendDns(name) => {
            let _ = send_dns_query(name);
        }
        DemoFollowup::None => {}
    }
}

enum DemoFollowup {
    None,
    PingGateway,
    SendDns(&'static str),
}

fn handle_ipv4(payload: &[u8]) {
    let Some((hdr, ip_payload)) = ipv4::Ipv4Header::parse(payload) else {
        return;
    };
    if hdr.dst != OUR_IP {
        return;
    }
    match hdr.protocol {
        ipv4::PROTO_ICMP => handle_icmp(&hdr, ip_payload),
        ipv4::PROTO_UDP => handle_udp(&hdr, ip_payload),
        ipv4::PROTO_TCP => tcp::handle_segment(&hdr, ip_payload),
        _ => {}
    }
}

fn handle_udp(ip: &ipv4::Ipv4Header, payload: &[u8]) {
    let Some((hdr, data)) = udp::UdpHeader::parse(payload) else {
        return;
    };
    crate::serial::serial_println!(
        "[ NETSTACK ] UDP: {}.{}.{}.{}:{} -> :{}, {} B payload",
        ip.src[0],
        ip.src[1],
        ip.src[2],
        ip.src[3],
        hdr.src_port,
        hdr.dst_port,
        data.len(),
    );

    // Userland gethostbyname() reply matching. Takes precedence over the demo.
    let userland_match = {
        let s = STACK.lock();
        match s.query_pending {
            Some(q)
                if ip.src == DNS_IP && hdr.src_port == dns::PORT && hdr.dst_port == q.src_port =>
            {
                Some(q)
            }
            _ => None,
        }
    };
    if let Some(q) = userland_match {
        let mut state = q;
        if data.len() >= 2 {
            let rid = u16::from_be_bytes([data[0], data[1]]);
            if rid != q.id {
                return; // unexpected id; let it timeout
            }
        }
        match dns::parse_first_a(data) {
            Ok(ip4) => {
                state.resolved_ip = ip4;
                state.completed = true;
            }
            Err(_) => {
                state.failed = true;
                state.completed = true;
            }
        }
        let mut s = STACK.lock();
        s.query_pending = Some(state);
        return;
    }

    // DNS reply matching for the boot-time demo flow.
    let (expected_port, expected_id, query, stage) = {
        let s = STACK.lock();
        (s.dns_src_port, s.dns_id, s.dns_query, s.demo)
    };
    if stage == DemoStage::AwaitDnsReply
        && ip.src == DNS_IP
        && hdr.src_port == dns::PORT
        && hdr.dst_port == expected_port
    {
        // Verify transaction id.
        if data.len() >= 2 {
            let rid = u16::from_be_bytes([data[0], data[1]]);
            if rid != expected_id {
                crate::serial::serial_println!(
                    "[ NETSTACK ] DNS: id mismatch (got {:04x}, want {:04x})",
                    rid,
                    expected_id
                );
                return;
            }
        }
        match dns::parse_first_a(data) {
            Ok(ip4) => {
                crate::serial::serial_println!(
                    "[ NETSTACK ] DNS: {} is at {}.{}.{}.{}",
                    query,
                    ip4[0],
                    ip4[1],
                    ip4[2],
                    ip4[3],
                );
                {
                    let mut s = STACK.lock();
                    s.resolved_ip = ip4;
                    s.demo = DemoStage::AwaitTcpEstablished;
                }
                // Kick TCP handshake (off-subnet → use gateway MAC).
                let peer_mac = match next_hop_mac(ip4) {
                    Some(m) => m,
                    None => {
                        crate::serial::serial_println!(
                            "[ NETSTACK ] TCP: no MAC for next hop, aborting demo"
                        );
                        let mut s = STACK.lock();
                        s.demo = DemoStage::Done;
                        return;
                    }
                };
                match tcp::connect(ip4, 80, peer_mac) {
                    Ok(id) => {
                        let mut s = STACK.lock();
                        s.http_conn = Some(id);
                    }
                    Err(e) => {
                        crate::serial::serial_println!("[ NETSTACK ] TCP: connect failed: {:?}", e);
                        let mut s = STACK.lock();
                        s.demo = DemoStage::Done;
                    }
                }
            }
            Err(e) => {
                crate::serial::serial_println!("[ NETSTACK ] DNS: parse error {:?}", e);
            }
        }
    }
}

/// Called by the TCP layer when a connection enters ESTABLISHED. The demo
/// uses this to send a minimal HTTP/1.0 GET request.
pub fn on_tcp_established(id: tcp::ConnId) {
    let interested = {
        let s = STACK.lock();
        s.demo == DemoStage::AwaitTcpEstablished && s.http_conn == Some(id)
    };
    if !interested {
        return;
    }

    let request: &[u8] = b"GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n";
    match tcp::send(id, request) {
        Ok(()) => {
            crate::serial::serial_println!(
                "[ NETSTACK ] HTTP: GET sent ({} B), waiting for response",
                request.len(),
            );
            let mut s = STACK.lock();
            s.demo = DemoStage::AwaitHttpResponse;
        }
        Err(e) => {
            crate::serial::serial_println!("[ NETSTACK ] HTTP: send failed: {:?}", e);
            let mut s = STACK.lock();
            s.demo = DemoStage::Done;
        }
    }
}

/// Called when new bytes arrive on a TCP connection. Demo prints the first
/// HTTP status line, then asks TCP to close gracefully.
pub fn on_tcp_data(id: tcp::ConnId) {
    let interested = {
        let s = STACK.lock();
        s.demo == DemoStage::AwaitHttpResponse && s.http_conn == Some(id)
    };
    if !interested {
        return;
    }

    let mut buf = [0u8; 512];
    let n = tcp::read(id, &mut buf);
    if n == 0 {
        return;
    }

    // Extract first line (HTTP status).
    let mut end = n;
    for (i, w) in buf[..n].windows(2).enumerate() {
        if w == b"\r\n" {
            end = i;
            break;
        }
    }
    let line = core::str::from_utf8(&buf[..end]).unwrap_or("<non-utf8>");
    crate::serial::serial_println!(
        "[ NETSTACK ] HTTP: status line: {}  ({} B in this chunk)",
        line,
        n,
    );

    // Demo done — close gracefully. Further data on the connection is dropped.
    {
        let mut s = STACK.lock();
        s.demo = DemoStage::Done;
    }
    let _ = tcp::close(id);
    crate::serial::serial_println!("[ NETSTACK ] HTTP: closing — Phase E krok 4 done");
}

/// Called when a TCP connection enters CLOSED (peer RST, FIN exchange done,
/// or retransmit-budget exhausted).
pub fn on_tcp_closed(id: tcp::ConnId) {
    let mut s = STACK.lock();
    if s.http_conn == Some(id) {
        s.http_conn = None;
    }
}

fn handle_icmp(ip: &ipv4::Ipv4Header, msg: &[u8]) {
    let Some((t, _code, id, seq, payload)) = icmp::parse(msg) else {
        return;
    };
    match t {
        icmp::TYPE_ECHO_REPLY => {
            let advance_to_dns = {
                let mut s = STACK.lock();
                s.echo_replies = s.echo_replies.wrapping_add(1);
                crate::serial::serial_println!(
                    "[ NETSTACK ] ICMP: echo reply from {}.{}.{}.{} id={:04x} seq={} payload={}B (total {})",
                    ip.src[0], ip.src[1], ip.src[2], ip.src[3], id, seq, payload.len(),
                    s.echo_replies,
                );
                if s.demo == DemoStage::AwaitIcmpReply {
                    s.demo = DemoStage::AwaitDnsArp;
                    true
                } else {
                    false
                }
            };
            if advance_to_dns {
                send_arp_request(DNS_IP);
            }
        }
        icmp::TYPE_ECHO_REQUEST => {
            // Respond to anyone pinging us.
            let mut out = [0u8; 128];
            let icmp_len = icmp::ICMP_HDR_LEN + payload.len();
            if eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN + icmp_len > out.len() {
                return;
            }

            // Need peer MAC: look up in cache (must have been learned).
            let dst_mac = {
                let s = STACK.lock();
                s.arp_cache.lookup(ip.src)
            };
            let Some(dst_mac) = dst_mac else {
                return;
            };

            let (src_mac, ip_id) = {
                let mut s = STACK.lock();
                s.ip_id = s.ip_id.wrapping_add(1);
                (s.mac, s.ip_id)
            };

            eth::write_header(&mut out, &dst_mac, &src_mac, eth::ETHERTYPE_IPV4);
            let icmp_off = eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN;
            out[icmp_off + icmp::ICMP_HDR_LEN..icmp_off + icmp_len].copy_from_slice(payload);
            icmp::write_echo(
                &mut out[icmp_off..icmp_off + icmp_len],
                icmp::TYPE_ECHO_REPLY,
                id,
                seq,
                payload.len(),
            );
            ipv4::Ipv4Header::write(
                &mut out[eth::ETH_HDR_LEN..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN],
                &OUR_IP,
                &ip.src,
                ipv4::PROTO_ICMP,
                icmp_len,
                ip_id,
            );
            transmit(&out[..eth::ETH_HDR_LEN + ipv4::IPV4_HDR_LEN + icmp_len]);
        }
        _ => {}
    }
}
