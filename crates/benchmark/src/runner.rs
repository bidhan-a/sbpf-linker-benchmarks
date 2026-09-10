use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt::Write as _,
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::{Command, Output},
};

use sha2::{Digest, Sha256};

use crate::{
    BenchmarkResult, BenchmarkRunner, Mollusk, ProgramRegistration, Pubkey, cargo, invalid_data,
    manifest::{ExpectedBenchmark, ExpectedProgram, ProgramPackage},
    model::{BenchmarkReport, ElfReport, ProgramReport, ResultStatus, StepReport, StepStatus},
};

const DEFAULT_LOADER_ID: Pubkey = solana_sdk_ids::bpf_loader_upgradeable::ID;

struct PreparedProgram<'a> {
    expected: &'a ExpectedProgram,
    program_id: Pubkey,
    runner: Option<BenchmarkRunner>,
    build: StepReport,
    execution_log: PathBuf,
    elf: Option<ElfReport>,
    elf_bytes: Option<Vec<u8>>,
    issues: Vec<String>,
}

pub(crate) fn run_programs(
    root: &Path,
    output_dir: &Path,
    linker: &Path,
    programs: &[(usize, &ExpectedProgram)],
    packages: &BTreeMap<String, ProgramPackage>,
    registrations: &[ProgramRegistration],
) -> Result<Vec<ProgramReport>, Box<dyn Error>> {
    let mut prepared = Vec::with_capacity(programs.len());
    for &(manifest_index, expected) in programs {
        let package = packages.get(&expected.name).ok_or_else(|| {
            invalid_data(format!(
                "program `{}` disappeared from Cargo metadata",
                expected.name
            ))
        })?;
        prepared.push(prepare_program(
            root,
            output_dir,
            linker,
            manifest_index,
            expected,
            package,
            registrations,
        )?);
    }

    let mut mollusk = Mollusk::default();
    for program in &prepared {
        if let Some(elf) = &program.elf_bytes {
            mollusk.add_program_with_loader_and_elf(&program.program_id, &DEFAULT_LOADER_ID, elf);
        }
    }

    Ok(prepared
        .into_iter()
        .map(|program| execute_program(&mollusk, program))
        .collect())
}

