#![cfg_attr(any(target_arch = "bpf", target_os = "solana"), no_std)]

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
mod program {
    pub struct Desc {
        pub validate: fn(&[u8]) -> bool,
        pub key: [u8; 32],
    }

    fn always_true(b: &[u8]) -> bool {
        !b.is_empty()
    }

    const DESC: Desc = Desc {
        validate: always_true,
        key: [
            0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6,
            0xB7, 0xB8, 0xC1, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xD1, 0xD2, 0xD3, 0xD4,
            0xD5, 0xD6, 0xD7, 0xD8,
        ],
    };

    #[inline(never)]
    fn use_desc(d: &Desc, probe: &[u8], i: usize) -> u64 {
        if !(d.validate)(probe) {
            return 1;
        }
        if d.key[i & 31] == DESC.key[i & 31] {
            0
        } else {
            1
        }
    }

    #[unsafe(no_mangle)]
    extern "C" fn entrypoint(input: *mut u8) -> u64 {
        let probe = unsafe { core::slice::from_raw_parts(input, 1) };
        let d: &Desc = core::hint::black_box(&DESC);
        let index = unsafe { core::ptr::read_volatile(input.add(16)) } as usize;
        use_desc(d, probe, index)
    }
}

#[cfg(test)]
mod tests {
    use harness::{Check, Instruction, Mollusk, Pubkey};

    #[test]
    fn test_function_pointer() {
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
        let result = Benchmark::new("bench-function-pointer").run(
            mollusk,
            &Instruction {
                program_id,
                accounts: vec![],
                data: vec![3],
            },
            &[],
        )?;

        Ok(vec![result])
    }
}
