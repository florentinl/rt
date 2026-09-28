use crossterm::style::{Attribute, Color, Stylize};

use crate::venv::TestEnvironment;

pub fn step(message: impl AsRef<str>) {
    eprintln!("{} {}", "==>".bold().cyan(), message.as_ref());
}
pub fn detail(message: impl AsRef<str>) {
    eprintln!("    {}", message.as_ref());
}
pub fn print_environment_hierarchy(environments: &[TestEnvironment], mut emit: impl FnMut(&str)) {
    let mut previous_suite = None;
    for environment in environments {
        if previous_suite != Some(environment.suite.as_str()) {
            emit(&format!("{}", environment.suite.as_str().bold().yellow()));
            previous_suite = Some(&environment.suite);
        }
        emit(&format!(
            "  {} {} {}",
            environment.hash.as_str().bold().blue(),
            environment.name.as_str().bold().yellow(),
            environment.python.as_str().bold().green()
        ));
        for run in &environment.runs {
            emit(&format!(
                "    └─ {}  {} {}",
                run.id.as_str().cyan(),
                "runs:".with(Color::White).attribute(Attribute::Dim),
                run.command
                    .as_str()
                    .with(Color::Yellow)
                    .attribute(Attribute::Italic)
            ));
        }
    }
}
