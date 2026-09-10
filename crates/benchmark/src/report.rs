use std::{
    error::Error,
    fmt::Write as _,
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use crate::model::{BenchmarkReport, Report, ResultStatus, RunStatus, StepStatus};

pub(crate) fn write(output_dir: &Path, report: &Report) -> Result<(), Box<dyn Error>> {
    let json = serde_json::to_vec_pretty(report)?;
    write_atomic(
        output_dir.join("report.json"),
        &[json.as_slice(), b"\n"].concat(),
    )?;
    write_atomic(
        output_dir.join("report.md"),
        render_markdown(report).as_bytes(),
    )?;
    Ok(())
}

pub(crate) fn print_terminal(report: &Report, output_dir: &Path) {
    println!("sBPF Linker Benchmark Report");
    println!("Linker: {}", report.linker.version.replace('\n', " / "));
    println!();
    println!(
        "{:<18} {:<24} {:>11} {:>11} {:>8} {:>8}  Result",
        "Program", "Benchmark", "Expected CU", "Actual CU", "Delta", "Correct"
    );
    for program in &report.programs {
        for benchmark in &program.benchmarks {
            println!(
                "{:<18} {:<24} {:>11} {:>11} {:>8} {:>8}  {}",
                program.name,
                benchmark.name,
                optional_u64(benchmark.expected_compute_units),
                optional_u64(benchmark.actual_compute_units),
                optional_delta(benchmark.delta_compute_units),
                correctness_label(benchmark),
                benchmark.status.label(),
            );
        }
    }
    let total = report
        .programs
        .iter()
        .map(|program| program.benchmarks.len())
        .sum::<usize>();
    let passing = report
        .programs
        .iter()
        .flat_map(|program| &program.benchmarks)
        .filter(|benchmark| benchmark.status.passed())
        .count();
    println!();
    println!("Benchmarks passed: {passing}/{total}");
    println!(
        "Result: {}",
        if matches!(report.status, RunStatus::Passed) {
            "PASSED"
        } else {
            "FAILED"
        }
    );
    println!("JSON: {}", output_dir.join("report.json").display());
    println!("Markdown: {}", output_dir.join("report.md").display());
}

fn render_markdown(report: &Report) -> String {
    let mut output = String::new();
    writeln!(output, "# sBPF Linker Benchmark Report\n").unwrap();
    writeln!(
        output,
        "**Result: {}**\n",
        if matches!(report.status, RunStatus::Passed) {
            "PASSED"
        } else {
            "FAILED"
        }
    )
    .unwrap();
    writeln!(output, "## Environment\n").unwrap();
    writeln!(output, "| Property | Value |\n|---|---|").unwrap();
    writeln!(output, "| Linker | `{}` |", markdown(&report.linker.path)).unwrap();
    writeln!(
        output,
        "| Manifest schema | `{}` |",
        report.manifest_schema_version
    )
    .unwrap();
    writeln!(
        output,
        "| Linker version | `{}` |",
        markdown(&report.linker.version.replace('\n', " / "))
    )
    .unwrap();
    writeln!(
        output,
        "| Architecture | `{}` |",
        report.environment.architecture
    )
    .unwrap();
    writeln!(
        output,
        "| Rust | `{}` |",
        markdown(&report.environment.rustc)
    )
    .unwrap();
    writeln!(
        output,
        "| Cargo | `{}` |",
        markdown(&report.environment.cargo)
    )
    .unwrap();
    writeln!(output, "| Mollusk | `{}` |\n", report.environment.mollusk).unwrap();
    writeln!(output, "## Results\n").unwrap();
    writeln!(output, "| Program | Benchmark | Correctness | Expected CU | Actual CU | Delta CU | Delta % | Result |\n|---|---|:---:|---:|---:|---:|---:|---|").unwrap();
    for program in &report.programs {
        for benchmark in &program.benchmarks {
            writeln!(
                output,
                "| `{}` | `{}` | {} | {} | {} | {} | {} | {} |",
                markdown(&program.name),
                markdown(&benchmark.name),
                correctness_label(benchmark),
                optional_u64(benchmark.expected_compute_units),
                optional_u64(benchmark.actual_compute_units),
                optional_delta(benchmark.delta_compute_units),
                optional_percentage(benchmark.delta_percent),
                benchmark.status.label(),
            )
            .unwrap();
        }
    }
    output.push('\n');
    writeln!(output, "## Programs\n").unwrap();
    writeln!(
        output,
        "| Program | Build | Execution | ELF bytes |\n|---|:---:|:---:|---:|"
    )
    .unwrap();
    for program in &report.programs {
        writeln!(
            output,
            "| `{}` | {} | {} | {} |",
            markdown(&program.name),
            step_label(program.build.status),
            step_label(program.execution.status),
            program
                .elf
                .as_ref()
                .map(|elf| elf.size_bytes.to_string())
                .unwrap_or_else(|| "—".to_owned()),
        )
        .unwrap();
    }
    let issues = report
        .programs
        .iter()
        .flat_map(|program| {
            program
                .issues
                .iter()
                .map(move |issue| format!("`{}`: {}", markdown(&program.name), markdown(issue)))
        })
        .collect::<Vec<_>>();
    if !issues.is_empty() {
        writeln!(output, "\n## Issues\n").unwrap();
        for issue in issues {
            writeln!(output, "- {issue}").unwrap();
        }
    }
    render_baseline_actions(&mut output, report);
    output
}

fn render_baseline_actions(output: &mut String, report: &Report) {
    let unbaselined = report
        .programs
        .iter()
        .flat_map(|program| {
            program.benchmarks.iter().filter_map(move |benchmark| {
                (benchmark.status == ResultStatus::Unbaselined).then_some((program, benchmark))
            })
        })
        .collect::<Vec<_>>();
    if !unbaselined.is_empty() {
        writeln!(output, "\n## Baseline updates required\n").unwrap();
        for (program, benchmark) in unbaselined {
            writeln!(
                output,
                "- `{}/{}`: set `expected_compute_units` to `{}` after review.",
                markdown(&program.name),
                markdown(&benchmark.name),
                benchmark
                    .actual_compute_units
                    .expect("unbaselined result has actual CUs")
            )
            .unwrap();
        }
    }
    let unregistered = report
        .programs
        .iter()
        .flat_map(|program| {
            program.benchmarks.iter().filter_map(move |benchmark| {
                (benchmark.status == ResultStatus::Unregistered).then_some((program, benchmark))
            })
        })
        .collect::<Vec<_>>();
    if !unregistered.is_empty() {
        writeln!(output, "\n## Unregistered benchmarks\n").unwrap();
        for (program, benchmark) in unregistered {
            writeln!(
                output,
                "- Add `{}/{}` to `programs/manifest.json` with `\"expected_compute_units\": null` (measured `{}` CU).",
                markdown(&program.name),
                markdown(&benchmark.name),
                benchmark
                    .actual_compute_units
                    .expect("unregistered result has actual CUs")
            )
            .unwrap();
        }
    }
}

fn correctness_label(benchmark: &BenchmarkReport) -> &'static str {
    match &benchmark.correctness {
        Some(correctness) if correctness.passed => "PASS",
        Some(_) => "FAIL",
        None => "—",
    }
}

fn step_label(status: StepStatus) -> &'static str {
    match status {
        StepStatus::Passed => "PASS",
        StepStatus::Failed => "FAIL",
        StepStatus::Skipped => "—",
    }
}

fn optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "—".to_owned(), |value| value.to_string())
}

fn optional_delta(value: Option<i64>) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |value| {
            if value > 0 {
                format!("+{value}")
            } else {
                value.to_string()
            }
        },
    )
}

fn optional_percentage(value: Option<f64>) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |value| {
            if value > 0.0 {
                format!("+{value:.2}%")
            } else {
                format!("{value:.2}%")
            }
        },
    )
}

fn markdown(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn write_atomic(path: PathBuf, contents: &[u8]) -> io::Result<()> {
    let temp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_path)?;
    if let Err(error) = file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temp_path, &path))
    {
        let _ = fs::remove_file(temp_path);
        return Err(error);
    }
    Ok(())
}
