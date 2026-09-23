#![cfg_attr(any(target_arch = "bpf", target_os = "solana"), no_std)]

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(any(target_arch = "bpf", target_os = "solana"))]
mod program {
    #[unsafe(no_mangle)]
    pub static SMALL: [u8; 3] = [1, 2, 3];

    #[repr(C, align(64))]
    pub struct Align64(pub [u8; 64]);

    #[unsafe(no_mangle)]
    pub static ALIGNED: Align64 = Align64([0x5a; 64]);

    #[unsafe(no_mangle)]
    pub extern "C" fn entrypoint(_: *mut u8) -> u64 {
        let ptr = core::hint::black_box(core::ptr::addr_of!(ALIGNED));
        (ptr as usize & 63) as u64
    }
}

#[cfg(test)]
mod tests {
    use harness::{Instruction, Mollusk};

    #[test]
    fn aligned_static_address_is_64_byte_aligned() {
        let program_id = [2u8; 32].into();

        let mollusk = Mollusk::new(&program_id, &harness::program_elf());

        let instruction = Instruction::new_with_bytes(program_id.into(), &[], vec![]);
        let result = mollusk.process_instruction(&instruction, &[]);

        // The program returns `address_of(ALIGNED) % 64`. A correctly laid-out
        // `#[repr(align(64))]` static therefore returns zero (program success).
        assert_eq!(result.raw_result, Ok(()));
    }
}
