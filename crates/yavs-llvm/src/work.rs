//! The work clock of `--tier=auto` (#670, #673): user-mode instructions retired by the
//! simulation threads, read from one `perf_event_open` counter per thread. A loaded host, SMT
//! neighbours and the clock speed change how long the same work takes, not how many instructions
//! it is, so the same design and binary reach the same decisions on a busy host as on an idle one.
//!
//! Every simulation thread notes its thread id when the interpreter first gives it a sampling slot
//! (`yavs_interp::on_simulation_thread`; one system call that costs nothing a run of a few
//! milliseconds notices). The counters are opened by the sampler thread, which a run starts after
//! `START_AFTER` (`activate`: `perf_event_open` takes the thread id, so one thread can open them all),
//! and it reads them one by one and charges each thread's instructions to the function that thread
//! runs, so a worker that spins or waits (no function in its slot) adds nothing. Where the kernel
//! refuses a counter (or there is none: not Linux, a virtual machine without a PMU,
//! `perf_event_paranoid` 3) the sampler falls back to CPU time (`activate` says why).

/// Instructions that count as one microsecond of work: the policy's constants (compile prices,
/// thresholds) are microseconds, and this is the rate the interpreter retires user instructions at
/// per CPU microsecond on the development host (about 3 instructions a cycle at 5 GHz; the median of the designs measured).
pub(crate) const INSTR_PER_US: u64 = 16_000;

/// The microseconds of work that `instr` instructions are.
pub(crate) fn work_us(instr: u64) -> u64 {
    instr / per_us()
}

