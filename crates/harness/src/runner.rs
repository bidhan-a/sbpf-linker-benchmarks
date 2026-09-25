use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{
    BenchmarkResult, BenchmarkRunner, Mollusk, ProgramRegistration, Pubkey, cargo,
    compiler::Compiler,
    invalid_data, logging,
    manifest::{ExpectedBenchmark, ExpectedProgram, Manifest, ProgramPackage},
    model::{BenchmarkReport, ProgramReport, TestOutcome, TestReport},
};

const DEFAULT_LOADER_ID: Pubkey = solana_sdk_ids::bpf_loader_upgradeable::ID;

pub(crate) struct PreparedProgram<'a> {
    expected: &'a ExpectedProgram,
    program_id: Pubkey,
    runner: Option<BenchmarkRunner>,
    elf_path: PathBuf,
    elf_bytes: Vec<u8>,
    load_error: Option<String>,
    tests: Vec<TestReport>,
    test_output: String,
}

struct SpinnerGuard<'a>(&'a indicatif::ProgressBar);
impl Drop for SpinnerGuard<'_> {
    fn drop(&mut self) {
        self.0.finish_and_clear();
    }
}

pub(crate) fn run_programs(
    root: &Path,
    compiler: Compiler,
    registrations: &[ProgramRegistration],
    manifest: &Manifest,
    packages: &BTreeMap<String, ProgramPackage>,
    run_timestamp: u64,
) -> Result<(Vec<ProgramReport>, bool), Box<dyn Error>> {
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
        )?);
    }

    // Display spinner while report is being generated.
    let spinner = indicatif::ProgressBar::new_spinner();
    spinner.set_style(
        indicatif::ProgressStyle::with_template("{spinner} {msg}")
            .expect("spinner template is static"),
    );
    spinner.set_message("Generating report...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));
    let _spinner = SpinnerGuard(&spinner);

    run_tests(root, &mut prepared)?;

    let mut mollusk = Mollusk::default();
    for program in &mut prepared {
        // If Mollusk panics when loading a program, catch it and report as error
        // instead of crashing and exiting the entire process.
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mollusk.add_program_with_loader_and_elf(
                &program.program_id,
                &DEFAULT_LOADER_ID,
                &program.elf_bytes,
            );
        }));
        std::panic::set_hook(previous_hook);
        if let Err(payload) = result {
            program.load_error = Some(panic_message(&payload));
        }
    }

    let mut reports = Vec::with_capacity(prepared.len());
    let mut saved_artifacts = false;
    for program in &mut prepared {
        let report = execute_program(&mollusk, program)?;
        // Artifacts are only saved for runs with an issue.
        let has_issue = report.tests.iter().any(TestReport::is_failed)
            || report.benchmarks.iter().any(BenchmarkReport::has_issue)
            || program.load_error.is_some();
        if has_issue {
            save_artifact(
                compiler,
                &program.expected.name,
                &program.elf_path,
                run_timestamp,
            );
            saved_artifacts = true;
        }
        reports.push(report);
    }
    if saved_artifacts {
        let mut log_content = String::new();
        for program in &prepared {
            if !program.test_output.is_empty() {
                log_content.push_str(&format!(
                    "--- cargo test -p {} ---\n{}\n\n",
                    program.expected.name, program.test_output
                ));
            }
        }
        log_content.push_str("--- runtime log ---\n");
        for record in logging::take_records() {
            log_content.push_str(&record);
            log_content.push('\n');
        }
        let log_path = Path::new("/tmp")
            .join(compiler.tool_name())
            .join(run_timestamp.to_string())
            .join("log.txt");
        if let Err(error) = fs::write(&log_path, log_content) {
            eprintln!("warning: failed to write log.txt: {error}");
        }
    }
    Ok((reports, saved_artifacts))
}

fn prepare_program<'a>(
    root: &Path,
    compiler: Compiler,
    manifest_index: usize,
    expected: &'a ExpectedProgram,
    package: &ProgramPackage,
    runner: Option<BenchmarkRunner>,
) -> Result<PreparedProgram<'a>, Box<dyn Error>> {
    let program_id = program_id(manifest_index)?;

    let program_dir = root.join("programs").join(&expected.name);
    let status = Command::new(cargo())
        .arg(compiler.subcommand())
        .args(compiler.build_args())
        .current_dir(&program_dir)
        .status()
        .map_err(|error| {
            format!(
                "failed to spawn `{}` for program `{}`: {error}",
                cargo().to_string_lossy(),
                expected.name
            )
        })?;
    if !status.success() {
        return Err(format!("program `{}` failed to build", expected.name).into());
    }

    let elf_path = compiler.elf_path(root, &package.target_name);
    let elf_bytes = fs::read(&elf_path).map_err(|error| {
        format!(
            "program `{}` built but ELF not found at `{}`: {error}",
            expected.name,
            elf_path.display()
        )
    })?;

    Ok(PreparedProgram {
        expected,
        program_id,
        runner,
        elf_path,
        elf_bytes,
        load_error: None,
        test_output: String::new(),
        tests: Vec::new(),
    })
}

