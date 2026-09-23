use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{
    BenchmarkResult, BenchmarkRunner, Mollusk, ProgramRegistration, Pubkey, cargo,
    compiler::Compiler,
    invalid_data,
    manifest::{ExpectedBenchmark, ExpectedProgram, Manifest, ProgramPackage},
    model::{BenchmarkReport, ProgramReport},
};

const DEFAULT_LOADER_ID: Pubkey = solana_sdk_ids::bpf_loader_upgradeable::ID;

pub(crate) struct PreparedProgram<'a> {
    expected: &'a ExpectedProgram,
    program_id: Pubkey,
    runner: Option<BenchmarkRunner>,
    elf_path: Option<PathBuf>,
    elf_bytes: Option<Vec<u8>>,
    loaded: bool,
}

pub(crate) fn run_programs(
    root: &Path,
    compiler: Compiler,
    registrations: &[ProgramRegistration],
    manifest: &Manifest,
    packages: &BTreeMap<String, ProgramPackage>,
    run_timestamp: u64,
) -> Result<Vec<ProgramReport>, Box<dyn Error>> {
    let mut runners = BTreeMap::new();
    for registration in registrations {
        if runners
            .insert(registration.name, registration.runner)
            .is_some()
        {
            return Err(invalid_data(format!(
                "duplicate benchmark registration for program `{}`",
                registration.name
            )));
        }
    }

    let mut prepared = Vec::with_capacity(manifest.programs.len());
    for (manifest_index, expected) in manifest.programs.iter().enumerate() {
        let package = packages.get(&expected.name).ok_or_else(|| {
            invalid_data(format!(
                "program `{}` disappeared from Cargo metadata",
                expected.name
            ))
        })?;
        prepared.push(prepare_program(
            root,
            compiler,
            manifest_index,
            expected,
            package,
            runners.get(expected.name.as_str()).copied(),
            run_timestamp,
        )?);
    }

    run_tests(root, &mut prepared);

    let mut mollusk = Mollusk::default();
    for program in &mut prepared {
        let Some(elf) = program.elf_bytes.as_deref() else {
            continue;
        };
        // The ELF loader panics on unparsable programs; a failure must not
        // abort the run, it only excludes the program from execution.
        let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mollusk.add_program_with_loader_and_elf(&program.program_id, &DEFAULT_LOADER_ID, elf);
        }))
        .is_ok();
        program.loaded = loaded;
    }

    let mut reports = Vec::with_capacity(prepared.len());
    for program in prepared {
        reports.push(execute_program(&mollusk, program)?);
    }
    Ok(reports)
}

fn prepare_program<'a>(
    root: &Path,
    compiler: Compiler,
    manifest_index: usize,
    expected: &'a ExpectedProgram,
    package: &ProgramPackage,
    runner: Option<BenchmarkRunner>,
    run_timestamp: u64,
) -> Result<PreparedProgram<'a>, Box<dyn Error>> {
    let program_id = program_id(manifest_index)?;

    let program_dir = root.join("programs").join(&expected.name);
    let status = Command::new(cargo())
        .arg(compiler.subcommand())
        .args(compiler.build_args())
        .current_dir(&program_dir)
        .status();
    let built = matches!(status, Ok(status) if status.success());

    let elf_path = root
        .join(compiler.target_dir())
        .join(compiler.elf_file_name(&package.target_name));
    let (elf_path, elf_bytes) = if built {
        match fs::read(&elf_path) {
            Ok(bytes) => {
                save_artifact(
                    root,
                    compiler,
                    &expected.name,
                    &elf_path,
                    &compiler.elf_file_name(&package.target_name),
                    run_timestamp,
                );
                (Some(elf_path), Some(bytes))
            }
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };

    Ok(PreparedProgram {
        expected,
        program_id,
        runner,
        elf_path,
        elf_bytes,
        loaded: false,
    })
}

