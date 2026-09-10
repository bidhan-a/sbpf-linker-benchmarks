mod manifest;
mod model;
mod report;
mod runner;

use std::{
    env,
    error::Error,
    ffi::{OsStr, OsString},
    fmt, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use mollusk_svm::result::Config;
use serde::Serialize;

use crate::model::{Environment, Linker, ProgramReport, Report, RunStatus};

const ARCHITECTURE: &str = "v0";
const DEFAULT_LINKER: &str = "sbpf-linker";
const REPORT_SCHEMA_VERSION: u32 = 2;

pub use mollusk_svm::{Mollusk, result::Check};
pub use solana_account::Account;
pub use solana_instruction::Instruction;
pub use solana_pubkey::Pubkey;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CorrectnessResult {
    pub passed: bool,
    pub actual: String,
    pub details: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkResult {
    pub name: String,
    pub correctness: CorrectnessResult,
    pub compute_units: u64,
}

pub type BenchmarkRunner = fn(&Mollusk, Pubkey) -> Result<Vec<BenchmarkResult>, BenchmarkError>;

#[derive(Clone, Copy)]
pub struct ProgramRegistration {
    pub(crate) name: &'static str,
    pub(crate) runner: BenchmarkRunner,
}

impl ProgramRegistration {
    pub const fn new(name: &'static str, runner: BenchmarkRunner) -> Self {
        Self { name, runner }
    }
}

pub fn run(root: &Path, registrations: &[ProgramRegistration]) -> ExitCode {
    match run_inner(root, registrations) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(root: &Path, registrations: &[ProgramRegistration]) -> Result<bool, Box<dyn Error>> {
    let Some(options) = Options::parse(root)? else {
        return Ok(true);
    };
    let linker = resolve_executable(&options.linker, "linker")?;
    let manifest_path = canonical_file(&options.manifest, "manifest")?;
    let manifest = manifest::load(&manifest_path)?;
    let packages = manifest::load_packages(root)?;
    manifest::validate(&manifest, &packages)?;
    let selected_programs = manifest::select(&manifest, options.program.as_deref())?;
    let linker_version = command_version(linker.as_os_str())?;
    let output_dir = create_output_dir(root, options.output)?;
    let started_at_unix_seconds = unix_timestamp()?;
    let environment = Environment {
        architecture: ARCHITECTURE.to_owned(),
        cargo: command_version(OsStr::new("cargo")).unwrap_or_else(|_| "unknown".to_owned()),
        mollusk: "0.15.1".to_owned(),
        rustc: command_version(OsStr::new("rustc")).unwrap_or_else(|_| "unknown".to_owned()),
    };

    let programs = runner::run_programs(
        root,
        &output_dir,
        &linker,
        &selected_programs,
        &packages,
        registrations,
    )?;

    let passed = programs.iter().all(ProgramReport::passed);
    let report = Report {
        schema_version: REPORT_SCHEMA_VERSION,
        status: if passed {
            RunStatus::Passed
        } else {
            RunStatus::Failed
        },
        started_at_unix_seconds,
        manifest: manifest_path.display().to_string(),
        manifest_schema_version: manifest.schema_version(),
        linker: Linker {
            path: linker.display().to_string(),
            version: linker_version,
        },
        environment,
        programs,
    };
    report::write(&output_dir, &report)?;
    report::print_terminal(&report, &output_dir);
    Ok(passed)
}

#[derive(Debug)]
struct Options {
    linker: PathBuf,
    manifest: PathBuf,
    output: Option<PathBuf>,
    program: Option<String>,
}

impl Options {
    fn parse(root: &Path) -> Result<Option<Self>, Box<dyn Error>> {
        let mut args = env::args_os().skip(1);
        match args.next() {
            Some(command) if command == "run" => {}
            Some(command) if command == "--help" || command == "-h" => {
                print_usage();
                return Ok(None);
            }
            Some(command) => {
                return Err(invalid_input(format!(
                    "unknown command `{}`; expected `run`",
                    command.to_string_lossy()
                )));
            }
            None => {}
        }

        let mut linker = PathBuf::from(DEFAULT_LINKER);
        let mut manifest = root.join("programs/manifest.json");
        let mut output = None;
        let mut program = None;
        while let Some(argument) = args.next() {
            let value = match argument.to_str() {
                Some("--linker" | "--manifest" | "--output" | "--program") => {
                    args.next().ok_or_else(|| {
                        invalid_input(format!("missing value for {}", argument.to_string_lossy()))
                    })?
                }
                Some("--help" | "-h") => {
                    print_usage();
                    return Ok(None);
                }
                _ => {
                    return Err(invalid_input(format!(
                        "unknown argument `{}`",
                        argument.to_string_lossy()
                    )));
                }
            };
            match argument.to_str().expect("matched UTF-8 argument") {
                "--linker" => linker = value.into(),
                "--manifest" => manifest = value.into(),
                "--output" => output = Some(value.into()),
                "--program" => {
                    program = Some(
                        value
                            .into_string()
                            .map_err(|_| invalid_input("program name must be UTF-8"))?,
                    )
                }
                _ => unreachable!(),
            }
        }

        Ok(Some(Self {
            linker,
            manifest,
            output,
            program,
        }))
    }
}

fn print_usage() {
    println!(
        "Usage: sbpf-linker-benchmarks run [--linker PATH] [--manifest PATH] [--output PATH] [--program NAME]"
    );
}

fn command_version(command: &OsStr) -> Result<String, Box<dyn Error>> {
    let output = Command::new(command).arg("--version").output()?;
    if !output.status.success() {
        return Err(command_error(
            &format!("{} --version", command.to_string_lossy()),
            &output,
        ));
    }
    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout.trim().to_owned())
}

fn create_output_dir(root: &Path, requested: Option<PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    let (path, replace) = requested.map_or_else(
        || (root.join("target/benchmark-results"), true),
        |path| (path, false),
    );
    if replace && path.exists() {
        fs::remove_dir_all(&path)?;
    } else if path.exists() {
        return Err(invalid_input(format!(
            "output path already exists: {}",
            path.display()
        )));
    }
    fs::create_dir_all(&path)?;
    Ok(fs::canonicalize(path)?)
}

fn canonical_file(path: &Path, description: &str) -> Result<PathBuf, Box<dyn Error>> {
    let path = fs::canonicalize(path).map_err(|error| {
        invalid_input(format!(
            "failed to resolve {description} `{}`: {error}",
            path.display()
        ))
    })?;
    if !path.is_file() {
        return Err(invalid_input(format!(
            "{description} is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn resolve_executable(command: &Path, description: &str) -> Result<PathBuf, Box<dyn Error>> {
    if command.components().count() == 1 {
        Ok(command.to_owned())
    } else {
        canonical_file(command, description)
    }
}

fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn unix_timestamp() -> Result<u64, Box<dyn Error>> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn command_error(command: &str, output: &Output) -> Box<dyn Error> {
    invalid_data(format!(
        "`{command}` failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}

fn invalid_data(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}

pub struct Benchmark {
    name: String,
}

impl Benchmark {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn run(
        self,
        mollusk: &Mollusk,
        instruction: &Instruction,
        accounts: &[(Pubkey, Account)],
        checks: &[Check<'_>],
    ) -> Result<BenchmarkResult, BenchmarkError> {
        validate_name(&self.name).map_err(BenchmarkError::InvalidBenchmarkName)?;

        let result = mollusk.process_instruction(instruction, accounts);
        let passed = result.run_checks(
            checks,
            &Config {
                panic: false,
                verbose: true,
            },
            mollusk,
        );

        Ok(BenchmarkResult {
            name: self.name,
            correctness: CorrectnessResult {
                passed,
                actual: format!("{:?}", result.program_result),
                details: (!passed).then(|| "one or more Mollusk checks failed".to_owned()),
            },
            compute_units: result.compute_units_consumed,
        })
    }
}

#[derive(Debug)]
pub enum BenchmarkError {
    InvalidBenchmarkName(String),
    Setup(String),
}

impl BenchmarkError {
    pub fn setup(message: impl Into<String>) -> Self {
        Self::Setup(message.into())
    }
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBenchmarkName(message) => formatter.write_str(message),
            Self::Setup(message) => write!(formatter, "benchmark setup failed: {message}"),
        }
    }
}

impl Error for BenchmarkError {}

pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!(
            "benchmark name `{name}` must contain only ASCII letters, digits, `-`, or `_`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_name;

    #[test]
    fn validates_benchmark_names() {
        assert!(validate_name("bench-function_pointer-1").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("invalid/name").is_err());
    }
}
