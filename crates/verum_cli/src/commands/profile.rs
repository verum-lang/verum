//! Profile one ordinary compiler check. Source cost estimates and measured
//! compiler time have separate provenance in both text and machine output.

use crate::config::Manifest;
use crate::error::{CliError, Result};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path, time::Duration};
use verum_common::Text;
use verum_compiler::{
    options::CompilerOptions,
    pipeline::CompilationPipeline,
    profile_cmd::{CbgrCostModel, FunctionProfile, ProfileReport},
    session::Session,
};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum TimeUnit {
    #[value(alias = "micro", alias = "microseconds")]
    Us,
    #[value(alias = "nano", alias = "nanoseconds")]
    Ns,
}

impl TimeUnit {
    fn display(self, elapsed: Duration) -> String {
        match self {
            Self::Us => format!("{} µs", elapsed.as_micros()),
            Self::Ns => format!("{} ns", elapsed.as_nanos()),
        }
    }
}

pub struct Request<'a> {
    pub file: Option<&'a Path>,
    pub memory: bool,
    pub timing: bool,
    pub cache: bool,
    pub hot_threshold: f64,
    pub functions: &'a [Text],
    pub precision: TimeUnit,
    pub suggest: bool,
    pub json: bool,
    pub output: Option<&'a Path>,
}

#[derive(Serialize)]
struct Phase {
    name: String,
    elapsed: Duration,
}

#[derive(Serialize)]
struct CompilerTiming {
    measurement: &'static str,
    elapsed: Duration,
    phases: Vec<Phase>,
}

#[derive(Serialize)]
struct ReferenceTiers {
    functions_analyzed: u64,
    managed: u64,
    checked: u64,
    unsafe_refs: u64,
}

#[derive(Serialize)]
struct Profile {
    schema_version: u8,
    input: std::path::PathBuf,
    files_checked: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    compilation: Option<CompilerTiming>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cbgr_analysis_cache_entries: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_tiers: Option<ReferenceTiers>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_model: Option<CbgrCostModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_measurement: Option<&'static str>,
    // Retain the file report's functions map for existing consumers.
    #[serde(skip_serializing_if = "Option::is_none")]
    functions: Option<BTreeMap<String, FunctionProfile>>,
}

fn failed(error: impl std::fmt::Display) -> CliError {
    CliError::ProfilingFailed(error.to_string())
}

