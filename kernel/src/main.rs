// RaCore — Kernel entry point
//
// This is the main kernel crate for RacOS. It targets x86_64 bare metal
// (no_std, no_main) and is loaded by the UEFI bootloader.

#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]
#![allow(dead_code, unused_imports, unused_variables, function_casts_as_integer, unreachable_code)]

extern crate alloc;

mod arch;
mod boot;
mod drivers;
mod elf;
mod fb_console;
mod interrupts;
mod mm;
mod net;
mod security;
mod serial;
mod syscall;
mod task;
mod tty;
mod vfs;
#[macro_use]
mod print;
mod panic;
mod shell_fs;
mod sync;
mod mod_loader;

use boot::BootInfo;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

const RUN_KERNEL_SELF_TESTS: bool = false;
const FORCE_KERNEL_SHELL: bool = false;
const KEYBOARD_POLLING_FAILSAFE: bool = false;
const KERNEL_SHELL_DEBUG: bool = true;
const INIT_QUICK_FAIL_WINDOW_TICKS: u64 = 5_000;

static EMERGENCY_SHELL_SPAWNED: AtomicBool = AtomicBool::new(false);
static INIT_WATCH_PID: AtomicU32 = AtomicU32::new(0);
static INIT_WATCH_START_TICK: AtomicU64 = AtomicU64::new(0);

