#![no_std]
#![no_main]

extern crate alloc;

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc < 2 {
        unsafe { libc_lite::println("Usage: rapt install <pkg> [...]") };
        return 1;
    }

    // Parse argv[1] as command
    let cmd_ptr = unsafe { *argv.offset(1) };
    if cmd_ptr.is_null() {
        return 1;
    }
    let cmd = unsafe { cstr_to_str(cmd_ptr) };
    let cmd = match cmd {
        Some(s) => s,
        None => return 1,
    };

    match cmd {
        "install" => {
            if argc < 3 {
                unsafe { libc_lite::println("Usage: rapt install <pkg> [...]") };
                return 1;
            }
            let mut packages = alloc::vec::Vec::new();
            for i in 2..argc as isize {
                let pkg_ptr = unsafe { *argv.offset(i) };
                if pkg_ptr.is_null() {
                    continue;
                }
                let pkg = unsafe { cstr_to_str(pkg_ptr) };
                if let Some(pkg) = pkg {
                    packages.push(pkg);
                }
            }
            cmd_install(&packages)
        }
        _ => {
            unsafe { libc_lite::println("Unknown command") };
            1
        }
    }
}

unsafe fn cstr_to_str(ptr: *const u8) -> Option<&'static str> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    while *ptr.offset(len) != 0 {
        len += 1;
        if len > 1024 { // safety limit
            return None;
        }
    }
    let slice = core::slice::from_raw_parts(ptr, len);
    core::str::from_utf8(slice).ok()
}

fn cmd_install(packages: &[&str]) -> i32 {
    // Mock repository index for demo
    let index = [
        rapt::RepoPackage {
            name: "libc-lite".into(),
            version: "0.1.0".into(),
            arch: "x86_64".into(),
            filename: "libc-lite-0.1.0-x86_64.rpk".into(),
            depends: alloc::vec![],
        },
        rapt::RepoPackage {
            name: "demo".into(),
            version: "1.0.0".into(),
            arch: "x86_64".into(),
            filename: "demo-1.0.0-x86_64.rpk".into(),
            depends: alloc::vec!["libc-lite >= 0.1.0".into()],
        },
        rapt::RepoPackage {
            name: "tool".into(),
            version: "2.0.0".into(),
            arch: "x86_64".into(),
            filename: "tool-2.0.0-x86_64.rpk".into(),
            depends: alloc::vec!["demo".into()],
        },
    ];

    match rapt::plan_install(&index, packages) {
        Ok(plan) => {
            unsafe { libc_lite::println("Install plan:") };
            for step in plan {
                unsafe { libc_lite::print("  ") };
                unsafe { libc_lite::print(&step.name) };
                unsafe { libc_lite::print(" ") };
                unsafe { libc_lite::print(&step.version) };
                unsafe { libc_lite::print(" (") };
                unsafe { libc_lite::print(&step.filename) };
                unsafe { libc_lite::println(")") };
            }
            0
        }
        Err(rapt::Error::MissingPackage(name)) => {
            unsafe { libc_lite::print("Missing package: ") };
            unsafe { libc_lite::println(&name) };
            1
        }
        Err(rapt::Error::CyclicDependency) => {
            unsafe { libc_lite::println("Cyclic dependency detected") };
            1
        }
        Err(_) => {
            unsafe { libc_lite::println("Install planning failed") };
            1
        }
    }
}

extern crate libc_lite;
extern crate rapt;