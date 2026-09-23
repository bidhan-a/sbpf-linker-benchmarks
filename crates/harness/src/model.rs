pub(crate) struct Report {
    /// Compiler tool and version
    pub(crate) compiler: String,
    pub(crate) programs: Vec<ProgramReport>,
}

#[derive(Debug)]
pub(crate) struct BenchmarkReport {
    pub(crate) name: String,
    pub(crate) expected_compute_units: u64,
    pub(crate) actual_compute_units: u64,
    pub(crate) delta_compute_units: i64,
}

pub(crate) struct ProgramReport {
    pub(crate) name: String,
    pub(crate) benchmarks: Vec<BenchmarkReport>,
}