fn run_tests(root: &Path, programs: &mut [PreparedProgram<'_>]) -> Result<(), Box<dyn Error>> {
    for program in programs {
        let output = Command::new(cargo())
            .args(["test", "-p", &program.expected.name])
            .current_dir(root)
            .env(crate::SBPF_PROGRAM_ELF_ENV, &program.elf_path)
            .output()
            .map_err(|error| {
                format!(
                    "failed to execute tests for program `{}`: {error}",
                    program.expected.name
                )
            })?;
        program.test_output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr_text = error_text(&String::from_utf8_lossy(&output.stderr));
        program.tests = parse_test_output(&String::from_utf8_lossy(&output.stdout));
        if program.tests.is_empty() && !output.status.success() {
            program.tests.push(TestReport {
                name: "-".to_owned(),
                outcome: TestOutcome::Failed,
                error: if stderr_text.is_empty() {
                    Some("unknown error".to_owned())
                } else {
                    Some(stderr_text)
                },
            });
        }
    }
    Ok(())
}

// Parses the per-test rows out of libtest's output.
fn parse_test_output(stdout: &str) -> Vec<TestReport> {
    let mut tests: Vec<TestReport> = Vec::new();
    let mut messages: BTreeMap<String, String> = BTreeMap::new();
    let mut block_name: Option<String> = None;
    let mut block_lines: Vec<String> = Vec::new();

    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("test ")
            && let Some((name, outcome)) = rest.split_once(" ... ")
        {
            let outcome = match outcome {
                "ok" => TestOutcome::Passed,
                "FAILED" => TestOutcome::Failed,
                "ignored" => TestOutcome::Ignored,
                _ => continue,
            };
            tests.push(TestReport {
                name: name.to_owned(),
                outcome,
                error: None,
            });
            continue;
        }

        if let Some(name) = line
            .strip_prefix("---- ")
            .and_then(|rest| rest.strip_suffix(" stdout ----"))
        {
            flush_message_block(&mut block_name, &mut block_lines, &mut messages);
            block_name = Some(name.to_owned());
            continue;
        }

        if block_name.is_some() {
            if line == "failures:" || line.starts_with("---- ") {
                flush_message_block(&mut block_name, &mut block_lines, &mut messages);
                continue;
            }
            block_lines.push(line.to_owned());
        }
    }
    flush_message_block(&mut block_name, &mut block_lines, &mut messages);

    for test in &mut tests {
        if let Some(error) = messages.get(&test.name) {
            test.error = Some(error.clone());
        }
    }
    tests
}

fn flush_message_block(
    block_name: &mut Option<String>,
    block_lines: &mut Vec<String>,
    messages: &mut BTreeMap<String, String>,
) {
    if let Some(name) = block_name.take() {
        let message = block_lines
            .iter()
            .map(String::as_str)
            .find(|line| !line.is_empty() && !line.starts_with("thread '"))
            .map(ToOwned::to_owned);
        if let Some(message) = message {
            messages.insert(name, message);
        }
    }
    block_lines.clear();
}

fn execute_program(
    mollusk: &Mollusk,
    program: &mut PreparedProgram<'_>,
) -> Result<ProgramReport, Box<dyn Error>> {
    let mut benchmarks = Vec::new();
    if let Some(message) = &program.load_error {
        benchmarks = failed_benchmark_reports(program.expected, message);
    } else if let Some(runner) = program.runner {
        match runner(mollusk, program.program_id) {
            Ok(results) => benchmarks = benchmark_reports(program.expected, results)?,
            Err(error) => {
                benchmarks = failed_benchmark_reports(program.expected, &error.to_string());
            }
        }
    }

    Ok(ProgramReport {
        name: program.expected.name.clone(),
        benchmarks,
        tests: std::mem::take(&mut program.tests),
    })
}

fn save_artifact(compiler: Compiler, program_name: &str, elf_path: &Path, run_timestamp: u64) {
    let artifacts_dir = Path::new("/tmp")
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
    let Some(elf_file_name) = elf_path.file_name() else {
        return;
    };
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
            matched.insert(name, result);
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
        .map(|benchmark| match matched.remove(&benchmark.name) {
            Some(result) => benchmark_report(benchmark, result),
            None => benchmark_error_row(benchmark, "benchmark not reported by program"),
        })
        .collect())
}

fn benchmark_report(expected: &ExpectedBenchmark, result: BenchmarkResult) -> BenchmarkReport {
    let baseline = expected.expected_compute_units;
    let error = result.error.as_deref().map(error_text);
    BenchmarkReport {
        name: expected.name.clone(),
        expected_compute_units: baseline,
        actual_compute_units: result.compute_units,
        delta_compute_units: result.compute_units as i64 - baseline as i64,
        error,
    }
}

fn failed_benchmark_reports(expected: &ExpectedProgram, message: &str) -> Vec<BenchmarkReport> {
    expected
        .benchmarks
        .iter()
        .map(|benchmark| benchmark_error_row(benchmark, message))
        .collect()
}

fn benchmark_error_row(benchmark: &ExpectedBenchmark, message: &str) -> BenchmarkReport {
    BenchmarkReport {
        name: benchmark.name.clone(),
        expected_compute_units: benchmark.expected_compute_units,
        actual_compute_units: 0,
        delta_compute_units: 0,
        error: Some(error_text(message)),
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

fn error_text(message: &str) -> String {
    message
        .lines()
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| message.to_owned())
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "unknown panic".to_owned())
}
