// RaCore — Networking
//
// Phase D: loopback SOCK_STREAM sockets (defined below).
// Phase E: real Ethernet/IPv4/ICMP stack — see the sub-modules.

extern crate alloc;

use alloc::vec::Vec;

pub mod arp;
pub mod dns;
pub mod eth;
pub mod icmp;
pub mod ipv4;
pub mod stack;
pub mod tcp;
pub mod udp;

pub const AF_INET: i32 = 2;
pub const SOCK_STREAM: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    Inval,
    NotSup,
    BadFd,
    AddrInUse,
    NotConn,
    ConnRefused,
    Again,
    Pipe,
}

type NetResult<T> = Result<T, NetError>;

type SocketId = usize;

#[derive(Clone)]
struct FdBinding {
    pid: u32,
    fd: i32,
    sid: SocketId,
}

struct Socket {
    domain: i32,
    stype: i32,
    protocol: i32,
    local_port: Option<u16>,
    peer: Option<SocketId>,
    listening: bool,
    backlog: usize,
    pending: Vec<SocketId>,
    recv_buf: Vec<u8>,
    shutdown_read: bool,
    shutdown_write: bool,
    peer_write_closed: bool,
}

impl Socket {
    fn new(domain: i32, stype: i32, protocol: i32) -> Self {
        Socket {
            domain,
            stype,
            protocol,
            local_port: None,
            peer: None,
            listening: false,
            backlog: 0,
            pending: Vec::new(),
            recv_buf: Vec::new(),
            shutdown_read: false,
            shutdown_write: false,
            peer_write_closed: false,
        }
    }
}

#[derive(Clone)]
struct TcpFdBinding {
    pid: u32,
    fd: i32,
    conn_id: tcp::ConnId,
}

struct NetState {
    sockets: Vec<Option<Socket>>,
    bindings: Vec<FdBinding>,
    tcp_bindings: Vec<TcpFdBinding>,
    next_ephemeral_port: u16,
}

impl NetState {
    fn new() -> Self {
        NetState {
            sockets: Vec::new(),
            bindings: Vec::new(),
            tcp_bindings: Vec::new(),
            next_ephemeral_port: 49152,
        }
    }

    fn alloc_sid(&mut self, sock: Socket) -> SocketId {
        for (i, slot) in self.sockets.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(sock);
                return i;
            }
        }
        self.sockets.push(Some(sock));
        self.sockets.len() - 1
    }

    fn socket_mut(&mut self, sid: SocketId) -> NetResult<&mut Socket> {
        self.sockets
            .get_mut(sid)
            .and_then(|s| s.as_mut())
            .ok_or(NetError::BadFd)
    }

    fn socket_ref(&self, sid: SocketId) -> NetResult<&Socket> {
        self.sockets
            .get(sid)
            .and_then(|s| s.as_ref())
            .ok_or(NetError::BadFd)
    }

    fn sid_by_fd(&self, pid: u32, fd: i32) -> NetResult<SocketId> {
        self.bindings
            .iter()
            .find(|b| b.pid == pid && b.fd == fd)
            .map(|b| b.sid)
            .ok_or(NetError::BadFd)
    }

    fn is_port_in_use(&self, port: u16) -> bool {
        self.sockets
            .iter()
            .flatten()
            .any(|s| s.local_port == Some(port) && s.listening)
    }

    fn alloc_ephemeral(&mut self) -> u16 {
        // Very small allocator for MVP.
        for _ in 0..4096 {
            let p = self.next_ephemeral_port;
            self.next_ephemeral_port = self.next_ephemeral_port.wrapping_add(1);
            if self.next_ephemeral_port < 49152 {
                self.next_ephemeral_port = 49152;
            }
            if !self.is_port_in_use(p) {
                return p;
            }
        }
        55000
    }
}

static mut NET_STATE: Option<NetState> = None;

fn state_mut() -> &'static mut NetState {
    unsafe {
        if (*core::ptr::addr_of!(NET_STATE)).is_none() {
            *core::ptr::addr_of_mut!(NET_STATE) = Some(NetState::new());
        }
        (*core::ptr::addr_of_mut!(NET_STATE)).as_mut().unwrap()
    }
}

