# sBPF Linker Benchmarks

Workflow for benchmarking sBPF programs built using `sbpf-linker` and upstream tooling.

The workflow uses a single benchmark runner to run benchmarks for all programs. It keeps all loaded programs and measurements in one process and uses a common Mollusk instance to execute the benchmarks. This makes it possible to aggregate the results for all programs and generate a combined report.

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

Each package contains the sBPF program and a `benchmark` [module](programs/function-pointer/src/lib.rs#L51) that defines its instructions, accounts, input data, and correctness checks. A program can define multiple benchmarks.

[`programs/manifest.json`](programs/manifest.json) contains a list of programs to benchmark. Each program entry lists its benchmark names and expected baseline CU consumption. More benchmark metrics can be added later, such as ELF size.

The baseline CUs are generated using the release version of `sbpf-linker`.

For example:

```json
{
  "schema_version": 1,
  "programs": [
    {
      "name": "function-pointer",
      "benchmarks": [
        {
          "name": "bench-function-pointer",
          "expected_compute_units": 31
        }
        {...}
      ]
    },
    {...}
  ]
}
```

## How It Works

1. The entry point in `src/main.rs` starts the benchmark runner.

2. The runner reads `programs/manifest.json` to determine which programs and benchmarks to run and their expected baseline CUs.

3. It builds each program for `bpfel-unknown-none` with the selected sbpf-linker.

4. It loads the successfully built program ELFs into a shared Mollusk instance with unique program IDs.

5. For each program, the runner calls its `benchmark::run(&Mollusk, Pubkey)` function. The function executes one or more instruction, account, and input combinations and returns their `BenchmarkResult` values.

6. The runner collects all results, validates their correctness, and compares their measured CUs with the baselines in the manifest.

7. Finally, it generates a combined report and prints it to the terminal


## How to Add Benchmarks

1. Add the sBPF program as a Cargo package, for example `programs/example-program/`

2. Add a benchmark module to the program crate:

   ```rust,ignore
   #[cfg(not(target_arch = "bpf"))]
   pub mod benchmark {
       use sbpf_benchmark::{BenchmarkError, BenchmarkResult, Mollusk, Pubkey};

       pub fn run(
           mollusk: &Mollusk,
           program_id: Pubkey,
       ) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
           // Construct instructions, accounts, and checks, then return each result.
       }
   }
   ```

3. Use `Benchmark::new("bench-example").run(...)` for each instruction/account/input combination that should be measured. Return all results from the module's `run` function.

   Every result is an independent report row, so a program may define multiple benchmarks:

   ```rust,ignore
   let result = Benchmark::new("bench-example").run(
       mollusk,
       &instruction,
       &accounts,
       &[Check::success()],
   )?;

   Ok(vec![result])
   ```

4. Register the program and its benchmarks in `programs/manifest.json`.

## Running Locally

The benchmarks can be run locally with:

```bash
cargo run --linker /path/to/sbpf-linker
```

A successful run prints a summary like this:

```text
sBPF Linker Benchmark Report

Linker: sbpf-linker 0.2.1 / LLVM 23.1.0

Program            Benchmark                Expected CU   Actual CU    Delta  Result

function-pointer   bench-function-pointer            31          31        0    PASS

pointer-table      bench-pointer-table               23          23        0    PASS

Benchmarks passed: 2/2

Result: PASSED
```

## How it will work in CI

1. A PR is opened against `sbpf-linker`.

2. The `sbpf-linker` CI job builds the linker from the PR branch

3. The job checks out this benchmark repository and runs the benchmarks using the linker built in step 2:

   ```bash
   cargo run --linker /path/to/sbpf-linker
   ```

4. The benchmark runner reads `programs/manifest.json`, runs the benchmark workflow, and creates a final Markdown report

5. The CI job posts the Markdown report as a comment on the PR itself so that correctness failures and CU changes are visible during review

6. The CI job fails when there is a build failure, correctness failure, CU regression, or other benchmark failure