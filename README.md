# sBPF Linker Benchmarks

A workflow for building, testing, and benchmarking sBPF programs. Each program is compiled using the compiler specified by the runner(`cargo-build-sbpf` or `cargo-build-sbf`), followed by execution of its Mollusk tests and benchmarks. The results are aggregated and displayed in a single report in the terminal.

## Compilers

The runner accepts a single required flag:

```bash
cargo run -- --compiler=<cargo-build-sbpf|cargo-build-sbf>
```

| Compiler | Tool invoked
|---|---|
| `cargo-build-sbpf` | `cargo build-sbpf` (nightly Rust + `sbpf-linker`)
| `cargo-build-sbf` | `cargo build-sbf --arch v3` (Solana platform tools)


## Programs and Manifest

All programs are independent Cargo packages under `programs/`:

```text
programs/
├── manifest.json
├── function-pointer/
│   ├── Cargo.toml
│   └── src/lib.rs
└── pointer-table/
    ├── Cargo.toml
    └── src/lib.rs
```

Each package contains the on-chain program plus, optionally:

- a `#[cfg(test)] mod tests {..}` block with Mollusk correctness tests.
- a `pub mod benchmark {..}` block defining one or more benchmarks.

[`programs/manifest.json`](programs/manifest.json) lists every program, its benchmark names, and their expected 
baseline compute units. A program with tests only can omit the `benchmarks` field (or leave it empty).
Programs without a benchmark module are only built and tested and are not included in the final benchmark report.

Baselines in `programs/manifest.json` are generated with the release `cargo-build-sbpf` and `sbpf-linker` and both compilers 
are compared against the same baselines.

## Example runs

cargo-build-sbf
```
> cargo run -- --compiler cargo-build-sbf
...
<build and test logs>
...

Benchmark Report
Compiler: cargo-build-sbf 4.1.0 / platform-tools v1.54 / rustc 1.89.0

Program            Benchmark                Expected CU   Actual CU     Diff
function-pointer   bench-function-pointer            32          30       -2
pointer-table      bench-pointer-table               15          17       +2

Run complete
Artifacts generated at: /Users/bidhan/sbpf-linker-benchmarks/artifacts/cargo-build-sbf/1790160477
```

cargo-build-sbpf
```
> cargo run -- --compiler cargo-build-sbpf
...
<build and test logs>
...

Benchmark Report
Compiler: cargo-build-sbpf 0.1.0 / sbpf-linker 0.2.1 / LLVM 23.1.1 / rustc 1.100.0-nightly (215a8af4b 2026-09-15)

Program            Benchmark                Expected CU   Actual CU     Diff
function-pointer   bench-function-pointer            32          32        0
pointer-table      bench-pointer-table               15          15        0

Run complete
Artifacts generated at: /Users/bidhan/sbpf-linker-benchmarks/artifacts/cargo-build-sbpf/1790160550
```


## How It Works

1. The runner reads `programs/manifest.json` to determine which programs and benchmarks to run, and their expected baseline CUs.
2. Each program is built with the selected compiler inside its package directory. 
3. Each program's tests run with `cargo test -p <program>`.
4. For benchmarking, built ELFs are loaded into one shared Mollusk instance with unqiue program IDs.
5. Each program's `benchmark::run(&Mollusk, Pubkey)` executes its instruction and account combinations and returns `BenchmarkResult` values.
6. Results are cross-checked against the manifest and each row is printed with its baseline, measured CUs, and the diff (`measured - baseline`).
7. The combined report is printed to the terminal.

Test and build output streams to the terminal as it happens and the benchmark report is printed at the end.

## Artifacts

Every successful build is archived under `artifacts/` folder (ignored in git):

```text
artifacts/
└── <compiler>/<program>/<unix-timestamp>/
    ├── <crate>.so          copy of the built ELF
    └── disassembly.s       `sbpf disassemble` output
```

Currently, we can only view these artifacts but later we can add commands to compare and analyze them (like diffing the disassembly of two runs to debug a CU change).


## How to Add a Program

1. Add the program as a Cargo package under `programs/`.
2. Add Mollusk tests if needed. Load the program ELF with `harness::program_elf()`, which returns the ELF path based on the compiler used, so we don't have to hardcode the paths. An example can be found [here](programs/function-pointer/src/lib.rs#L51).
3. To benchmark, add a [benchmark module](programs/function-pointer/src/lib.rs#L73).

   ```rust,ignore
   #[cfg(not(any(target_arch = "bpf", target_os = "solana")))]
   pub mod benchmark {
       use harness::{Benchmark, BenchmarkError, BenchmarkResult, Instruction, Mollusk, Pubkey};

       pub fn run(
           mollusk: &Mollusk,
           program_id: Pubkey,
       ) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
           let result = Benchmark::new("bench-example").run(
               mollusk,
               &Instruction { program_id, accounts: vec![], data: vec![0] },
               &[],
           )?;
           Ok(vec![result]) // one report row per result
       }
   }
   ```

4. If the program has benchmarks, add the program as dependency to the root `Cargo.toml` (test-only programs do not need to be added).
5. Register the program and its benchmarks (with baselines) in `programs/manifest.json`.