fn save_artifact(
    root: &Path,
    compiler: Compiler,
    program_name: &str,
    elf_path: &Path,
    elf_file_name: &str,
    run_timestamp: u64,
) {
    let artifacts_dir = root
        .join("artifacts")
        .join(compiler.tool_name())
        .join(run_timestamp.to_string())
        .join(program_name);
    if let Err(error) = fs::create_dir_all(&artifacts_dir) {
        eprintln!(
            "warning: failed to create artifacts directory `{}`: {error}",
            artifacts_dir.display()
        );
        return;
    }
    if let Err(error) = fs::copy(elf_path, artifacts_dir.join(elf_file_name)) {
        eprintln!("warning: failed to copy ELF into artifacts: {error}");
        return;
    }
    match Command::new("sbpf")
        .arg("disassemble")
        .arg(elf_path)
        .output()
    {
        Ok(output) if output.status.success() => {
            if let Err(error) = fs::write(artifacts_dir.join("disassembly.s"), output.stdout) {
                eprintln!("warning: failed to write disassembly: {error}");
            }
        }
        Ok(output) => eprintln!(
            "warning: `sbpf disassemble` failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => eprintln!("warning: failed to run `sbpf disassemble`: {error}"),
    }
}

fn run_tests(root: &Path, programs: &mut [PreparedProgram<'_>]) {
    for program in programs {
        let Some(elf_path) = program.elf_path.as_ref() else {
            continue;
        };
        let status = Command::new(cargo())
            .args(["test", "-p", &program.expected.name])
            .current_dir(root)
            .env(crate::SBPF_PROGRAM_ELF_ENV, elf_path)
            .status();
        if let Err(error) = status {
            eprintln!("error: failed to execute tests: {error}");
        }
    }
}

fn execute_program(
    mollusk: &Mollusk,
    program: PreparedProgram<'_>,
) -> Result<ProgramReport, Box<dyn Error>> {
    let mut benchmarks = Vec::new();
    if program.loaded
        && let Some(runner) = program.runner
    {
        match runner(mollusk, program.program_id) {
            Ok(results) => benchmarks = benchmark_reports(program.expected, results)?,
            Err(error) => eprintln!("error: {error}"),
        }
    }

    Ok(ProgramReport {
        name: program.expected.name.clone(),
        benchmarks,
    })
}

fn benchmark_reports(
    expected: &ExpectedProgram,
    results: Vec<BenchmarkResult>,
) -> Result<Vec<BenchmarkReport>, Box<dyn Error>> {
    let mut matched = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for result in results {
        let name = result.name.clone();
        if !seen.insert(name.clone()) {
            return Err(invalid_data(format!(
                "duplicate benchmark result `{}` for program `{}`",
                name, expected.name
            )));
        }
        if expected.benchmarks.iter().any(|b| b.name == name) {
            matched.entry(name).or_insert(result);
        } else {
            return Err(invalid_data(format!(
                "benchmark `{}` returned by program `{}` is not in the manifest",
                name, expected.name
            )));
        }
    }

    Ok(expected
        .benchmarks
        .iter()
        .filter_map(|benchmark| {
            matched
                .remove(&benchmark.name)
                .map(|result| benchmark_report(benchmark, result))
        })
        .collect())
}

fn benchmark_report(expected: &ExpectedBenchmark, result: BenchmarkResult) -> BenchmarkReport {
    let baseline = expected.expected_compute_units;
    BenchmarkReport {
        name: expected.name.clone(),
        expected_compute_units: baseline,
        actual_compute_units: result.compute_units,
        delta_compute_units: result.compute_units as i64 - baseline as i64,
    }
}

fn program_id(manifest_index: usize) -> Result<Pubkey, Box<dyn Error>> {
    let value = u64::try_from(manifest_index)
        .map_err(|error| invalid_data(format!("invalid manifest index: {error}")))?
        .checked_add(1)
        .ok_or_else(|| invalid_data("manifest index overflow"))?;
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&value.to_le_bytes());
    Ok(Pubkey::from(bytes))
}

#[cfg(test)]
mod tests {
    use crate::{
        BenchmarkResult, Pubkey,
        manifest::{ExpectedProgram, expected_benchmark},
    };

    use super::{benchmark_report, benchmark_reports, program_id};

    fn result(compute_units: u64) -> BenchmarkResult {
        BenchmarkResult {
            name: "benchmark".to_owned(),
            compute_units,
        }
    }

    fn expected() -> crate::manifest::ExpectedBenchmark {
        expected_benchmark("benchmark", 31)
    }

    #[test]
    fn assigns_unique_nonzero_program_ids() {
        let first = program_id(0).unwrap();
        let second = program_id(1).unwrap();

        assert_ne!(first, Pubkey::default());
        assert_ne!(first, second);
        assert_eq!(first.to_bytes()[..8], 1u64.to_le_bytes());
        assert_eq!(second.to_bytes()[..8], 2u64.to_le_bytes());
    }

    #[test]
    fn compares_against_the_baseline() {
        let report = benchmark_report(&expected(), result(30));

        assert_eq!(report.expected_compute_units, 31);
        assert_eq!(report.actual_compute_units, 30);
        assert_eq!(report.delta_compute_units, -1);
    }

    #[test]
    fn rejects_unregistered_results() {
        let expected = ExpectedProgram {
            name: "program".to_owned(),
            benchmarks: vec![expected_benchmark("expected", 1)],
        };

        let error = benchmark_reports(&expected, vec![result(2)]).unwrap_err();

        assert!(error.to_string().contains("not in the manifest"));
    }

    #[test]
    fn rejects_duplicate_results() {
        let expected = ExpectedProgram {
            name: "program".to_owned(),
            benchmarks: vec![expected_benchmark("benchmark", 1)],
        };

        let error = benchmark_reports(&expected, vec![result(2), result(3)]).unwrap_err();

        assert!(error.to_string().contains("duplicate"));
    }
}
