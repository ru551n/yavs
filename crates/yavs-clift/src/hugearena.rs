//! Machine code in one huge-page arena (`yavs_rt::hugezone`) instead of the default provider's
//! 4 KiB pages, which cost a large process an instruction-TLB entry per page. Linux x86_64 only;
//! everywhere else, where the host refuses the reservation, and with `YAVS_CLIFT_HUGE=0`, the
//! default provider is used. A kernel without huge pages leaves the arena on small pages.

use cranelift_jit::{BranchProtection, JITMemoryKind, JITMemoryProvider, SystemMemoryProvider};
use cranelift_module::ModuleResult;
use std::sync::{Mutex, OnceLock, PoisonError};
use yavs_rt::hugezone::{self, Zones};

/// One reservation per process, whatever the number of JITs.
static ZONES: OnceLock<Option<Mutex<Zones>>> = OnceLock::new();

struct Provider {
    zones: &'static Mutex<Zones>,
    /// For what does not fit the arena; a call out of range goes through Cranelift's veneer.
    overflow: SystemMemoryProvider,
}

/// The provider of a new JIT, or `None` to keep the default one.
pub fn provider() -> Option<Box<dyn JITMemoryProvider + Send>> {
    let zones = ZONES
        .get_or_init(|| {
            if std::env::var("YAVS_CLIFT_HUGE").is_ok_and(|v| v == "0") {
                return None;
            }
            hugezone::reserve().map(Mutex::new)
        })
        .as_ref()?;
    Some(Box::new(Provider {
        zones,
        overflow: SystemMemoryProvider::new(),
    }))
}

impl JITMemoryProvider for Provider {
    fn allocate(
        &mut self,
        size: usize,
        align: u64,
        kind: JITMemoryKind,
    ) -> std::io::Result<*mut u8> {
        let mut z = self.zones.lock().unwrap_or_else(PoisonError::into_inner);
        let got = match kind {
            JITMemoryKind::Executable => z.code.alloc(size, align as usize, true),
            JITMemoryKind::Writable | JITMemoryKind::ReadOnly => {
                z.data.alloc(size, align as usize, false)
            }
        };
        drop(z);
        match got {
            Some(p) => Ok(p as *mut u8),
            None => self.overflow.allocate(size, align, kind),
        }
    }

    // ponytail: the arena is never given back (the default provider leaks too, so function
    // pointers stay valid); a full arena goes to `overflow`.
    unsafe fn free_memory(&mut self) {}

    /// The code zone is writable and executable at once: protecting part of a huge page would
    /// split it. x86 keeps its instruction cache coherent.
    fn finalize(&mut self, branch_protection: BranchProtection) -> ModuleResult<()> {
        self.overflow.finalize(branch_protection)
    }
}
