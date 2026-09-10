use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

use crate::{cargo, command_error, invalid_data, invalid_input, validate_name};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    schema_version: u32,
    pub(crate) programs: Vec<ExpectedProgram>,
}

impl Manifest {
    pub(crate) fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedProgram {
    pub(crate) name: String,
    pub(crate) benchmarks: Vec<ExpectedBenchmark>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExpectedBenchmark {
    pub(crate) name: String,
    expected_compute_units: NullableComputeUnits,
}

impl ExpectedBenchmark {
    pub(crate) fn expected_compute_units(&self) -> Option<u64> {
        self.expected_compute_units.value()
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
enum NullableComputeUnits {
    Value(u64),
    Null(()),
}

impl NullableComputeUnits {
    fn value(self) -> Option<u64> {
        match self {
            Self::Value(value) => Some(value),
            Self::Null(()) => None,
        }
    }
}

#[cfg(test)]
pub(crate) fn expected_benchmark(
    name: &str,
    expected_compute_units: Option<u64>,
) -> ExpectedBenchmark {
    ExpectedBenchmark {
        name: name.to_owned(),
        expected_compute_units: expected_compute_units
            .map_or(NullableComputeUnits::Null(()), NullableComputeUnits::Value),
    }
}

pub(crate) fn load(path: &Path) -> Result<Manifest, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(invalid_data(format!(
            "unsupported manifest schema version {}; expected {SCHEMA_VERSION}",
            manifest.schema_version
        )));
    }
    Ok(manifest)
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    name: String,
    manifest_path: PathBuf,
    targets: Vec<CargoTarget>,
}

#[derive(Debug, Deserialize)]
struct CargoTarget {
    name: String,
    kind: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct ProgramPackage {
    pub(crate) target_name: String,
}

pub(crate) fn load_packages(
    root: &Path,
) -> Result<BTreeMap<String, ProgramPackage>, Box<dyn Error>> {
    let output = Command::new(cargo())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(command_error("cargo metadata", &output));
    }
    let metadata: CargoMetadata = serde_json::from_slice(&output.stdout)?;
    let programs_dir = root.join("programs");
    let mut packages = BTreeMap::new();
    for package in metadata.packages {
        let Some(package_dir) = package.manifest_path.parent() else {
            continue;
        };
        if package_dir.parent() != Some(programs_dir.as_path()) {
            continue;
        }
        let library_targets = package
            .targets
            .into_iter()
            .filter(|target| target.kind.iter().any(|kind| kind == "lib"))
            .collect::<Vec<_>>();
        if library_targets.len() != 1 {
            return Err(invalid_data(format!(
                "program `{}` must have exactly one library target",
                package.name
            )));
        }
        let target_name = library_targets.into_iter().next().expect("one target").name;
        if packages
            .insert(package.name.clone(), ProgramPackage { target_name })
            .is_some()
        {
            return Err(invalid_data(format!(
                "duplicate Cargo program package `{}`",
                package.name
            )));
        }
    }
    Ok(packages)
}

pub(crate) fn validate(
    manifest: &Manifest,
    packages: &BTreeMap<String, ProgramPackage>,
) -> Result<(), Box<dyn Error>> {
    if manifest.programs.is_empty() {
        return Err(invalid_data("manifest contains no programs"));
    }
    let mut program_names = BTreeSet::new();
    for program in &manifest.programs {
        validate_name(&program.name).map_err(invalid_data)?;
        if !program_names.insert(program.name.as_str()) {
            return Err(invalid_data(format!(
                "duplicate manifest program `{}`",
                program.name
            )));
        }
        let package = packages.get(&program.name).ok_or_else(|| {
            invalid_data(format!(
                "manifest program `{}` is not a package under `programs/`",
                program.name
            ))
        })?;
        let expected_target = program.name.replace('-', "_");
        if package.target_name != expected_target {
            return Err(invalid_data(format!(
                "program `{}` must use the default library target name `{expected_target}`",
                program.name
            )));
        }
        if program.benchmarks.is_empty() {
            return Err(invalid_data(format!(
                "manifest program `{}` contains no benchmarks",
                program.name
            )));
        }
        let mut benchmark_names = BTreeSet::new();
        for benchmark in &program.benchmarks {
            validate_name(&benchmark.name).map_err(invalid_data)?;
            if !benchmark_names.insert(benchmark.name.as_str()) {
                return Err(invalid_data(format!(
                    "duplicate benchmark `{}/{}`",
                    program.name, benchmark.name
                )));
            }
        }
    }
    let manifest_names = manifest
        .programs
        .iter()
        .map(|program| program.name.as_str())
        .collect::<BTreeSet<_>>();
    let package_names = packages.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if manifest_names != package_names {
        let missing = package_names
            .difference(&manifest_names)
            .copied()
            .collect::<Vec<_>>();
        return Err(invalid_data(format!(
            "programs missing from manifest: {}",
            missing.join(", ")
        )));
    }
    Ok(())
}

pub(crate) fn select<'a>(
    manifest: &'a Manifest,
    selected: Option<&str>,
) -> Result<Vec<(usize, &'a ExpectedProgram)>, Box<dyn Error>> {
    match selected {
        Some(name) => manifest
            .programs
            .iter()
            .enumerate()
            .find(|(_, program)| program.name == name)
            .map(|program| vec![program])
            .ok_or_else(|| invalid_input(format!("program `{name}` is not in the manifest"))),
        None => Ok(manifest.programs.iter().enumerate().collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::Manifest;

    #[test]
    fn requires_expected_compute_units_field() {
        let manifest = r#"{
            "schema_version": 1,
            "programs": [{
                "name": "program",
                "benchmarks": [{"name": "benchmark"}]
            }]
        }"#;
        assert!(serde_json::from_str::<Manifest>(manifest).is_err());
    }

    #[test]
    fn accepts_explicit_null_compute_units() {
        let manifest = r#"{
            "schema_version": 1,
            "programs": [{
                "name": "program",
                "benchmarks": [{
                    "name": "benchmark",
                    "expected_compute_units": null
                }]
            }]
        }"#;
        let manifest = serde_json::from_str::<Manifest>(manifest).unwrap();
        assert!(
            manifest.programs[0].benchmarks[0]
                .expected_compute_units()
                .is_none()
        );
    }
}