/// Kernel entry point, called from assembly stub `_start`.
///
/// # Safety
/// Called once by the bootloader with a valid BootInfo pointer.
/// Must not return.
#[no_mangle]
pub extern "C" fn kernel_main(boot_info: &'static BootInfo) -> ! {
    // Initialize serial output first — all diagnostics depend on it
    serial::init();

    // Initialize framebuffer console if available
    unsafe {
        fb_console::init(boot_info);
    }

    println!("RACORE: RacOS kernel starting (Build {})", env!("CARGO_PKG_VERSION"));
    
    // Validate boot info
    boot::validate(boot_info);

    serial::serial_println!("[  0.000010] RACORE: Boot info validated (magic OK, version {})", boot_info.version);

    // Report memory
    let usable_bytes = boot::count_usable_memory(boot_info);
    serial::serial_println!(
        "[  0.000020] RACORE: Memory detected: {} MiB usable",
        usable_bytes / (1024 * 1024)
    );

    // Initialize architecture-specific structures (GDT, IDT)
    arch::init();

    serial::serial_println!("[  0.000100] RACORE: Arch init complete (GDT, IDT)");

    // Initialize module loader
    mod_loader::init();

    // Initialize physical memory manager from boot memory map
    // SAFETY: Called once, boot_info memory map is valid from bootloader.
    unsafe {
        mm::phys::init_from_memory_map(
            boot_info.memory_map.entries,
            boot_info.memory_map.entry_count,
        );
    }

    // Reserve kernel memory region so allocator doesn't hand it out
    // SAFETY: kernel_physical_base is valid from bootloader.
    unsafe {
        // Reserve first 2 MiB (kernel + boot structures)
        mm::phys::reserve_range(0, 2 * 1024 * 1024);
        // Reserve framebuffer if present
        if boot_info.framebuffer.address != 0 {
            let fb_size = boot_info.framebuffer.pitch as u64 * boot_info.framebuffer.height as u64;
            mm::phys::reserve_range(boot_info.framebuffer.address, fb_size);
        }
    }

    serial::serial_println!(
        "[  0.000150] RACORE: Free frames after reservations: {}",
        mm::phys::free_count()
    );

    // Initialize kernel heap allocator
    // SAFETY: Physical allocator is initialized and reservations are done.
    unsafe {
        mm::heap::init().expect("Failed to initialize kernel heap");
        tty::vt::init();
    }

    // Initialize drivers (subsystem, block, PCI).
    drivers::init();
    // Phase F smoke test: verify AHCI persistence (write marker on first boot,
    // confirm it on later boots).
    drivers::ahci_self_test();
    // Phase E smoke test: verify the NIC TX path before the rest of init runs.
    drivers::nic_self_test();
    // Phase E krok 2/3: bring up the IPv4 stack and run the ARP→ICMP→DNS demo.
    net::stack::init();
    net::stack::start_demo("example.com");

    // Initialize shell filesystem API after block devices are ready.
    shell_fs::init(KERNEL_SHELL_DEBUG);

    // Initialize PIC + PIT (interrupts::init handles both). Used to call
    // pit::init() again here, which double-initialised the PIT and printed
    // duplicate "PIT initialized" lines.
    interrupts::init();

    // Input defaults: IRQ mode enabled, polling disabled unless fail-safe selected.
    drivers::ps2_keyboard::set_debug(KERNEL_SHELL_DEBUG);
    if KEYBOARD_POLLING_FAILSAFE {
        drivers::ps2_keyboard::set_input_mode(drivers::ps2_keyboard::InputMode::Polling);
    } else {
        drivers::ps2_keyboard::set_input_mode(drivers::ps2_keyboard::InputMode::Irq);
    }

    // Initialize scheduler with idle task (PID 0)
    // SAFETY: Called once, heap is ready, interrupts still disabled.
    unsafe { task::scheduler::init(); }

    // Initialize SYSCALL/SYSRET mechanism
    // SAFETY: Called once, GDT/TSS are initialized, interrupts disabled.
    unsafe { syscall::entry::init(); }

    // Snapshot the boot-time kernel CR3. All future user page tables inherit
    // their kernel mappings from this snapshot (see virt::create_user_page_table).
    // Must happen before any user process is constructed.
    unsafe { mm::virt::capture_kernel_cr3(); }

    // Initialize VFS
    unsafe { vfs::mount::init(); }

    // Set up and mount initramfs at root
    {
        // Use the binary initramfs from the bootloader if available; fall back to built-in.
        let initramfs = if boot_info.initramfs_base != 0 {
            serial::serial_println!(
                "[  0.000280] RACORE: Loading binary initramfs ({} bytes @ 0x{:X})",
                boot_info.initramfs_size,
                boot_info.initramfs_base
            );
            vfs::initramfs::Initramfs::from_binary(boot_info.initramfs_base, boot_info.initramfs_size)
                .unwrap_or_else(|| {
                    serial::serial_println!("[  0.000280] RACORE: Binary initramfs parse failed, using built-in");
                    let mut fs = vfs::initramfs::Initramfs::new();
                    let _sbin_ino = fs.add_dir("sbin");
                    let _etc_ino = fs.add_dir("etc");
                    fs
                })
        } else {
            serial::serial_println!("[  0.000280] RACORE: No initramfs from bootloader, using built-in");
            let mut fs = vfs::initramfs::Initramfs::new();
            let _sbin_ino = fs.add_dir("sbin");
            let _etc_ino = fs.add_dir("etc");
            fs
        };

        let initramfs_fs = vfs::initramfs::InitramfsFs::new(initramfs);

        unsafe {
            vfs::mount::mount_table().mount("/", initramfs_fs);
        }
    }

    // Set up and mount devfs at /dev
    {
        let mut devfs = vfs::devfs::Devfs::new();
        devfs.register_defaults();
        let devfs_fs = vfs::devfs::DevfsFilesystem::new(devfs);

        unsafe {
            vfs::mount::mount_table().mount("/dev", devfs_fs);
        }
    }

    // Set up and mount tmpfs at /tmp
    {
        let tmpfs = unsafe { vfs::tmpfs::init() };
        let tmpfs_fs = vfs::tmpfs::TmpfsFilesystem::new(tmpfs);
        unsafe {
            vfs::mount::mount_table().mount("/tmp", tmpfs_fs);
        }
    }

    // Set up and mount procfs at /proc
    {
        let procfs = vfs::procfs::Procfs::new();
        let procfs_fs = vfs::procfs::ProcFilesystem::new(procfs);
        unsafe {
            vfs::mount::mount_table().mount("/proc", procfs_fs);
        }
    }

    // Set up and mount racfs at /var (ephemeral, block-device-backed on ram0)
    {
        let racfs = unsafe { vfs::racfs::init() };
        let racfs_fs = vfs::racfs::RacfsFilesystem::new(racfs);
        unsafe {
            vfs::mount::mount_table().mount("/var", racfs_fs);
        }
    }

    // Phase F krok 3: mount racfs on the persistent SATA disk at /mnt.
    // Open existing FS if the superblock is valid; otherwise format it once.
    if let Some(sda) = drivers::block::find("sda") {
        match vfs::racfs::Racfs::open_or_format(sda) {
            Ok(racfs) => {
                // Run persistence test against the on-disk FS before handing
                // it off to the mount table.
                vfs::racfs::persistence_test(&racfs, "sda");
                let racfs_fs = vfs::racfs::RacfsFilesystem::new(racfs);
                unsafe { vfs::mount::mount_table().mount("/mnt", racfs_fs); }
                serial::serial_println!("[  0.000370] RACORE: racfs mounted on /mnt (persistent, on sda)");
            }
            Err(e) => serial::serial_println!("[  0.000370] RACORE: /mnt mount failed: {:?}", e),
        }
    }

    serial::serial_println!(
        "[  0.000300] RACORE: VFS ready (initramfs + devfs + tmpfs + procfs + racfs mounted), block-devices={}",
        drivers::block::count()
    );

    if RUN_KERNEL_SELF_TESTS {
        // Optional bring-up self-tests.
        unsafe {
            task::scheduler::spawn("test-task-a", test_task_a)
                .expect("Failed to spawn test-task-a");
            task::scheduler::spawn("test-task-b", test_task_b)
                .expect("Failed to spawn test-task-b");
            task::scheduler::spawn("test-racfs", test_racfs)
                .expect("Failed to spawn test-racfs");
            task::scheduler::spawn("test-security", test_security)
                .expect("Failed to spawn test-security");
            task::scheduler::spawn("test-net", test_net)
                .expect("Failed to spawn test-net");
        }
    }

    if FORCE_KERNEL_SHELL {
        spawn_kernel_shell_once();
    } else {
        match try_spawn_init() {
            Some(_init_pid) => {
                // Watchdog temporarily disabled during user-mode bring-up — it
                // races with the supervisor loop and was masking a scheduler
                // round-robin issue. Re-enable once spawn/wait paths settle.
                serial::serial_println!("[  0.000360] RACORE: init watchdog skipped (bring-up)");
            }
            None => {
                serial::serial_println!("[  0.000360] RACORE: init start failed, entering emergency shell");
                spawn_kernel_shell_once();
            }
        }
    }

    // Enable timer IRQ (IRQ0) and serial IRQ (IRQ4)
    interrupts::pic::enable_irq(0);
    interrupts::pic::enable_irq(4);

    // Enable interrupts
    serial::serial_println!("[  0.000400] RACORE: Enabling interrupts");
    unsafe { core::arch::asm!("sti", options(nomem, nostack)); }

    serial::serial_println!("[  0.000500] RACORE: Entering idle loop (scheduler active)");

    // Idle loop — halt until interrupt, repeat forever
    idle_loop()
}

