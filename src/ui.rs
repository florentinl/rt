use crossterm::style::{Attribute, Color, Stylize};
use indexmap::IndexMap;

use crate::venv::RiotVenv;

/// Minimal helper for consistent CLI output.
pub fn step(message: impl AsRef<str>) {
    eprintln!("{} {}", "==>".bold().cyan(), message.as_ref());
}

/// Print a detail line associated with the latest step.
pub fn detail(message: impl AsRef<str>) {
    eprintln!("    {}", message.as_ref());
}

/// Insert a blank line to visually separate sections.
pub fn blank_line() {
    eprintln!();
}

#[must_use] 
pub fn format_pkgs(
    all_pkgs: &IndexMap<String, String>,
    shared_pkgs: &IndexMap<String, String>,
) -> String {
    format_unique_entries(all_pkgs, shared_pkgs, |key, val| format!("{key}{val} "))
}

#[must_use] 
pub fn format_envs(
    all_envs: &IndexMap<String, String>,
    shared_envs: &IndexMap<String, String>,
) -> String {
    format_unique_entries(all_envs, shared_envs, |key, val| format!("{key}={val} "))
}

fn format_unique_entries(
    map: &IndexMap<String, String>,
    shared: &IndexMap<String, String>,
    mut formatter: impl FnMut(&str, &str) -> String,
) -> String {
    let mut buf = String::new();
    for (key, val) in map {
        if !shared.contains_key(key) {
            buf.push_str(&formatter(key, val));
        }
    }
    buf
}

pub fn print_venv_hierarchy(venvs: &[RiotVenv], mut emit: impl FnMut(&str)) {
    let venvs: Vec<&RiotVenv> = venvs.iter().collect();
    if venvs.is_empty() {
        return;
    }

    for selected in venvs {
        let deps_raw = format_pkgs(&selected.pkgs, &selected.shared_pkgs);
        let deps = deps_raw.trim().to_string();

        let mut contexts = selected.execution_contexts.iter().collect::<Vec<_>>();
        contexts.sort_by_key(|exc| &exc.hash);

        let deps_segment = if deps.is_empty() {
            String::new()
        } else {
            format!(
                " {}",
                deps.with(Color::Magenta)
                    .attribute(Attribute::Dim)
                    .attribute(Attribute::Italic)
            )
        };

        emit(&format!(
            "{} {} {}{}",
            selected.hash.as_str().bold().blue(),
            selected.name.as_str().bold().yellow(),
            selected.python.as_str().bold().green(),
            deps_segment
        ));

        if contexts.is_empty() {
            continue;
        }

        let total = contexts.len();
        for (idx, context) in contexts.into_iter().enumerate() {
            let env_raw = format_envs(&context.env, &selected.shared_env);
            let env = env_raw.trim().to_string();
            let branch = if idx + 1 == total { "└" } else { "├" };
            let continuation = if idx + 1 == total { "  " } else { "│ " };

            let command_display = context.command.as_deref().map_or_else(
                || {
                    "<no command configured>"
                        .attribute(Attribute::Dim)
                        .to_string()
                },
                |cmd| {
                    if cmd.is_empty() {
                        "<empty command>".attribute(Attribute::Dim).to_string()
                    } else {
                        highlight_cmdargs(cmd)
                    }
                },
            );

            emit(&format!(
                "{branch}─ {}  {} {}",
                context.hash.as_str().cyan(),
                "runs:".with(Color::White).attribute(Attribute::Dim),
                command_display
            ));
            if !env.is_empty() {
                emit(&format!(
                    "{continuation} {}  {} {}",
                    " ".repeat(context.hash.len()),
                    "with:".with(Color::White).attribute(Attribute::Dim),
                    env.with(Color::Magenta)
                        .attribute(Attribute::Dim)
                        .attribute(Attribute::Italic)
                ));
            }
        }
    }
}

fn highlight_cmdargs(cmd: &str) -> String {
    if cmd.contains("{cmdargs}") {
        let replaced = cmd.replace("{cmdargs}", &"{cmdargs}".magenta().to_string());
        replaced
            .with(Color::Yellow)
            .attribute(Attribute::Dim)
            .attribute(Attribute::Italic)
            .to_string()
    } else {
        cmd.with(Color::Yellow)
            .attribute(Attribute::Dim)
            .attribute(Attribute::Italic)
            .to_string()
    }
}
