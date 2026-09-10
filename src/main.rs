use std::{path::Path, process::ExitCode};

use sbpf_benchmark::ProgramRegistration;

fn main() -> ExitCode {
    let programs = [
        ProgramRegistration::new("function-pointer", function_pointer::benchmark::run),
        ProgramRegistration::new("pointer-table", pointer_table::benchmark::run),
    ];

    sbpf_benchmark::run(Path::new(env!("CARGO_MANIFEST_DIR")), &programs)
}
