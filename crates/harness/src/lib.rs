mod compiler;

pub use compiler::Compiler;

mod logging;
mod manifest;
mod model;
mod report;
mod runner;

use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    process::Output,
    time::{SystemTime, UNIX_EPOCH},
};

pub use mollusk_svm::Mollusk;
pub use mollusk_svm::result::Check;
pub use solana_account::Account;
pub use solana_instruction::Instruction;
pub use solana_pubkey::Pubkey;

/// Environment variable which contains the absolute path of the program ELF to load
pub(crate) const SBPF_PROGRAM_ELF_ENV: &str = "SBPF_PROGRAM_ELF";

/// Returns the path of the current program's ELF.
pub fn program_elf() -> String {
    let mut path = match env::var_os(SBPF_PROGRAM_ELF_ENV) {
        Some(path) => PathBuf::from(path),
        // Fallback to `cargo-build-sbpf` elf path.
        None => {
            let compiler = compiler::Compiler::CargoBuildSbpf;
            let package_dir =
                env::current_dir().expect("test working directory is the package directory");
            let target = package_dir
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.replace('-', "_"))
                .expect("test working directory is the package directory");
            let root = package_dir
                .parent()
                .and_then(Path::parent)
                .expect("package directory is inside the workspace `programs/` directory");
            compiler.elf_path(root, &target)
        }
    };
    path.set_extension("");
    path.into_os_string()
        .into_string()
        .unwrap_or_else(|path| path.to_string_lossy().into_owned())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BenchmarkResult {
    pub name: String,
    pub compute_units: u64,
    pub error: Option<String>,
}

pub type BenchmarkRunner = fn(&Mollusk, Pubkey) -> Result<Vec<BenchmarkResult>, BenchmarkError>;

#[derive(Clone, Copy, Debug)]
pub struct ProgramRegistration {
    pub(crate) name: &'static str,
    pub(crate) runner: BenchmarkRunner,
}

impl ProgramRegistration {
    pub const fn new(name: &'static str, runner: BenchmarkRunner) -> Self {
        Self { name, runner }
    }
}

pub fn run(
    root: &Path,
    compiler: Compiler,
    registrations: &[ProgramRegistration],
) -> Result<(), Box<dyn Error>> {
    logging::init();
    let manifest = manifest::load(&root.join("programs/manifest.json"))?;
    let packages = manifest::load_packages(root)?;
    manifest::validate(&manifest, &packages)?;
    let compiler_version = compiler.version()?;
    let run_timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or_default())
        .unwrap_or_default();

    let (report, saved_artifacts) = {
        let (programs, saved_artifacts) = runner::run_programs(
            root,
            compiler,
            registrations,
            &manifest,
            &packages,
            run_timestamp,
        )?;

        (
            model::Report {
                compiler: compiler_version,
                programs,
            },
            saved_artifacts,
        )
    };
    let artifacts_dir = Path::new("/tmp")
        .join(compiler.tool_name())
        .join(run_timestamp.to_string());
    report::print_terminal(
        &report,
        saved_artifacts,
        &artifacts_dir.display().to_string(),
    );
    Ok(())
}

fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn command_error(command: &str, output: &Output) -> Box<dyn Error> {
    invalid_data(format!(
        "`{command}` failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
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
    ) -> Result<BenchmarkResult, BenchmarkError> {
        validate_name(&self.name).map_err(BenchmarkError::InvalidBenchmarkName)?;

        let result = mollusk.process_instruction(instruction, accounts);

        let error = result
            .raw_result
            .as_ref()
            .err()
            .map(|error| error.to_string());

        Ok(BenchmarkResult {
            name: self.name,
            compute_units: result.compute_units_consumed,
            error,
        })
    }
}

#[derive(Debug)]
pub enum BenchmarkError {
    InvalidBenchmarkName(String),
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBenchmarkName(message) => formatter.write_str(message),
        }
    }
}

impl Error for BenchmarkError {}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
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
