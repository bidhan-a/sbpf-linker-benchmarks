#![cfg_attr(any(target_arch = "bpf", target_os = "solana"), no_std)]

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
mod program {
    #[inline(never)]
    fn a(value: u64) -> u64 {
        core::hint::black_box(value);
        0
    }

    #[inline(never)]
    fn b(value: u64) -> u64 {
        core::hint::black_box(value);
        1
    }

    #[repr(C)]
    pub struct Entry {
        layout: &'static [u8],
        run: fn(u64) -> u64,
        id: [u8; 32],
        tag: u16,
    }

    #[unsafe(no_mangle)]
    pub static TABLE: [Entry; 2] = [
        Entry {
            layout: &[1, 2, 3],
            run: a,
            id: [0x11; 32],
            tag: 1,
        },
        Entry {
            layout: &[4, 5],
            run: b,
            id: [0x22; 32],
            tag: 2,
        },
    ];

    #[unsafe(no_mangle)]
    pub extern "C" fn entrypoint(input: *mut u8) -> u64 {
        let index = unsafe { core::ptr::read_volatile(input.add(16)) } as usize;
        let table = core::hint::black_box(&TABLE);
        let entry = &table[index % table.len()];

        core::hint::black_box(entry.layout);
        core::hint::black_box(&entry.id);
        core::hint::black_box(entry.tag);

        (entry.run)(index as u64)
    }
}

#[cfg(test)]
mod tests {
    use harness::{Check, Instruction, Mollusk};

    #[test]
    fn test_struct_table() {
        let mollusk = Mollusk::new(&[2u8; 32].into(), &harness::program_elf());
        for index in 0..4u8 {
            let checks = if index % 2 == 0 {
                vec![Check::success()]
            } else {
                vec![Check::err(1u64.into())]
            };
            mollusk.process_and_validate_instruction(
                &Instruction {
                    program_id: [2u8; 32].into(),
                    accounts: vec![],
                    data: vec![index],
                },
                &[],
                &checks,
            );
        }
    }
}