/// Halts the CPU in a loop, waking only on interrupts. Polls the network
/// stack on each wakeup so RX frames are processed without IRQ-driven NIC.
fn idle_loop() -> ! {
    loop {
        net::stack::poll();
        arch::halt();
    }
}

/// Attempt to load and spawn a user-space init process from initramfs.
///
/// Tries `/sbin/init`, then `/bin/sh`, then `/bin/hello`. If none exists or loading fails,
/// logs a warning and continues (kernel runs with kernel-space test tasks only).
fn try_spawn_init() -> Option<u32> {
    // List of candidate paths to try
    const INIT_PATHS: &[&str] = &["/sbin/init", "/bin/sh", "/bin/hello", "/hello"];

    for path in INIT_PATHS {
        // Look up the file in VFS
        let mt = unsafe { vfs::mount::mount_table() };
        let data = match mt.lookup_path(path) {
            Ok((fs, ino)) => {
                let inode = match fs.get_inode(ino) {
                    Ok(i) => i,
                    Err(_) => continue,
                };
                // Read file contents
                let meta = match inode.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size = meta.size as usize;
                if size == 0 {
                    continue;
                }
                let mut buf = alloc::vec![0u8; size];
                match inode.read(0, &mut buf) {
                    Ok(n) => {
                        buf.truncate(n);
                        buf
                    }
                    Err(_) => continue,
                }
            }
            Err(_) => continue,
        };

        serial::serial_println!(
            "[  0.000350] RACORE: Found init binary '{}' ({} bytes), loading ELF...",
            path,
            data.len()
        );

        // Parse ELF
        let loaded = match elf::load_elf(&data) {
            Ok(l) => l,
            Err(e) => {
                serial::serial_println!("[  0.000350] RACORE: ELF load failed for '{}': {:?}", path, e);
                continue;
            }
        };

        // Create user process
        let mut process = match task::process::UserProcess::from_elf(path, &loaded, &[path.as_bytes()]) {
            Ok(p) => p,
            Err(e) => {
                serial::serial_println!("[  0.000350] RACORE: Process create failed for '{}': {}", path, e);
                continue;
            }
        };

        // Set up stdin/stdout/stderr (FDs 0, 1, 2) pointing to /dev/console
        {
            let mt = unsafe { vfs::mount::mount_table() };
            if let Ok((fs, ino)) = mt.lookup_path("/dev/console") {
                if let Ok(inode) = fs.get_inode(ino) {
                    use alloc::sync::Arc;
                    use vfs::file::OpenFile;
                    let stdin  = Arc::new(OpenFile::new(ino, inode.clone(), 0)); // O_RDONLY
                    let stdout = Arc::new(OpenFile::new(ino, inode.clone(), 1)); // O_WRONLY
                    let stderr = Arc::new(OpenFile::new(ino, inode, 1));         // O_WRONLY
                    let _ = process.task.fd_table.alloc(stdin);  // fd 0
                    let _ = process.task.fd_table.alloc(stdout); // fd 1
                    let _ = process.task.fd_table.alloc(stderr); // fd 2
                    serial::serial_println!("[  0.000355] RACORE: FDs 0/1/2 → /dev/console");
                }
            }
        }

        // Spawn it
        match unsafe { task::scheduler::spawn_user(process) } {
            Ok(pid) => {
                serial::serial_println!(
                    "[  0.000350] RACORE: Spawned user process '{}' with PID {}",
                    path,
                    pid
                );
                return Some(pid);
            }
            Err(e) => {
                serial::serial_println!("[  0.000350] RACORE: spawn_user failed for '{}': {}", path, e);
            }
        }
    }

    serial::serial_println!("[  0.000350] RACORE: No init binary found in initramfs");
    None
}

