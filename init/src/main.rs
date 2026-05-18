#![no_std]
#![no_main]

extern crate alloc;
use libc_lite::*;

#[no_mangle]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // 1. Inicjalizacja stosu i podstawowych struktur (PID 1)
    let msg = "Init process v0.1.0 starting...\n";
    let _ = write(1, msg.as_bytes());

    // 2. Utworzenie gniazda Unix do komunikacji z Kernelem
    if let Ok(_sock) = socket(1, 1, 0) { // AF_UNIX, SOCK_STREAM
        // Związanie gniazda z adresem kontrolnym
        // Uwaga: Nowa sygnatura bind oczekuje SockAddrIn (stub w libc-lite)
        // Dla uproszczenia testu pominę bind lub użyję surowego syscalla jeśli trzeba
        let _ = write(1, b"Control socket created successfully\n");
    }

    // 3. Sprawdzenie mechanizmu POSIX Thread (spawn_thread)
    let thread_id = pthread_create(worker_thread as u64, 0x1337);
    if thread_id > 0 {
        let _ = write(1, b"Spawned worker_thread successfuly.\n");
    } else {
        let _ = write(1, b"Failed to spawn worker_thread.\n");
    }

    // 4. Główna pętla inita (Wait for messages / services)
    loop {
        for _ in 0..1000000 { unsafe { core::arch::asm!("nop"); } }
    }
}

fn worker_thread(_arg: u64) -> ! {
    let msg = b"Hello from User-Space Worker Thread!\n";
    loop {
        let _ = write(1, msg);
        for _ in 0..5000000 { unsafe { core::arch::asm!("nop"); } }
    }
}

/* DELETED duplicate panic handler - now provided by libc-lite */