fn prepare_program<'a>(
    root: &Path,
    output_dir: &Path,
    linker: &Path,
    manifest_index: usize,
    expected: &'a ExpectedProgram,
    package: &ProgramPackage,
    registrations: &[ProgramRegistration],
) -> Result<PreparedProgram<'a>, Box<dyn Error>> {
    let logs_dir = output_dir.join("logs");
    let build_log = logs_dir.join(format!("{}-build.log", expected.name));
    let execution_log = logs_dir.join(format!("{}-execution.log", expected.name));
    let program_id = program_id(manifest_index)?;
    let mut issues = Vec::new();
    let runner = match benchmark_runner(&expected.name, registrations) {
        Some(runner) => Some(runner),
        None => {
            issues.push(format!(
                "program `{}` has no host benchmark registration",
                expected.name
            ));
            None
        }
    };

    let mut build_command = Command::new(cargo());
    build_command
        .args([
            "build",
            "--release",
            "--target",
            "bpfel-unknown-none",
            "-Z",
            "build-std=core,alloc",
            "--locked",
            "-p",
            &expected.name,
        ])
        .current_dir(root)
        .env("CARGO_TARGET_BPFEL_UNKNOWN_NONE_LINKER", linker)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    let build_succeeded = match run_logged(&mut build_command, &build_log) {
        Ok(output) if output.status.success() => true,
        Ok(_) => {
            issues.push("sBPF build failed".to_owned());
            false
        }
        Err(error) => {
            issues.push(format!("failed to execute build: {error}"));
            false
        }
    };

    let (elf, elf_bytes) = if build_succeeded {
        let elf_path = root
            .join("target/bpfel-unknown-none/release")
            .join(format!("lib{}.so", package.target_name));
        match inspect_elf(&elf_path) {
            Ok((report, bytes)) => (Some(report), Some(bytes)),
            Err(error) => {
                issues.push(error.to_string());
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    Ok(PreparedProgram {
        expected,
        program_id,
        runner,
        build: step(
            if build_succeeded {
                StepStatus::Passed
            } else {
                StepStatus::Failed
            },
            &build_log,
        ),
        execution_log,
        elf,
        elf_bytes,
        issues,
    })
}

fn execute_program(mollusk: &Mollusk, mut program: PreparedProgram<'_>) -> ProgramReport {
    let (results, execution_status) = match (program.elf_bytes.is_some(), program.runner) {
        (true, Some(runner)) => match runner(mollusk, program.program_id) {
            Ok(results) => {
                if let Err(error) =
                    write_execution_log(&program.execution_log, program.program_id, &results, None)
                {
                    program
                        .issues
                        .push(format!("failed to write execution log: {error}"));
                }
                let status = if results.iter().all(|result| result.correctness.passed) {
                    StepStatus::Passed
                } else {
                    StepStatus::Failed
                };
                (Some(results), status)
            }
            Err(error) => {
                program.issues.push(error.to_string());
                if let Err(log_error) = write_execution_log(
                    &program.execution_log,
                    program.program_id,
                    &[],
                    Some(&error.to_string()),
                ) {
                    program
                        .issues
                        .push(format!("failed to write execution log: {log_error}"));
                }
                (None, StepStatus::Failed)
            }
        },
        (elf_available, _) => {
            let reason = if elf_available {
                "execution skipped because the program has no host benchmark registration"
            } else {
                "execution skipped because the program ELF is unavailable"
            };
            if let Err(error) = write_execution_log(
                &program.execution_log,
                program.program_id,
                &[],
                Some(reason),
            ) {
                program
                    .issues
                    .push(format!("failed to write execution log: {error}"));
            }
            (None, StepStatus::Skipped)
        }
    };

    let benchmarks = results.map_or_else(
        || missing_benchmarks(program.expected),
        |results| compare_benchmarks(program.expected, results, &mut program.issues),
    );

    ProgramReport {
        name: program.expected.name.clone(),
        build: program.build,
        execution: step(execution_status, &program.execution_log),
        elf: program.elf,
        benchmarks,
        issues: program.issues,
    }
}

fn benchmark_runner(name: &str, registrations: &[ProgramRegistration]) -> Option<BenchmarkRunner> {
    registrations
        .iter()
        .find(|registration| registration.name == name)
        .map(|registration| registration.runner)
}

fn program_id(manifest_index: usize) -> Result<Pubkey, Box<dyn Error>> {
    let counter = u64::try_from(manifest_index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .ok_or_else(|| invalid_data("program ID counter exhausted"))?;
    let mut bytes = [0u8; 32];
    bytes[..size_of::<u64>()].copy_from_slice(&counter.to_le_bytes());
    Ok(Pubkey::from(bytes))
}

fn missing_benchmarks(program: &ExpectedProgram) -> Vec<BenchmarkReport> {
    program
        .benchmarks
        .iter()
        .map(|benchmark| BenchmarkReport {
            name: benchmark.name.clone(),
            expected_compute_units: benchmark.expected_compute_units(),
            actual_compute_units: None,
            delta_compute_units: None,
            delta_percent: None,
            correctness: None,
            status: ResultStatus::MissingResult,
        })
        .collect()
}

fn compare_benchmarks(
    expected: &ExpectedProgram,
    results: Vec<BenchmarkResult>,
    issues: &mut Vec<String>,
) -> Vec<BenchmarkReport> {
    let mut results_by_name = BTreeMap::new();
    let mut duplicate_names = BTreeSet::new();
    for result in results {
        let name = result.name.clone();
        if results_by_name.insert(name.clone(), result).is_some() {
            duplicate_names.insert(name);
        }
    }
    for name in duplicate_names {
        issues.push(format!(
            "duplicate benchmark result `{}/{name}`",
            expected.name
        ));
        results_by_name.remove(&name);
    }

    let mut reports = Vec::new();
    for benchmark in &expected.benchmarks {
        reports.push(match results_by_name.remove(&benchmark.name) {
            Some(result) => compare_benchmark(benchmark, result),
            None => BenchmarkReport {
                name: benchmark.name.clone(),
                expected_compute_units: benchmark.expected_compute_units(),
                actual_compute_units: None,
                delta_compute_units: None,
                delta_percent: None,
                correctness: None,
                status: ResultStatus::MissingResult,
            },
        });
    }
    reports.extend(results_by_name.into_values().map(|result| BenchmarkReport {
        name: result.name,
        expected_compute_units: None,
        actual_compute_units: Some(result.compute_units),
        delta_compute_units: None,
        delta_percent: None,
        correctness: Some(result.correctness),
        status: ResultStatus::Unregistered,
    }));
    reports
}

fn compare_benchmark(expected: &ExpectedBenchmark, result: BenchmarkResult) -> BenchmarkReport {
    let actual = result.compute_units;
    let expected_cu = expected.expected_compute_units();
    let correctness = result.correctness;
    let (delta, delta_percent, status) = if !correctness.passed {
        (None, None, ResultStatus::CorrectnessFailed)
    } else if let Some(expected_cu) = expected_cu {
        let delta = i128::from(actual) - i128::from(expected_cu);
        let delta = i64::try_from(delta).unwrap_or(if delta.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        });
        let percentage = (expected_cu != 0).then(|| delta as f64 / expected_cu as f64 * 100.0);
        let status = match actual.cmp(&expected_cu) {
            std::cmp::Ordering::Less => ResultStatus::Improved,
            std::cmp::Ordering::Equal => ResultStatus::Unchanged,
            std::cmp::Ordering::Greater => ResultStatus::PerformanceRegression,
        };
        (Some(delta), percentage, status)
    } else {
        (None, None, ResultStatus::Unbaselined)
    };
    BenchmarkReport {
        name: expected.name.clone(),
        expected_compute_units: expected_cu,
        actual_compute_units: Some(actual),
        delta_compute_units: delta,
        delta_percent,
        correctness: Some(correctness),
        status,
    }
}

fn inspect_elf(path: &Path) -> Result<(ElfReport, Vec<u8>), Box<dyn Error>> {
    if !path.is_file() {
        return Err(invalid_data(format!(
            "expected ELF was not produced: {}",
            path.display()
        )));
    }
    let bytes = fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    let sha256 = digest
        .iter()
        .fold(String::with_capacity(64), |mut value, byte| {
            write!(&mut value, "{byte:02x}").expect("writing to a string cannot fail");
            value
        });
    let report = ElfReport {
        path: path.display().to_string(),
        size_bytes: u64::try_from(bytes.len())?,
        sha256,
    };
    Ok((report, bytes))
}

fn write_execution_log(
    path: &Path,
    program_id: Pubkey,
    results: &[BenchmarkResult],
    error: Option<&str>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut log = format!("program_id: {program_id}\n");
    for result in results {
        writeln!(
            log,
            "{}: {} CU, correctness {} ({})",
            result.name,
            result.compute_units,
            if result.correctness.passed {
                "passed"
            } else {
                "failed"
            },
            result.correctness.actual,
        )
        .expect("writing to a string cannot fail");
    }
    if let Some(error) = error {
        writeln!(log, "error: {error}").expect("writing to a string cannot fail");
    }
    fs::write(path, log)
}

fn run_logged(command: &mut Command, log_path: &Path) -> io::Result<Output> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let command_line = format!("{command:?}");
    match command.output() {
        Ok(output) => {
            let mut log = format!("$ {command_line}\n\n");
            log.push_str("--- stdout ---\n");
            log.push_str(&String::from_utf8_lossy(&output.stdout));
            log.push_str("\n--- stderr ---\n");
            log.push_str(&String::from_utf8_lossy(&output.stderr));
            log.push('\n');
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)?
                .write_all(log.as_bytes())?;
            Ok(output)
        }
        Err(error) => {
            let log = format!("$ {command_line}\n\n{error}\n\n");
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)?
                .write_all(log.as_bytes())?;
            Err(error)
        }
    }
}

