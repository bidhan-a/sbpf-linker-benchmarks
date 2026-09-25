#![cfg_attr(any(target_arch = "bpf", target_os = "solana"), no_std)]

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
mod program {
    pub trait Handler: Sync {
        fn check(&self, i: usize) -> u64;
    }

    pub struct Desc {
        pub key: [u8; 32],
    }

    impl Handler for Desc {
        #[inline(never)]
        fn check(&self, i: usize) -> u64 {
            if self.key[i & 31] == KEY[i & 31] {
                0
            } else {
                1
            }
        }
    }

    const KEY: [u8; 32] = [
        0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xB1, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7,
        0xB8, 0xC1, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xD1, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6,
        0xD7, 0xD8,
    ];

    pub static DESC: Desc = Desc { key: KEY };

    // Fat pointer to a trait object
    pub static DYN: &dyn Handler = &DESC;

    #[unsafe(no_mangle)]
    pub extern "C" fn entrypoint(input: *mut u8) -> u64 {
        let d: &dyn Handler = core::hint::black_box(DYN);
        // ix data starts at offset 16
        let index = unsafe { core::ptr::read_volatile(input.add(16)) } as usize;
        d.check(index)
    }
}

#[cfg(test)]
mod tests {
    use harness::{Check, Instruction, Mollusk};

    #[test]
    fn test_dyn_pointer() {
        let mollusk = Mollusk::new(&[2u8; 32].into(), &harness::program_elf());
        for index in 0..4u8 {
            mollusk.process_and_validate_instruction(
                &Instruction {
                    program_id: [2u8; 32].into(),
                    accounts: vec![],
                    data: vec![index],
                },
                &[],
                &[Check::success()],
            );
        }
    }
}
