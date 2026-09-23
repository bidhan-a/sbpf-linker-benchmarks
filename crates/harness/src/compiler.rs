use std::{error::Error, process::Command};

use crate::{cargo, command_error, invalid_input};

pub(crate) const CARGO_BUILD_SBPF: &str = "cargo-build-sbpf";
pub(crate) const CARGO_BUILD_SBF: &str = "cargo-build-sbf";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Compiler {
    CargoBuildSbpf,
    CargoBuildSbf,
}

impl Compiler {
    pub(crate) fn from_name(name: &str) -> Result<Self, Box<dyn Error>> {
        match name {
            CARGO_BUILD_SBPF => Ok(Self::CargoBuildSbpf),
            CARGO_BUILD_SBF => Ok(Self::CargoBuildSbf),
            other => Err(invalid_input(format!(
                "unknown compiler `{other}`; expected `{CARGO_BUILD_SBPF}` or `{CARGO_BUILD_SBF}`"
            ))),
        }
    }

    pub(crate) fn subcommand(self) -> &'static str {
        match self {
            Self::CargoBuildSbpf => "build-sbpf",
            Self::CargoBuildSbf => "build-sbf",
        }
    }

    pub(crate) fn build_args(self) -> &'static [&'static str] {
        match self {
            Self::CargoBuildSbpf => &[],
            Self::CargoBuildSbf => &["--arch", "v3"], // build with arch v3
        }
    }

    pub(crate) fn target_dir(self) -> &'static str {
        match self {
            Self::CargoBuildSbpf => "target/bpfel-unknown-none/release",
            Self::CargoBuildSbf => "target/sbpfv3-solana-solana/release",
        }
    }

    pub(crate) fn elf_file_name(self, target_name: &str) -> String {
        match self {
            Self::CargoBuildSbpf => format!("lib{target_name}.so"),
            Self::CargoBuildSbf => format!("{target_name}.so"),
        }
    }

    pub(crate) fn tool_name(self) -> &'static str {
        match self {
            Self::CargoBuildSbpf => CARGO_BUILD_SBPF,
            Self::CargoBuildSbf => CARGO_BUILD_SBF,
        }
    }

    pub(crate) fn version(self) -> Result<String, Box<dyn Error>> {
        let cargo = cargo().to_string_lossy().into_owned();
        match self {
            Self::CargoBuildSbpf => {
                let build_sbpf = command_version(&cargo, &[self.subcommand(), "--version"], false)?;
                let sbpf_linker = command_version("sbpf-linker", &["--version"], true)?;
                let rustc = command_version("rustc", &["--version"], false)?;
                Ok(format!("{build_sbpf} / {sbpf_linker} / {rustc}"))
            }
            Self::CargoBuildSbf => command_version(&cargo, &[self.subcommand(), "--version"], true),
        }
    }
}

fn command_version(
    command: &str,
    args: &[&str],
    join_lines: bool,
) -> Result<String, Box<dyn Error>> {
    let description = format!("{} {}", command, args.join(" "));
    let output = Command::new(command).args(args).output()?;
    if !output.status.success() {
        return Err(command_error(&description, &output));
    }
    let version = String::from_utf8(output.stdout)?.trim().to_owned();
    if join_lines {
        Ok(version.replace('\n', " / "))
    } else {
        Ok(version)
    }
}