fn arm_init_watchdog(init_pid: u32) {
    INIT_WATCH_PID.store(init_pid, Ordering::Relaxed);
    INIT_WATCH_START_TICK.store(interrupts::pit::ticks(), Ordering::Relaxed);

    match unsafe { task::scheduler::spawn("init-watchdog", init_watchdog_task) } {
        Ok(pid) => serial::serial_println!(
            "[  0.000360] RACORE: init watchdog armed (PID {}, watches init PID {})",
            pid,
            init_pid,
        ),
        Err(e) => {
            serial::serial_println!(
                "[  0.000360] RACORE: init watchdog spawn failed: {}, entering emergency shell",
                e,
            );
            spawn_kernel_shell_once();
        }
    }
}

fn init_watchdog_task() -> ! {
    loop {
        let init_pid = INIT_WATCH_PID.load(Ordering::Relaxed);
        let start_tick = INIT_WATCH_START_TICK.load(Ordering::Relaxed);
        let now = interrupts::pit::ticks();

        if init_pid != 0 {
            let init_state = unsafe {
                core::arch::asm!("cli", options(nomem, nostack));
                let state = task::scheduler::with_task_by_pid(init_pid, |t| t.state);
                core::arch::asm!("sti", options(nomem, nostack));
                state
            };

            let quick_window = now.saturating_sub(start_tick) <= INIT_QUICK_FAIL_WINDOW_TICKS;
            match init_state {
                Some(crate::task::task::TaskState::Zombie) | None if quick_window => {
                    serial::serial_println!(
                        "[  WATCH  ] init crashed early (PID {}, +{} ticks), enabling emergency shell",
                        init_pid,
                        now.saturating_sub(start_tick),
                    );
                    spawn_kernel_shell_once();
                    loop { task::scheduler::yield_now(); }
                }
                _ if now.saturating_sub(start_tick) > INIT_QUICK_FAIL_WINDOW_TICKS => {
                    serial::serial_println!(
                        "[  WATCH  ] init PID {} survived quick-fail window ({} ticks)",
                        init_pid,
                        INIT_QUICK_FAIL_WINDOW_TICKS,
                    );
                    loop { task::scheduler::yield_now(); }
                }
                _ => {}
            }
        }

        task::scheduler::yield_now();
    }
}

fn spawn_kernel_shell_once() {
    if EMERGENCY_SHELL_SPAWNED.swap(true, Ordering::SeqCst) {
        return;
    }

    unsafe {
        task::scheduler::spawn("kernel-shell", kernel_shell_task)
            .expect("Failed to spawn kernel-shell");
    }
    serial::serial_println!("[  0.000360] RACORE: Emergency kernel shell enabled");
}

/// Test kernel task A — prints periodically.
fn test_task_a() -> ! {
    let mut counter = 0u64;
    loop {
        if counter % 1000 == 0 {
            serial::serial_println!(
                "[  TASK-A ] tick {} (PID {})",
                counter,
                task::scheduler::current_pid()
            );
        }
        counter += 1;
        if counter % 100 == 0 {
            task::scheduler::yield_now();
        }
    }
}