/// `INSTR_PER_US` (`YAVS_AUTO_IPUS` overrides it, a tuning aid like `YAVS_LLVM_FAST_MIN`).
pub(crate) fn per_us() -> u64 {
    static PER: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PER.get_or_init(|| {
        std::env::var("YAVS_AUTO_IPUS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&v| v > 0)
            .unwrap_or(INSTR_PER_US)
    })
}

/// Have the threads that get a sampling slot noted (cheap: no counter is opened yet).
pub(crate) fn install() {
    imp::install();
}

/// Open the counters of the threads noted so far, and of every thread noted from now on: the clock of
/// the sampler is the counters from here on. `Err` says why not (the CPU time is the clock then).
pub(crate) fn activate() -> Result<(), String> {
    imp::activate()
}

/// Appends, per registered simulation thread, its slot index (`yavs_interp::Runtime::thread_sampler`)
/// and the instructions it retired so far.
pub(crate) fn per_thread(out: &mut Vec<(usize, u64)>) {
    imp::per_thread(out, None);
}

/// `per_thread` for the threads with these slot indices only: a read is a system call, and the
/// sampler shares the core with the simulation it counts.
pub(crate) fn of_threads(slots: &[usize], out: &mut Vec<(usize, u64)>) {
    imp::per_thread(out, Some(slots));
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod imp {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[cfg(target_arch = "x86_64")]
    const SYS_PERF_EVENT_OPEN: i64 = 298;
    #[cfg(target_arch = "x86_64")]
    const SYS_GETTID: i64 = 186;
    #[cfg(target_arch = "aarch64")]
    const SYS_PERF_EVENT_OPEN: i64 = 241;
    #[cfg(target_arch = "aarch64")]
    const SYS_GETTID: i64 = 178;

    unsafe extern "C" {
        fn syscall(num: i64, ...) -> i64;
        fn read(fd: i32, buf: *mut std::ffi::c_void, n: usize) -> isize;
        fn close(fd: i32) -> i32;
    }

    /// The simulation threads by slot index with their thread ids, and once `activate`d their counters
    /// (never closed while the process runs: a thread that ended keeps its count).
    struct State {
        active: bool,
        tids: Vec<(usize, i32)>,
        fds: Vec<(usize, i32)>,
    }

    static STATE: Mutex<State> = Mutex::new(State {
        active: false,
        tids: Vec::new(),
        fds: Vec::new(),
    });
    static REFUSED: AtomicBool = AtomicBool::new(false);

    fn state() -> std::sync::MutexGuard<'static, State> {
        STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A counter of the user-mode instructions of thread `tid` (`PERF_COUNT_HW_INSTRUCTIONS`, kernel
    /// and hypervisor excluded, enabled at once), reading `value, time enabled, time running`.
    fn open(tid: i32) -> Result<i32, std::io::Error> {
        // `struct perf_event_attr` (128 bytes) as 64-bit words: type and size, config, sample_period,
        // sample_type, read_format, flags, then fields we leave zero.
        let mut attr = [0u64; 16];
        attr[0] = 128 << 32; // type PERF_TYPE_HARDWARE (0), size
        attr[1] = 1; // PERF_COUNT_HW_INSTRUCTIONS
        attr[4] = 1 | 2; // read_format: TOTAL_TIME_ENABLED | TOTAL_TIME_RUNNING
        attr[5] = (1 << 5) | (1 << 6); // flags: exclude_kernel, exclude_hv (not disabled, not inherited)
        const PERF_FLAG_FD_CLOEXEC: u64 = 8;
        // SAFETY: `attr` is a `perf_event_attr` of the size it names; `tid` is a thread of this
        // process, cpu -1 is every CPU.
        let fd = unsafe {
            syscall(
                SYS_PERF_EVENT_OPEN,
                attr.as_ptr(),
                tid,
                -1i32,
                -1i32,
                PERF_FLAG_FD_CLOEXEC,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(fd as i32)
        }
    }

    pub(super) fn read_one(fd: i32) -> u64 {
        let mut v = [0u64; 3];
        // SAFETY: `v` holds the 24 bytes of the read format.
        let n = unsafe { read(fd, v.as_mut_ptr().cast(), 24) };
        if n != 24 {
            return 0;
        }
        // The counter shared the PMU with others: scale what it saw to the time it was enabled.
        if v[2] > 0 && v[2] < v[1] {
            (v[0] as f64 * v[1] as f64 / v[2] as f64) as u64
        } else {
            v[0]
        }
    }

    fn gettid() -> i32 {
        // SAFETY: gettid has no arguments and cannot fail.
        unsafe { syscall(SYS_GETTID) as i32 }
    }

    /// The hook of `yavs_interp::on_simulation_thread`: note the calling thread (slot `index`), and
    /// open its counter at once when the counters are on.
    fn note_thread(index: usize) {
        let tid = gettid();
        let mut st = state();
        st.tids.push((index, tid));
        if st.active {
            match open(tid) {
                Ok(fd) => st.fds.push((index, fd)),
                Err(e) => {
                    drop(st);
                    refused(&e);
                }
            }
        }
    }

    fn refused(e: &std::io::Error) {
        if !REFUSED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "warning: --tier=auto cannot count the instructions of a simulation thread ({e}): its work is not seen by the tier-up policy"
            );
        }
    }

    pub fn install() {
        yavs_interp::on_simulation_thread(note_thread);
    }

    pub fn activate() -> Result<(), String> {
        // Probe with the caller's own counter: no thread may be noted yet (the main thread is told
        // when it first interprets), and a refused kernel must not look like success (#743).
        match open(gettid()) {
            // SAFETY: `fd` was just opened here and is not stored.
            Ok(fd) => unsafe {
                close(fd);
            },
            Err(e) => {
                return Err(format!(
                    "perf_event_open of user instructions refused: {e} (kernel.perf_event_paranoid above 2, a virtual machine without counters or a container profile)"
                ));
            }
        }
        let mut st = state();
        let tids = st.tids.clone();
        for (index, tid) in tids {
            match open(tid) {
                Ok(fd) => st.fds.push((index, fd)),
                Err(e) => {
                    return Err(format!(
                        "perf_event_open of user instructions refused: {e} (kernel.perf_event_paranoid above 2, a virtual machine without counters or a container profile)"
                    ));
                }
            }
        }
        st.active = true;
        Ok(())
    }

    /// Count the calling thread (slot `index`) now (the test).
    #[cfg(test)]
    pub(super) fn register_now(index: usize) -> Result<i32, std::io::Error> {
        let fd = open(gettid())?;
        state().fds.push((index, fd));
        Ok(fd)
    }

    pub(super) fn per_thread(out: &mut Vec<(usize, u64)>, only: Option<&[usize]>) {
        let st = state();
        out.extend(
            st.fds
                .iter()
                .filter(|(i, _)| only.is_none_or(|o| o.contains(i)))
                .map(|&(i, fd)| (i, read_one(fd))),
        );
    }
}

#[cfg(not(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
mod imp {
    pub fn install() {}

    pub fn activate() -> Result<(), String> {
        Err("no instruction counter on this platform".into())
    }

    pub fn per_thread(_: &mut Vec<(usize, u64)>, _: Option<&[usize]>) {}
}

#[cfg(all(
    test,
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod tests {
    /// A registered thread's counter follows its own work, not that of the others, and a host that
    /// refuses the counter (a container profile, no PMU) is said so, not passed silently.
    #[test]
    fn counts_the_instructions_of_the_registered_thread() {
        let spin = |n: u64| {
            let mut x = 1u64;
            for i in 0..n {
                x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(i));
            }
            x
        };
        let worker = std::thread::spawn(move || {
            let fd = match super::imp::register_now(0) {
                Ok(fd) => fd,
                Err(e) => {
                    eprintln!("work clock test skipped: perf_event_open refused: {e}");
                    return None;
                }
            };
            let start = super::imp::read_one(fd);
            spin(2_000_000);
            Some(super::imp::read_one(fd) - start)
        });
        // The test thread itself is not counted: its spin adds nothing to the worker's counter.
        spin(2_000_000);
        if let Some(got) = worker.join().expect("worker") {
            assert!(
                (4_000_000..400_000_000).contains(&got),
                "2M iterations of a few instructions counted as {got}"
            );
        }
    }
}
