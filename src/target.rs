use std::path::{Path, PathBuf};
use crate::graph::target::{Target, predefined_targets};

#[derive(Debug, Clone)]
pub struct TargetDefinition {
    pub name: String,
    pub canonical_name: String,
    pub arch: String,
    pub os: String,
    pub abi: String,
}

impl TargetDefinition {
    pub fn from_target(target: Target) -> Self {
        Self {
            name: target.name.clone(),
            canonical_name: target.triple.clone(),
            arch: target.arch.clone(),
            os: target.os.clone(),
            abi: target.abi.clone(),
        }
    }

    pub fn host() -> Self {
        let host_triple = Target::host_target();
        let target = Target::new("host", &host_triple);
        Self::from_target(target)
    }
}

#[derive(Clone)]
pub struct TargetResolver {
    targets: Vec<TargetDefinition>,
}

impl TargetResolver {
    pub fn new() -> Self {
        let targets = predefined_targets()
            .into_iter()
            .map(|(name, triple)| {
                TargetDefinition::from_target(Target::new(name, triple))
            })
            .collect();
        
        Self { targets }
    }

    pub fn all_targets(&self) -> impl Iterator<Item = &TargetDefinition> {
        self.targets.iter()
    }

    pub fn resolve(&self, name: &str) -> Option<&TargetDefinition> {
        self.targets
            .iter()
            .find(|t| t.name == name || t.canonical_name == name)
    }

    pub fn get_canonical_triple(&self, name: &str) -> Option<String> {
        self.resolve(name).map(|t| t.canonical_name.clone())
    }

    pub fn get_output_directory(&self, name: &str, project_root: &Path) -> Option<PathBuf> {
        self.resolve(name).map(|t| project_root.join("forge-out").join(&t.name))
    }
}