pub fn create_socket(domain: i32, stype: i32, protocol: i32) -> NetResult<SocketId> {
    if domain != AF_INET || stype != SOCK_STREAM {
        return Err(NetError::NotSup);
    }
    let st = state_mut();
    Ok(st.alloc_sid(Socket::new(domain, stype, protocol)))
}

pub fn bind_fd(pid: u32, fd: i32, sid: SocketId) {
    let st = state_mut();
    st.bindings.push(FdBinding { pid, fd, sid });
}

pub fn close_fd(pid: u32, fd: i32) {
    let st = state_mut();
    if let Some(idx) = st.bindings.iter().position(|b| b.pid == pid && b.fd == fd) {
        let sid = st.bindings[idx].sid;
        st.bindings.swap_remove(idx);
        let _ = shutdown_sid(st, sid, 2);
    }
}

pub fn bind(pid: u32, fd: i32, port: u16) -> NetResult<()> {
    let st = state_mut();
    if st.is_port_in_use(port) {
        return Err(NetError::AddrInUse);
    }
    let sid = st.sid_by_fd(pid, fd)?;
    let sock = st.socket_mut(sid)?;
    sock.local_port = Some(port);
    Ok(())
}

pub fn listen(pid: u32, fd: i32, backlog: i32) -> NetResult<()> {
    let st = state_mut();
    let sid = st.sid_by_fd(pid, fd)?;
    let sock = st.socket_mut(sid)?;
    if sock.local_port.is_none() {
        return Err(NetError::Inval);
    }
    sock.listening = true;
    sock.backlog = (backlog.max(1) as usize).min(128);
    Ok(())
}

pub fn connect(pid: u32, fd: i32, port: u16) -> NetResult<()> {
    let st = state_mut();
    let client_sid = st.sid_by_fd(pid, fd)?;

    // Find listening socket on requested port.
    let listener_sid = st
        .sockets
        .iter()
        .enumerate()
        .find(|(_, s)| {
            s.as_ref()
                .map(|x| x.listening && x.local_port == Some(port))
                .unwrap_or(false)
        })
        .map(|(i, _)| i)
        .ok_or(NetError::ConnRefused)?;

    // Prepare client endpoint.
    let need_ephemeral = {
        let client = st.socket_mut(client_sid)?;
        if client.peer.is_some() || client.listening {
            return Err(NetError::Inval);
        }
        client.local_port.is_none()
    };
    if need_ephemeral {
        let eph = st.alloc_ephemeral();
        let client = st.socket_mut(client_sid)?;
        client.local_port = Some(eph);
    }

    // Create server-side accepted socket.
    let server_local = port;
    let server_sid = st.alloc_sid(Socket::new(AF_INET, SOCK_STREAM, 0));
    {
        let server_sock = st.socket_mut(server_sid)?;
        server_sock.local_port = Some(server_local);
        server_sock.peer = Some(client_sid);
    }

    {
        let client = st.socket_mut(client_sid)?;
        client.peer = Some(server_sid);
    }

    {
        let listener = st.socket_mut(listener_sid)?;
        if listener.pending.len() >= listener.backlog {
            return Err(NetError::Again);
        }
        listener.pending.push(server_sid);
    }

    Ok(())
}

pub fn accept(pid: u32, fd: i32) -> NetResult<SocketId> {
    let st = state_mut();
    let listener_sid = st.sid_by_fd(pid, fd)?;
    let listener = st.socket_mut(listener_sid)?;
    if !listener.listening {
        return Err(NetError::Inval);
    }
    if listener.pending.is_empty() {
        return Err(NetError::Again);
    }
    Ok(listener.pending.remove(0))
}

pub fn send(pid: u32, fd: i32, data: &[u8]) -> NetResult<usize> {
    let st = state_mut();
    let sid = st.sid_by_fd(pid, fd)?;
    let peer_sid = {
        let sock = st.socket_ref(sid)?;
        if sock.shutdown_write {
            return Err(NetError::Pipe);
        }
        sock.peer.ok_or(NetError::NotConn)?
    };

    let peer = st.socket_mut(peer_sid)?;
    if peer.shutdown_read {
        return Err(NetError::Pipe);
    }
    peer.recv_buf.extend_from_slice(data);
    Ok(data.len())
}