/// Test kernel task B — prints periodically.
fn test_task_b() -> ! {
    let mut counter = 0u64;
    loop {
        if counter % 1000 == 0 {
            serial::serial_println!(
                "[  TASK-B ] tick {} (PID {})",
                counter,
                task::scheduler::current_pid()
            );
        }
        counter += 1;
        if counter % 100 == 0 {
            task::scheduler::yield_now();
        }
    }
}

fn shell_write(s: &str) {
    serial::serial_print!("{}", s);
    tty::vt::vt_print(s);
}

fn shell_writeln(s: &str) {
    shell_write(s);
    shell_write("\n");
}

type CommandHandler = fn(&[&str]);

struct ShellCommand {
    name: &'static str,
    help: &'static str,
    handler: CommandHandler,
}

fn cmd_help(_args: &[&str]) {
    for cmd in SHELL_COMMANDS {
        shell_writeln(cmd.help);
    }
}

fn cmd_clear(_args: &[&str]) {
    tty::vt::vt_clear_current();
}

fn cmd_echo(args: &[&str]) {
    if args.is_empty() {
        shell_writeln("");
        return;
    }

    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            shell_write(" ");
        }
        shell_write(arg);
    }
    shell_write("\n");
}

fn cmd_version(_args: &[&str]) {
    shell_writeln(concat!("RacOS kernel ", env!("CARGO_PKG_VERSION")));
}

fn cmd_pwd(_args: &[&str]) {
    match shell_fs::pwd() {
        Ok(path) => shell_writeln(&path),
        Err(e) => {
            shell_write("pwd: ");
            shell_writeln(e);
        }
    }
}

fn cmd_cd(args: &[&str]) {
    let path = if args.is_empty() { "/" } else { args[0] };
    match shell_fs::chdir(path) {
        Ok(()) => {}
        Err(e) => {
            shell_write("cd: ");
            shell_writeln(e);
        }
    }
}

fn cmd_ls(args: &[&str]) {
    let path = if args.is_empty() { None } else { Some(args[0]) };
    match shell_fs::ls(path) {
        Ok(entries) => {
            if entries.is_empty() {
                shell_writeln("(empty)");
                return;
            }
            for entry in entries {
                if entry.is_dir {
                    shell_writeln(&alloc::format!("[dir] {}", entry.name));
                } else {
                    shell_writeln(&entry.name);
                }
            }
        }
        Err(e) => {
            shell_write("ls: ");
            shell_writeln(e);
        }
    }
}

fn cmd_mkdir(args: &[&str]) {
    if args.is_empty() {
        shell_writeln("mkdir: missing path");
        return;
    }
    match shell_fs::mkdir(args[0]) {
        Ok(()) => {}
        Err(e) => {
            shell_write("mkdir: ");
            shell_writeln(e);
        }
    }
}

fn cmd_touch(args: &[&str]) {
    if args.is_empty() {
        shell_writeln("touch: missing path");
        return;
    }
    match shell_fs::touch(args[0]) {
        Ok(()) => {}
        Err(e) => {
            shell_write("touch: ");
            shell_writeln(e);
        }
    }
}

fn cmd_cat(args: &[&str]) {
    if args.is_empty() {
        shell_writeln("cat: missing path");
        return;
    }

    match shell_fs::read_file(args[0]) {
        Ok(data) => {
            if data.is_empty() {
                return;
            }

            match core::str::from_utf8(&data) {
                Ok(text) => {
                    shell_write(text);
                    if !text.ends_with('\n') {
                        shell_write("\n");
                    }
                }
                Err(_) => {
                    shell_writeln("cat: binary or non-UTF8 data");
                }
            }
        }
        Err(e) => {
            shell_write("cat: ");
            shell_writeln(e);
        }
    }
}

const SHELL_COMMANDS: &[ShellCommand] = &[
    ShellCommand {
        name: "help",
        help: "help    - list available commands",
        handler: cmd_help,
    },
    ShellCommand {
        name: "clear",
        help: "clear   - clear terminal framebuffer",
        handler: cmd_clear,
    },
    ShellCommand {
        name: "echo",
        help: "echo    - print arguments",
        handler: cmd_echo,
    },
    ShellCommand {
        name: "version",
        help: "version - print OS version",
        handler: cmd_version,
    },
    ShellCommand {
        name: "pwd",
        help: "pwd     - print current directory",
        handler: cmd_pwd,
    },
    ShellCommand {
        name: "cd",
        help: "cd [p]  - change current directory",
        handler: cmd_cd,
    },
    ShellCommand {
        name: "ls",
        help: "ls [p]  - list directory contents",
        handler: cmd_ls,
    },
    ShellCommand {
        name: "mkdir",
        help: "mkdir p - create directory",
        handler: cmd_mkdir,
    },
    ShellCommand {
        name: "touch",
        help: "touch p - create file",
        handler: cmd_touch,
    },
    ShellCommand {
        name: "cat",
        help: "cat p   - print file contents",
        handler: cmd_cat,
    },
];

