use std::collections::HashMap;
use crate::graph::Target;
use crate::forge_root_config::PlatformConfig;

#[derive(Debug, Clone)]
pub struct Platform {
    pub name: String,
    pub target: Target,
    pub cpu: Option<String>,
    pub constraint_values: Vec<String>,
}

pub struct PlatformRegistry {
    platforms: HashMap<String, Platform>,
}

impl PlatformRegistry {
    pub fn new() -> Self {
        Self {
            platforms: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: &str, config: &PlatformConfig) {
        let triple = format!("{}-unknown-{}-{}", config.arch, config.os, config.abi);
        let target = Target::new(name, &triple);
        
        self.platforms.insert(name.to_string(), Platform {
            name: name.to_string(),
            target,
            cpu: config.cpu.clone(),
            constraint_values: config.constraint_values.clone(),
        });
    }

    pub fn get(&self, name: &str) -> Option<&Platform> {
        self.platforms.get(name)
    }

    pub fn list_names(&self) -> Vec<String> {
        self.platforms.keys().cloned().collect()
    }
}
