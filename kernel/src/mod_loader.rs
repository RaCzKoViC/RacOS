// RaCore — Dynamic Kernel Modules
//
// Placeholder for dynamic extension loading (ADR-002: Modular Monolithic).

use alloc::string::String;
use alloc::vec::Vec;

pub struct KernelModule {
    pub name: String,
    pub version: String,
    // Symbol table etc.
}

pub struct ModuleManager {
    loaded_modules: Vec<KernelModule>,
}

impl ModuleManager {
    pub fn new() -> Self {
        ModuleManager {
            loaded_modules: Vec::new(),
        }
    }

    pub fn load_module(&mut self, name: &str) -> Result<(), &str> {
        crate::serial::serial_println!("[ MOD  ] Loading module: {}", name);
        // TODO: Actually parse ELF and resolve symbols
        Ok(())
    }
}

static mut MODULE_MANAGER: Option<ModuleManager> = None;

pub fn init() {
    unsafe {
        MODULE_MANAGER = Some(ModuleManager::new());
    }
}