impl Request<'_> {
    fn estimate(
        &self,
        functions: &mut BTreeMap<String, FunctionProfile>,
        module_path: &str,
        module: &verum_ast::Module,
    ) -> Result<()> {
        for (name, cost) in ProfileReport::estimate(module, self.hot_threshold)
            .map_err(failed)?
            .functions
        {
            let qualified = if module_path.is_empty() {
                name.to_string()
            } else {
                format!("{module_path}.{name}")
            };
            if self.functions.is_empty()
                || self
                    .functions
                    .iter()
                    .any(|want| want.as_str() == name.as_str() || want.as_str() == qualified)
            {
                functions.insert(qualified, cost);
            }
        }
        Ok(())
    }

    fn collect(&self) -> Result<Profile> {
        if !(0.0..=100.0).contains(&self.hot_threshold) {
            return Err(CliError::InvalidArgument(
                "--hot-threshold must be between 0 and 100".into(),
            ));
        }

        let project = if self.file.is_none() {
            let root = Manifest::find_manifest_dir()?;
            let mut manifest = Manifest::from_file(&Manifest::manifest_path(&root))?;
            crate::feature_overrides::apply_global(&mut manifest)?;
            Some((root, manifest))
        } else {
            None
        };
        let (input, language_features) = match (&project, self.file) {
            (Some((root, manifest)), _) => (
                root.join("src"),
                crate::feature_overrides::manifest_to_features(manifest)?,
            ),
            (_, Some(path)) => (
                path.to_owned(),
                crate::feature_overrides::scratch_features()?,
            ),
            _ => unreachable!(),
        };
        let mut session = Session::new(CompilerOptions {
            input: input.clone(),
            language_features,
            hot_path_threshold: self.hot_threshold,
            ..Default::default()
        });
        let _installed = if let Some((root, manifest)) = &project {
            crate::registry::installed::register(&mut session, root, manifest)?
        } else {
            None
        };
        // Read the clock around the check itself. Report it independently of
        // phase coverage, and exclude the subsequent AST cost-model traversal.
        let start = std::time::Instant::now();
        let mut functions = self.memory.then(BTreeMap::new);
        let files_checked;
        let elapsed;
        if self.file.is_some() {
            let id = session.load_file(&input).map_err(failed)?;
            CompilationPipeline::new_check(&mut session)
                .run_check_only()
                .map_err(failed)?;
            elapsed = start.elapsed();
            if session.has_errors() {
                return Err(failed(session.format_diagnostics()));
            }
            if let Some(functions) = &mut functions {
                let module = session
                    .get_module(id)
                    .ok_or_else(|| failed("checked source module unavailable"))?;
                self.estimate(functions, "", &module)?;
            }
            files_checked = 1;
        } else {
            let mut pipeline = CompilationPipeline::new_check(&mut session);
            let result = pipeline.check_project().map_err(failed)?;
            elapsed = start.elapsed();
            if result.user_errors != 0 {
                return Err(failed(session.format_diagnostics()));
            }
            files_checked = result.files_checked;
            if let Some(functions) = &mut functions {
                for (path, module) in pipeline.checked_modules() {
                    let local =
                        pipeline
                            .session()
                            .get_source(module.file_id)
                            .is_some_and(|source| {
                                source
                                    .path
                                    .as_deref()
                                    .is_some_and(|path| path.starts_with(&input))
                            });
                    if local {
                        self.estimate(functions, path, module)?;
                    }
                }
            }
        }
        let compilation = self.timing.then(|| CompilerTiming {
            measurement: "compiler_wall_clock",
            elapsed,
            phases: session
                .get_phase_timings()
                .into_iter()
                .map(|(name, elapsed)| Phase { name, elapsed })
                .collect(),
        });
        let cbgr_analysis_cache_entries = self.cache.then(|| session.tier_cache_size());
        let reference_tiers = self.memory.then(|| {
            let tiers = session.tier_statistics();
            ReferenceTiers {
                functions_analyzed: tiers.functions_analyzed,
                managed: tiers.tier0_count,
                checked: tiers.tier1_count,
                unsafe_refs: tiers.tier2_count,
            }
        });
        Ok(Profile {
            schema_version: 1,
            reference_tiers,
            input,
            files_checked,
            compilation,
            cbgr_analysis_cache_entries,
            cost_model: self.memory.then_some(CbgrCostModel::DEFAULT),
            function_measurement: self.memory.then_some("static_cbgr_cost_model"),
            functions,
        })
    }
}

pub fn execute(request: Request<'_>) -> Result<()> {
    let report = request.collect()?;
    // Before --json, project-mode "-o json" selected JSON on stdout.
    // With a file input, "-o" has always named a file (even one named json).
    let legacy_json = request.file.is_none() && request.output == Some(Path::new("json"));
    let legacy_text = request.file.is_none() && request.output == Some(Path::new("text"));
    if let Some(path) = request.output.filter(|_| !(legacy_json || legacy_text)) {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    if request.json || legacy_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!(
        "Compiler check · {} · {} file(s)",
        report.input.display(),
        report.files_checked
    );
    if let Some(timing) = report.compilation {
        println!(
            "Elapsed compiler time: {}",
            request.precision.display(timing.elapsed)
        );
        for phase in timing.phases {
            println!(
                "  {}  {}",
                request.precision.display(phase.elapsed),
                phase.name
            );
        }
    }
    if let Some(entries) = report.cbgr_analysis_cache_entries {
        println!("Retained CBGR analysis entries: {entries}");
    }
    if let Some(tiers) = report.reference_tiers {
        println!(
            "Compiler reference tiers ({} functions analyzed): {} managed, {} checked, {} unsafe",
            tiers.functions_analyzed, tiers.managed, tiers.checked, tiers.unsafe_refs
        );
    }
    if let Some(functions) = report.functions {
        println!("Static CBGR cost estimates");
        for (name, profile) in functions {
            let marker = if request.suggest && profile.is_hot {
                " · review reference use"
            } else {
                ""
            };
            println!(
                "  {name}: {} managed references, {} ownership references, {:.1}% modeled overhead{marker}",
                profile.stats.num_cbgr_refs, profile.stats.num_ownership_refs, profile.overhead_pct
            );
        }
    }
    Ok(())
}
