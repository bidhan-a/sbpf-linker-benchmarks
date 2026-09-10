#![cfg_attr(target_arch = "bpf", no_std)]

#[cfg(target_arch = "bpf")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(target_arch = "bpf")]
mod program {
    #[repr(transparent)]
    struct Pubkey([u8; 32]);

    static AUTH0: Pubkey = Pubkey([0x11; 32]);
    static AUTH1: Pubkey = Pubkey([0x22; 32]);
    static AUTH2: Pubkey = Pubkey([0x33; 32]);
    static AUTH3: Pubkey = Pubkey([0x44; 32]);

    #[used]
    static REGISTRY: [&Pubkey; 4] = [&AUTH0, &AUTH1, &AUTH2, &AUTH3];

    #[unsafe(no_mangle)]
    fn entrypoint(input: *mut u8) -> u64 {
        // Instruction data starts at offset 16.
        let index = unsafe { core::ptr::read_volatile(input.add(16)) } as usize;

        let registry = core::ptr::addr_of!(REGISTRY).cast::<*const Pubkey>();
        let key = unsafe { core::ptr::read_volatile(registry.add(index)) };
        let val = unsafe { core::ptr::read_volatile(key.cast::<u8>()) };
        let expected = [0x11u8, 0x22, 0x33, 0x44][index];

        if val == expected {
            0x0
        } else {
            0x1
        }
    }
}

#[cfg(not(target_arch = "bpf"))]
pub mod benchmark {
    use sbpf_benchmark::{
        Benchmark, BenchmarkError, BenchmarkResult, Check, Instruction, Mollusk, Pubkey,
    };

    pub fn run(
        mollusk: &Mollusk,
        program_id: Pubkey,
    ) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
        let result = Benchmark::new("bench-pointer-table").run(
            mollusk,
            &Instruction {
                program_id,
                accounts: vec![],
                data: vec![2],
            },
            &[],
            &[Check::success()],
        )?;

        Ok(vec![result])
    }
}
