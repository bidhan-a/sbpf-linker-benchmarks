use crate::model::Report;

pub(crate) fn print_terminal(report: &Report, artifacts_dir: &str) {
    println!();
    println!("Benchmark Report");
    println!("Compiler: {}", report.compiler.replace('\n', " / "));
    println!();
    println!(
        "{:<18} {:<24} {:>11} {:>11} {:>8}",
        "Program", "Benchmark", "Expected CU", "Actual CU", "Diff"
    );
    for program in &report.programs {
        for benchmark in &program.benchmarks {
            println!(
                "{:<18} {:<24} {:>11} {:>11} {:>8}",
                program.name,
                benchmark.name,
                benchmark.expected_compute_units,
                benchmark.actual_compute_units,
                optional_delta(benchmark.delta_compute_units),
            );
        }
    }
    println!();
    println!("Run complete");
    println!("Artifacts generated at: {artifacts_dir}");
}

fn optional_delta(value: i64) -> String {
    if value > 0 {
        format!("+{value}")
    } else {
        value.to_string()
    }
}
