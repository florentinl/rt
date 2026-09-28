use std::collections::HashMap;

use crossterm::style::{Attribute, Color, Stylize};

use crate::{presentation::EnvironmentPresentation, venv::TestEnvironment};

pub fn step(message: impl AsRef<str>) {
    eprintln!("{} {}", "==>".bold().cyan(), message.as_ref());
}
pub fn detail(message: impl AsRef<str>) {
    eprintln!("    {}", message.as_ref());
}
pub fn print_environment_hierarchy(
    environments: &[TestEnvironment],
    presentations: &HashMap<String, EnvironmentPresentation>,
    mut emit: impl FnMut(&str),
) {
    let mut previous_suite = None;
    for environment in environments {
        if previous_suite != Some(environment.suite.as_str()) {
            emit(&format!("{}", environment.suite.as_str().bold().yellow()));
            previous_suite = Some(&environment.suite);
        }
        emit(&format!(
            "  {} {}",
            environment.hash.as_str().bold().blue(),
            presentations[&environment.hash]
                .name
                .as_str()
                .bold()
                .yellow(),
        ));
        for run in &environment.runs {
            let presentation = &presentations[&environment.hash].runs[&run.id];
            let label = presentation
                .label
                .as_ref()
                .map_or_else(String::new, |label| format!("{label} · "));
            emit(&format!(
                "    └─ {}  {}{}",
                run.id.as_str().cyan(),
                label,
                run.command
                    .as_str()
                    .with(Color::Yellow)
                    .attribute(Attribute::Italic)
            ));
        }
    }
}