fn parse_and_dispatch(line: &str) {
    if KERNEL_SHELL_DEBUG {
        serial::serial_println!("[ SHELL ] raw='{}'", line);
    }

    let mut tokens: [&str; 16] = [""; 16];
    let mut count = 0usize;
    for tok in line.split_whitespace() {
        if count >= tokens.len() {
            break;
        }
        tokens[count] = tok;
        count += 1;
    }

    if count == 0 {
        return;
    }

    let command = tokens[0];
    let args = &tokens[1..count];

    if KERNEL_SHELL_DEBUG {
        serial::serial_println!(
            "[ SHELL ] command='{}' argc={}",
            command,
            args.len()
        );
    }

    for cmd in SHELL_COMMANDS {
        if cmd.name == command {
            (cmd.handler)(args);
            return;
        }
    }

    shell_writeln("unknown command");
}

/// Minimal in-kernel emergency shell for bring-up.
fn kernel_shell_task() -> ! {
    shell_writeln("RacOS emergency shell (kernel mode)");
    shell_writeln("Type 'help' for commands.");
    shell_write("racos> ");

    let mut buf = [0u8; 256];
    let mut len = 0usize;
    buf[0] = 0;

    loop {
        if drivers::ps2_keyboard::input_mode() == drivers::ps2_keyboard::InputMode::Polling {
            unsafe {
                drivers::ps2_keyboard::poll_input();
            }
        }

        if let Some(b) = serial::read_byte_nonblocking() {
            match b {
                b'\r' | b'\n' => {
                    shell_write("\n");
                    let cmd = core::str::from_utf8(&buf[..len]).unwrap_or("").trim();
                    if KERNEL_SHELL_DEBUG {
                        serial::serial_println!("[ SHELL ] buffer_len={}", len);
                    }
                    parse_and_dispatch(cmd);

                    len = 0;
                    buf[0] = 0;
                    shell_write("racos> ");
                }
                8 => {
                    if len > 0 {
                        len -= 1;
                        buf[len] = 0;
                        shell_write("\x08 \x08");
                    }
                }
                c if (c as char).is_ascii_graphic() || c == b' ' => {
                    if len + 1 < buf.len() {
                        buf[len] = c;
                        len += 1;
                        buf[len] = 0;
                        let s = [c];
                        if let Ok(st) = core::str::from_utf8(&s) {
                            shell_write(st);
                        }
                    }
                }
                _ => {}
            }
        } else {
            task::scheduler::yield_now();
        }
    }
}

