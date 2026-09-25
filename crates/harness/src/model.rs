pub(crate) struct Report {
    pub(crate) compiler: Vec<(String, String)>,
    pub(crate) programs: Vec<ProgramReport>,
}

#[derive(Debug)]
pub(crate) struct BenchmarkReport {
    pub(crate) name: String,
    pub(crate) expected_compute_units: u64,
    pub(crate) actual_compute_units: u64,
    pub(crate) delta_compute_units: i64,
    pub(crate) error: Option<String>,
}

impl BenchmarkReport {
    pub(crate) fn has_issue(&self) -> bool {
        self.error.is_some() || self.delta_compute_units != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TestOutcome {
    Passed,
    Failed,
    Ignored,
}

impl TestOutcome {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Passed => "Pass",
            Self::Failed => "Failed",
            Self::Ignored => "Ignored",
        }
    }
}

#[derive(Debug)]
pub(crate) struct TestReport {
    pub(crate) name: String,
    pub(crate) outcome: TestOutcome,
    pub(crate) error: Option<String>,
}

impl TestReport {
    pub(crate) fn is_failed(&self) -> bool {
        self.outcome == TestOutcome::Failed
    }
}

pub(crate) struct ProgramReport {
    pub(crate) name: String,
    pub(crate) benchmarks: Vec<BenchmarkReport>,
    pub(crate) tests: Vec<TestReport>,
}