fn step(status: StepStatus, log: &Path) -> StepReport {
    StepReport {
        status,
        log: log.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        BenchmarkResult, CorrectnessResult, Pubkey,
        manifest::{ExpectedBenchmark, ExpectedProgram, expected_benchmark},
        model::ResultStatus,
    };

    use super::{compare_benchmark, compare_benchmarks, program_id};

    fn result(compute_units: u64, passed: bool) -> BenchmarkResult {
        BenchmarkResult {
            name: "benchmark".to_owned(),
            correctness: CorrectnessResult {
                passed,
                actual: if passed { "Success" } else { "Failure" }.to_owned(),
                details: None,
            },
            compute_units,
        }
    }

    fn expected(compute_units: Option<u64>) -> ExpectedBenchmark {
        expected_benchmark("benchmark", compute_units)
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
    fn classifies_compute_unit_drift() {
        assert_eq!(
            compare_benchmark(&expected(Some(31)), result(31, true)).status,
            ResultStatus::Unchanged
        );
        assert_eq!(
            compare_benchmark(&expected(Some(31)), result(30, true)).status,
            ResultStatus::Improved
        );
        assert_eq!(
            compare_benchmark(&expected(Some(31)), result(32, true)).status,
            ResultStatus::PerformanceRegression
        );
    }

    #[test]
    fn prioritizes_correctness_and_unbaselined_states() {
        assert_eq!(
            compare_benchmark(&expected(Some(31)), result(1, false)).status,
            ResultStatus::CorrectnessFailed
        );
        assert_eq!(
            compare_benchmark(&expected(None), result(31, true)).status,
            ResultStatus::Unbaselined
        );
    }

    #[test]
    fn classifies_missing_unregistered_and_duplicate_results() {
        let expected = ExpectedProgram {
            name: "program".to_owned(),
            benchmarks: vec![expected_benchmark("expected", Some(1))],
        };

        let mut issues = Vec::new();
        let reports = compare_benchmarks(
            &expected,
            vec![
                BenchmarkResult {
                    name: "extra".to_owned(),
                    ..result(2, true)
                },
                BenchmarkResult {
                    name: "duplicate".to_owned(),
                    ..result(3, true)
                },
                BenchmarkResult {
                    name: "duplicate".to_owned(),
                    ..result(4, true)
                },
            ],
            &mut issues,
        );

        assert_eq!(reports[0].status, ResultStatus::MissingResult);
        assert_eq!(reports[1].status, ResultStatus::Unregistered);
        assert_eq!(reports[1].name, "extra");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("duplicate"));
    }
}