/// Kernel self-test: exercise racfs mount/create/read/write/unlink under /var.
fn test_racfs() -> ! {
    serial::serial_println!("[TEST-RACFS] Starting racfs block-device test");

    // 1. mkdir /var/test
    let racfs = unsafe { vfs::racfs::instance() };
    match racfs.create_dir(0, "test") {
        Ok(dir_ino) => serial::serial_println!("[TEST-RACFS] mkdir /var/test => ino {}", dir_ino),
        Err(e) => serial::serial_println!("[TEST-RACFS] FAIL mkdir: {:?}", e),
    }

    // 2. create /var/test/hello.txt
    let test_dir_ino = match racfs.lookup_path("test") {
        Ok(ino) => ino,
        Err(e) => {
            serial::serial_println!("[TEST-RACFS] FAIL lookup test dir: {:?}", e);
            loop { task::scheduler::yield_now(); }
        }
    };

    let file_ino = match racfs.create_file(test_dir_ino, "hello.txt") {
        Ok(ino) => {
            serial::serial_println!("[TEST-RACFS] create /var/test/hello.txt => ino {}", ino);
            ino
        }
        Err(e) => {
            serial::serial_println!("[TEST-RACFS] FAIL create file: {:?}", e);
            loop { task::scheduler::yield_now(); }
        }
    };

    // 3. write data
    let data = b"Hello from racfs on ramdisk!";
    match racfs.write_file(file_ino, 0, data) {
        Ok(n) => serial::serial_println!("[TEST-RACFS] write {} bytes OK", n),
        Err(e) => serial::serial_println!("[TEST-RACFS] FAIL write: {:?}", e),
    }

    // 4. read back and verify
    let mut buf = [0u8; 64];
    match racfs.read_file(file_ino, 0, &mut buf) {
        Ok(n) => {
            let read_str = core::str::from_utf8(&buf[..n]).unwrap_or("<invalid utf8>");
            if &buf[..n] == data {
                serial::serial_println!("[TEST-RACFS] read-back PASS: '{}'", read_str);
            } else {
                serial::serial_println!("[TEST-RACFS] read-back FAIL: got '{}'", read_str);
            }
        }
        Err(e) => serial::serial_println!("[TEST-RACFS] FAIL read: {:?}", e),
    }

    // 5. readdir /var/test
    match racfs.readdir(test_dir_ino) {
        Ok(entries) => {
            serial::serial_println!("[TEST-RACFS] readdir /var/test: {} entries", entries.len());
            for e in &entries {
                serial::serial_println!("[TEST-RACFS]   - {} (ino {})", e.name, e.ino);
            }
        }
        Err(e) => serial::serial_println!("[TEST-RACFS] FAIL readdir: {:?}", e),
    }

    // 6. unlink /var/test/hello.txt
    match racfs.unlink(test_dir_ino, "hello.txt") {
        Ok(()) => serial::serial_println!("[TEST-RACFS] unlink /var/test/hello.txt OK"),
        Err(e) => serial::serial_println!("[TEST-RACFS] FAIL unlink: {:?}", e),
    }

    // 7. verify deletion
    match racfs.readdir(test_dir_ino) {
        Ok(entries) => {
            if entries.is_empty() {
                serial::serial_println!("[TEST-RACFS] post-unlink readdir PASS: 0 entries");
            } else {
                serial::serial_println!("[TEST-RACFS] post-unlink readdir FAIL: {} entries", entries.len());
            }
        }
        Err(e) => serial::serial_println!("[TEST-RACFS] FAIL post-unlink readdir: {:?}", e),
    }

    // 8. Block device stats
    {
        let dev_count = drivers::block::count();
        serial::serial_println!("[TEST-RACFS] block devices registered: {}", dev_count);
    }

    serial::serial_println!("[TEST-RACFS] All tests complete");

    // Park this task.
    loop {
        task::scheduler::yield_now();
    }
}

/// Kernel self-test: capability + DAC checks (Phase C2/C3/C6).
fn test_security() -> ! {
    serial::serial_println!("[TEST-SEC ] Starting security self-test");

    // C3: DAC checks with a synthetic inode metadata object.
    let mut meta = vfs::inode::InodeMetadata::new(1, vfs::inode::FileType::Regular);
    meta.uid = 0;
    meta.gid = 0;
    meta.mode = vfs::inode::FileMode::new(0o644);

    let user_creds = task::task::Credentials {
        uid: 1000,
        gid: 1000,
        euid: 1000,
        egid: 1000,
        cap_permitted: 0,
        cap_effective: 0,
        cap_inheritable: 0,
    };

    let dac_read = security::dac::can_access(&user_creds, &meta, security::dac::Access::Read);
    let dac_write = security::dac::can_access(&user_creds, &meta, security::dac::Access::Write);
    if dac_read && !dac_write {
        serial::serial_println!("[TEST-SEC ] C3 DAC PASS (0644 blocks non-owner writes)");
    } else {
        serial::serial_println!("[TEST-SEC ] C3 DAC FAIL (read={}, write={})", dac_read, dac_write);
    }

    let mut cap_creds = user_creds;
    cap_creds.cap_effective = security::capability::cap_mask(security::capability::CAP_DAC_OVERRIDE);
    let override_write = security::dac::can_access(&cap_creds, &meta, security::dac::Access::Write);
    if override_write {
        serial::serial_println!("[TEST-SEC ] C2 CAP_DAC_OVERRIDE PASS");
    } else {
        serial::serial_println!("[TEST-SEC ] C2 CAP_DAC_OVERRIDE FAIL");
    }

    // C2/C4 integration: CAP_SETUID gates setuid behavior.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let _ = task::scheduler::with_current_task_mut(|t| {
            t.creds.uid = 1000;
            t.creds.euid = 1000;
            t.creds.gid = 1000;
            t.creds.egid = 1000;
            t.creds.cap_permitted = 0;
            t.creds.cap_effective = 0;
            t.creds.cap_inheritable = 0;
        });
        core::arch::asm!("sti", options(nomem, nostack));
    }

    let denied = syscall::handlers::sys_setuid(0);
    if denied == Err(syscall::error::SyscallError::EPERM) {
        serial::serial_println!("[TEST-SEC ] C2 CAP_SETUID gate PASS (denied without cap)");
    } else {
        serial::serial_println!("[TEST-SEC ] C2 CAP_SETUID gate FAIL ({:?})", denied);
    }

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let _ = task::scheduler::with_current_task_mut(|t| {
            t.creds.cap_effective = security::capability::cap_mask(security::capability::CAP_SETUID);
        });
        core::arch::asm!("sti", options(nomem, nostack));
    }

    let allowed = syscall::handlers::sys_setuid(0);
    if allowed == Ok(0) {
        serial::serial_println!("[TEST-SEC ] C2 CAP_SETUID gate PASS (allowed with cap)");
    } else {
        serial::serial_println!("[TEST-SEC ] C2 CAP_SETUID gate FAIL ({:?})", allowed);
    }

    // Restore root credentials for this long-lived kernel task.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        let _ = task::scheduler::with_current_task_mut(|t| {
            t.creds = task::task::Credentials::root();
        });
        core::arch::asm!("sti", options(nomem, nostack));
    }

    serial::serial_println!("[TEST-SEC ] Security self-test complete");
    loop {
        task::scheduler::yield_now();
    }
}

