use crossterm::style::Stylize;
use indexmap::IndexMap;

use crate::{
    config::{RepoConfig, Selector},
    error::RtResult,
    venv::{Suite, environment_path, profile_path, select_test_environments},
};

pub fn run(suites: &IndexMap<String, Suite>, repo: &RepoConfig, selector: String) -> RtResult<()> {
    for environment in select_test_environments(suites, Selector::Pattern(selector))? {
        println!(
            "{} {}",
            "Test environment".bold().cyan(),
            environment.hash.as_str().bold().blue()
        );
        println!("  suite: {}", environment.suite);
        println!("  name: {}", environment.name);
        println!("  integration: {}", environment.integration_name);
        println!("  python: {}", environment.python);
        println!(
            "  path: {}",
            environment_path(&repo.cache_root, &environment.hash).display()
        );
        println!("  lockfile: {}", environment.lockfile);
        if !environment.services.is_empty() {
            println!("  services: {}", environment.services.join(", "));
        }
        println!("  direct dependencies:");
        for dependency in &environment.direct_dependencies {
            println!("    {dependency}");
        }
        println!("  runs:");
        for run in &environment.runs {
            println!("    {}", run.id.as_str().cyan());
            println!(
                "      profile: {}",
                profile_path(&repo.cache_root, &environment.hash, run.index).display()
            );
            println!("      command: {}", run.command);
            for (key, value) in &run.env {
                println!("      {key}={value}");
            }
        }
    }
    Ok(())
}
