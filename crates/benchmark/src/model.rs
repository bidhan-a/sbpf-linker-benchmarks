use serde::Serialize;

use crate::CorrectnessResult;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunStatus {
    Passed,
    Failed,
}

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) schema_version: u32,
    pub(crate) status: RunStatus,
    pub(crate) started_at_unix_seconds: u64,
    pub(crate) manifest: String,
    pub(crate) manifest_schema_version: u32,
    pub(crate) linker: Linker,
    pub(crate) environment: Environment,
    pub(crate) programs: Vec<ProgramReport>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Linker {
    pub(crate) path: String,
    pub(crate) version: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Environment {
    pub(crate) architecture: String,
    pub(crate) cargo: String,
    pub(crate) mollusk: String,
    pub(crate) rustc: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StepStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Serialize)]
pub(crate) struct StepReport {
    pub(crate) status: StepStatus,
    pub(crate) log: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ElfReport {
    pub(crate) path: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResultStatus {
    Unchanged,
    Improved,
    PerformanceRegression,
    CorrectnessFailed,
    Unbaselined,
    Unregistered,
    MissingResult,
}

impl ResultStatus {
    pub(crate) fn passed(self) -> bool {
        matches!(self, Self::Unchanged | Self::Improved)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "UNCHANGED",
            Self::Improved => "IMPROVED",
            Self::PerformanceRegression => "REGRESSION",
            Self::CorrectnessFailed => "CORRECTNESS FAILURE",
            Self::Unbaselined => "UNBASELINED",
            Self::Unregistered => "UNREGISTERED",
            Self::MissingResult => "MISSING RESULT",
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct BenchmarkReport {
    pub(crate) name: String,
    pub(crate) expected_compute_units: Option<u64>,
    pub(crate) actual_compute_units: Option<u64>,
    pub(crate) delta_compute_units: Option<i64>,
    pub(crate) delta_percent: Option<f64>,
    pub(crate) correctness: Option<CorrectnessResult>,
    pub(crate) status: ResultStatus,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProgramReport {
    pub(crate) name: String,
    pub(crate) build: StepReport,
    pub(crate) execution: StepReport,
    pub(crate) elf: Option<ElfReport>,
    pub(crate) benchmarks: Vec<BenchmarkReport>,
    pub(crate) issues: Vec<String>,
}

impl ProgramReport {
    pub(crate) fn passed(&self) -> bool {
        matches!(self.build.status, StepStatus::Passed)
            && matches!(self.execution.status, StepStatus::Passed)
            && self
                .benchmarks
                .iter()
                .all(|benchmark| benchmark.status.passed())
            && self.issues.is_empty()
    }
}
