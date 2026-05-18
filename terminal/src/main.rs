#![no_std]
#![no_main]

extern crate libc_lite;
extern crate racterm;

const O_RDWR: u32 = 0x0002;
const SIGTERM: i32 = 15;

const PTMX_PATH: &[u8] = b"/dev/ptmx\0";
const PTS0_PATH: &[u8] = b"/dev/pts0\0";
const SHELL_PATH: &[u8] = b"/bin/sh\0";

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    libc_lite::println("racterm: starting PTY session");

    let ptmx_fd = match libc_lite::open(PTMX_PATH, O_RDWR, 0) {
        Ok(fd) => fd,
        Err(_) => {
            libc_lite::eprintln("racterm: failed to open /dev/ptmx");
            return 1;
        }
    };

    let pts_fd = match libc_lite::open(PTS0_PATH, O_RDWR, 0) {
        Ok(fd) => fd,
        Err(_) => {
            let _ = libc_lite::close(ptmx_fd);
            libc_lite::eprintln("racterm: failed to open /dev/pts0");
            return 1;
        }
    };

    let shell_pid = match libc_lite::fork() {
        Ok(0) => spawn_shell_child(ptmx_fd, pts_fd),
        Ok(pid) => pid,
        Err(_) => {
            let _ = libc_lite::close(ptmx_fd);
            let _ = libc_lite::close(pts_fd);
            libc_lite::eprintln("racterm: fork for shell failed");
            return 1;
        }
    };

    let input_pid = match libc_lite::fork() {
        Ok(0) => relay_input_child(ptmx_fd, pts_fd),
        Ok(pid) => pid,
        Err(_) => {
            let _ = libc_lite::kill(shell_pid, SIGTERM);
            let mut status = 0;
            let _ = libc_lite::waitpid(shell_pid, &mut status, 0);
            let _ = libc_lite::close(ptmx_fd);
            let _ = libc_lite::close(pts_fd);
            libc_lite::eprintln("racterm: fork for input relay failed");
            return 1;
        }
    };

    let _ = libc_lite::close(pts_fd);

    let mut term = racterm::terminal::Terminal::new(25, 80);
    let mut out_buf = [0u8; 512];

    loop {
        match libc_lite::read(ptmx_fd, &mut out_buf) {
            Ok(0) => break,
            Ok(n) => {
                term.feed(&out_buf[..n]);
                let _ = libc_lite::write(1, &out_buf[..n]);
            }
            Err(_) => {
                let _ = libc_lite::sched_yield();
            }
        }

        let mut shell_status = 0;
        if let Ok(done) = libc_lite::waitpid(shell_pid, &mut shell_status, libc_lite::WNOHANG) {
            if done == shell_pid {
                break;
            }
        }
    }

    let _ = libc_lite::kill(input_pid, SIGTERM);

    let mut input_status = 0;
    let _ = libc_lite::waitpid(input_pid, &mut input_status, 0);

    let mut shell_status = 0;
    let _ = libc_lite::waitpid(shell_pid, &mut shell_status, 0);

    let _ = libc_lite::close(ptmx_fd);
    let _ = libc_lite::close(pts_fd);

    libc_lite::println("racterm: session ended");
    0
}

fn spawn_shell_child(ptmx_fd: i32, pts_fd: i32) -> ! {
    let _ = libc_lite::setsid();

    let _ = libc_lite::dup2(pts_fd, 0);
    let _ = libc_lite::dup2(pts_fd, 1);
    let _ = libc_lite::dup2(pts_fd, 2);

    let _ = libc_lite::close(ptmx_fd);
    let _ = libc_lite::close(pts_fd);

    if libc_lite::exec(SHELL_PATH).is_err() {
        libc_lite::eprintln("racterm: exec /bin/sh failed");
    }

    libc_lite::exit(127)
}

fn relay_input_child(ptmx_fd: i32, pts_fd: i32) -> ! {
    let _ = libc_lite::close(pts_fd);
    let mut in_buf = [0u8; 128];

    loop {
        match libc_lite::read(0, &mut in_buf) {
            Ok(0) => break,
            Ok(n) => {
                let mut written = 0usize;
                while written < n {
                    match libc_lite::write(ptmx_fd, &in_buf[written..n]) {
                        Ok(0) => break,
                        Ok(m) => written += m,
                        Err(_) => {
                            let _ = libc_lite::sched_yield();
                            break;
                        }
                    }
                }
            }
            Err(_) => {
                let _ = libc_lite::sched_yield();
            }
        }
    }

    let _ = libc_lite::close(ptmx_fd);
    libc_lite::exit(0)
}