pub fn recv(pid: u32, fd: i32, out: &mut [u8]) -> NetResult<usize> {
    let st = state_mut();
    let sid = st.sid_by_fd(pid, fd)?;
    let sock = st.socket_mut(sid)?;

    if sock.shutdown_read {
        return Ok(0);
    }
    if sock.recv_buf.is_empty() {
        if sock.peer_write_closed {
            return Ok(0);
        }
        return Err(NetError::Again);
    }

    let n = out.len().min(sock.recv_buf.len());
    out[..n].copy_from_slice(&sock.recv_buf[..n]);
    sock.recv_buf.drain(..n);
    Ok(n)
}

fn shutdown_sid(st: &mut NetState, sid: SocketId, how: i32) -> NetResult<()> {
    let (set_rd, set_wr) = match how {
        0 => (true, false),
        1 => (false, true),
        2 => (true, true),
        _ => return Err(NetError::Inval),
    };

    let peer_sid = {
        let sock = st.socket_mut(sid)?;
        if set_rd {
            sock.shutdown_read = true;
        }
        if set_wr {
            sock.shutdown_write = true;
        }
        sock.peer
    };

    if set_wr {
        if let Some(psid) = peer_sid {
            if let Ok(peer) = st.socket_mut(psid) {
                peer.peer_write_closed = true;
            }
        }
    }

    Ok(())
}

pub fn shutdown(pid: u32, fd: i32, how: i32) -> NetResult<()> {
    let st = state_mut();
    let sid = st.sid_by_fd(pid, fd)?;
    shutdown_sid(st, sid, how)
}

pub fn sockname(pid: u32, fd: i32) -> NetResult<(u16, u32)> {
    let st = state_mut();
    let sid = st.sid_by_fd(pid, fd)?;
    let sock = st.socket_ref(sid)?;
    Ok((sock.local_port.unwrap_or(0), 0x7F00_0001))
}

pub fn peername(pid: u32, fd: i32) -> NetResult<(u16, u32)> {
    let st = state_mut();
    let sid = st.sid_by_fd(pid, fd)?;
    let sock = st.socket_ref(sid)?;
    let peer_sid = sock.peer.ok_or(NetError::NotConn)?;
    let peer = st.socket_ref(peer_sid)?;
    Ok((peer.local_port.unwrap_or(0), 0x7F00_0001))
}

// ─────────────────────────────────────────────────────────────
// Phase E krok 5: TCP fd ↔ ConnId mapping
//
// A POSIX-style fd may be bound to a loopback socket (the FdBinding above)
// OR to a real TCP connection. The two tables are independent: a fd starts
// in `bindings` after sys_socket; sys_connect upgrades it to `tcp_bindings`
// when the destination is not loopback.
// ─────────────────────────────────────────────────────────────

pub fn bind_fd_tcp(pid: u32, fd: i32, conn_id: tcp::ConnId) {
    let st = state_mut();
    st.tcp_bindings.retain(|b| !(b.pid == pid && b.fd == fd));
    st.tcp_bindings.push(TcpFdBinding { pid, fd, conn_id });
}

pub fn tcp_id_by_fd(pid: u32, fd: i32) -> Option<tcp::ConnId> {
    let st = state_mut();
    st.tcp_bindings
        .iter()
        .find(|b| b.pid == pid && b.fd == fd)
        .map(|b| b.conn_id)
}

/// Remove a TCP binding (called from sys_close). Returns the conn id so the
/// caller can issue tcp::close on it.
pub fn close_fd_tcp(pid: u32, fd: i32) -> Option<tcp::ConnId> {
    let st = state_mut();
    let idx = st
        .tcp_bindings
        .iter()
        .position(|b| b.pid == pid && b.fd == fd)?;
    Some(st.tcp_bindings.swap_remove(idx).conn_id)
}