/// Kernel self-test: loopback networking MVP.
fn test_net() -> ! {
    serial::serial_println!("[TEST-NET ] Starting networking test");

    // PCI Enumeration check
    let pci_devs = drivers::pci::enumerate_pci();
    serial::serial_println!("[TEST-NET ] Found {} PCI devices", pci_devs.len());

    let server_pid = task::scheduler::current_pid();
    let server_sid = match net::create_socket(net::AF_INET, net::SOCK_STREAM, 0) {
        Ok(s) => s,
        Err(e) => {
            serial::serial_println!("[TEST-NET ] FAIL socket(server): {:?}", e);
            loop { task::scheduler::yield_now(); }
        }
    };
    net::bind_fd(server_pid, 100, server_sid);
    let _ = net::bind(server_pid, 100, 18080);
    let _ = net::listen(server_pid, 100, 8);

    let client_sid = match net::create_socket(net::AF_INET, net::SOCK_STREAM, 0) {
        Ok(s) => s,
        Err(e) => {
            serial::serial_println!("[TEST-NET ] FAIL socket(client): {:?}", e);
            loop { task::scheduler::yield_now(); }
        }
    };
    net::bind_fd(server_pid, 101, client_sid);

    match net::connect(server_pid, 101, 18080) {
        Ok(()) => serial::serial_println!("[TEST-NET ] connect PASS"),
        Err(e) => {
            serial::serial_println!("[TEST-NET ] connect FAIL: {:?}", e);
            loop { task::scheduler::yield_now(); }
        }
    }

    let accepted_sid = match net::accept(server_pid, 100) {
        Ok(s) => s,
        Err(e) => {
            serial::serial_println!("[TEST-NET ] accept FAIL: {:?}", e);
            loop { task::scheduler::yield_now(); }
        }
    };
    net::bind_fd(server_pid, 102, accepted_sid);

    let payload = b"net-loopback-ok";
    match net::send(server_pid, 101, payload) {
        Ok(n) if n == payload.len() => {
            serial::serial_println!("[TEST-NET ] send PASS ({} bytes)", n)
        }
        Ok(n) => serial::serial_println!("[TEST-NET ] send PARTIAL ({})", n),
        Err(e) => serial::serial_println!("[TEST-NET ] send FAIL: {:?}", e),
    }

    let mut rx = [0u8; 64];
    match net::recv(server_pid, 102, &mut rx) {
        Ok(n) => {
            if &rx[..n] == payload {
                serial::serial_println!("[TEST-NET ] recv PASS '{}')", core::str::from_utf8(&rx[..n]).unwrap_or("?"));
            } else {
                serial::serial_println!("[TEST-NET ] recv FAIL content mismatch");
            }
        }
        Err(e) => serial::serial_println!("[TEST-NET ] recv FAIL: {:?}", e),
    }

    let _ = net::shutdown(server_pid, 101, 2);
    let _ = net::shutdown(server_pid, 102, 2);
    net::close_fd(server_pid, 100);
    net::close_fd(server_pid, 101);
    net::close_fd(server_pid, 102);

    serial::serial_println!("[TEST-NET ] Loopback socket test complete");
    loop {
        task::scheduler::yield_now();
    }
}
