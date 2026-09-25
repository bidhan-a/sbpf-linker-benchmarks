use comfy_table::{
    Cell, CellAlignment, ContentArrangement, ContentLineStyle, LineStyle, Table, presets::UTF8_FULL,
};

use crate::model::{Report, TestOutcome};

fn styled_table() -> Table {
    let mut table = Table::new();
    table.load_style(
        UTF8_FULL
            .header_lines(ContentLineStyle::new('│', '│', '│'))
            .content_lines(ContentLineStyle::new('│', '│', '│'))
            .row_separator(LineStyle::new('├', '─', '┼', '┤')),
    );
    table
}

pub(crate) fn print_terminal(report: &Report, saved_artifacts: bool, artifacts_dir: &str) {
    println!();
    println!("{}", "─".repeat(33));
    println!("\tBenchmark Report");
    println!("{}", "─".repeat(33));

    println!();
    println!("Compiler");
    println!("{}", compiler_table(report));

    println!();
    let benchmarks = benchmark_table(report);
    println!("Benchmarks");
    println!("{benchmarks}");

    println!();
    let tests = test_table(report);
    println!("Tests");
    println!("{tests}");

    println!();
    println!("Summary");
    println!("{}", summary_table(report));

    println!();
    if saved_artifacts {
        println!("[WARN] Issues detected! Artifacts generated at: {artifacts_dir}.");
    }
}

fn compiler_table(report: &Report) -> Table {
    let mut table = styled_table();
    table
        .set_header(vec!["Toolchain", "Version"])
        .set_content_arrangement(ContentArrangement::Dynamic);
    for (label, version) in &report.compiler {
        table.add_row(vec![Cell::new(label), Cell::new(version)]);
    }
    table
}

fn summary_table(report: &Report) -> Table {
    let (changed, unchanged) = report
        .programs
        .iter()
        .flat_map(|program| &program.benchmarks)
        .fold((0, 0), |(changed, unchanged), benchmark| {
            if benchmark.has_issue() {
                (changed + 1, unchanged)
            } else {
                (changed, unchanged + 1)
            }
        });
    let (passed, failed) = report
        .programs
        .iter()
        .flat_map(|program| &program.tests)
        .fold((0, 0), |(passed, failed), test| {
            if test.is_failed() {
                (passed, failed + 1)
            } else if test.outcome == TestOutcome::Passed {
                (passed + 1, failed)
            } else {
                (passed, failed)
            }
        });
    let mut table = styled_table();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.add_row(vec![
        Cell::new("Benchmarks"),
        Cell::new(format!("{unchanged} unchanged, {changed} changed")),
    ]);
    table.add_row(vec![
        Cell::new("Tests"),
        Cell::new(format!("{passed} passed, {failed} failed")),
    ]);
    table
}

fn benchmark_table(report: &Report) -> Table {
    let mut table = styled_table();
    table
        .set_header(vec![
            "Program",
            "Benchmark",
            "Expected CU",
            "Actual CU",
            "Delta",
            "Error",
        ])
        .set_content_arrangement(ContentArrangement::Dynamic);
    for program in &report.programs {
        for benchmark in &program.benchmarks {
            let measured = benchmark.error.is_none();
            table.add_row(vec![
                Cell::new(&program.name),
                Cell::new(&benchmark.name),
                Cell::new(benchmark.expected_compute_units).set_alignment(CellAlignment::Right),
                if measured {
                    Cell::new(benchmark.actual_compute_units).set_alignment(CellAlignment::Right)
                } else {
                    Cell::new("-").set_alignment(CellAlignment::Right)
                },
                if measured {
                    Cell::new(get_delta(benchmark.delta_compute_units))
                        .set_alignment(CellAlignment::Right)
                } else {
                    Cell::new("-").set_alignment(CellAlignment::Right)
                },
                Cell::new(benchmark.error.as_deref().unwrap_or("-")),
            ]);
        }
    }
    table
}

fn test_table(report: &Report) -> Table {
    let mut table = styled_table();
    table
        .set_header(vec!["Program", "Test", "Result", "Error"])
        .set_content_arrangement(ContentArrangement::Dynamic);
    for program in &report.programs {
        for test in &program.tests {
            table.add_row(vec![
                Cell::new(&program.name),
                Cell::new(&test.name),
                Cell::new(test.outcome.label()),
                Cell::new(test.error.as_deref().unwrap_or("-")),
            ]);
        }
    }
    table
}

fn get_delta(value: i64) -> String {
    if value > 0 {
        format!("+{value}")
    } else {
        value.to_string()
    }
}
