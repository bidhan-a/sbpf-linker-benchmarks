#![cfg_attr(any(target_arch = "bpf", target_os = "solana"), no_std)]

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
mod program {
    #[repr(transparent)]
    struct Pubkey([u8; 32]);

    static AUTH0: Pubkey = Pubkey([0x11; 32]);
    static AUTH1: Pubkey = Pubkey([0x22; 32]);
    static AUTH2: Pubkey = Pubkey([0x33; 32]);
    static AUTH3: Pubkey = Pubkey([0x44; 32]);

    #[unsafe(no_mangle)]
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

#[cfg(test)]
mod tests {
    use harness::{Check, Instruction, Mollusk, Pubkey};

    #[test]
    fn test_pointer_table_all_indices() {
        let program_id: Pubkey = [2u8; 32].into();
        let mollusk = Mollusk::new(&program_id, &harness::program_elf());
        for index in 0..4u8 {
            mollusk.process_and_validate_instruction(
                &Instruction {
                    program_id,
                    accounts: vec![],
                    data: vec![index],
                },
                &[],
                &[Check::success()],
            );
        }
    }
}

#[cfg(not(any(target_arch = "bpf", target_os = "solana")))]
pub mod benchmark {
    use harness::{Benchmark, BenchmarkError, BenchmarkResult, Instruction, Mollusk, Pubkey};

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
        )?;

        Ok(vec![result])
    }
}
