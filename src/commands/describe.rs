use std::path::Path;

use crossterm::style::{Attribute, Color, Stylize};
use indexmap::IndexMap;
use pyo3::{PyResult, Python};

use crate::{
    config::{RepoConfig, Selector},
    venv::{select_execution_contexts, venv_path, RiotVenv},
};

pub fn run(py: Python<'_>, repo: &RepoConfig, hash: String) -> PyResult<()> {
    let selected = select_execution_contexts(py, &repo.riotfile_path, Selector::Pattern(hash))?;

    for venv in selected {
        describe_venv(repo, &venv);
    }

    Ok(())
}

fn describe_venv(repo: &RepoConfig, venv: &RiotVenv) {
    println!(
        "{} {}",
        "Virtual environment".bold().cyan(),
        venv.hash.as_str().bold().blue()
    );
    print_kv("name", venv.name.as_str().bold().yellow(), 2);
    print_kv("python", venv.python.as_str().bold().green(), 2);
    print_kv(
        "path",
        format_path(&relative_venv_path(&repo.riot_root, &venv.hash)),
        2,
    );
    print_section("packages", 2, || print_packages(&venv.pkgs, 4));

    println!("  {}", style_label("execution contexts"));
    if venv.execution_contexts.is_empty() {
        println!("    {}", "<none>".attribute(Attribute::Dim));
        return;
    }

    let mut contexts = venv.execution_contexts.iter().collect::<Vec<_>>();
    contexts.sort_by(|a, b| a.hash.cmp(&b.hash));

    for ctx in contexts {
        println!("    {}", ctx.hash.as_str().cyan());
        print_kv(
            "path",
            format_path(&relative_venv_path(&repo.riot_root, &ctx.hash)),
            6,
        );
        if ctx.create {
            print_kv("create venv", bool_flag(true), 6);
        }
        if ctx.skip_dev_install {
            print_kv("skip dev install", bool_flag(true), 6);
        }
        print_kv("command", format_command(ctx.command.as_deref()), 6);
        print_section("env", 6, || print_env_block(&ctx.env, 8));
    }
}

fn print_packages(pkgs: &IndexMap<String, String>, indent: usize) {
    let pad = " ".repeat(indent);
    if pkgs.is_empty() {
        println!("{}{}", pad, "<none>".attribute(Attribute::Dim));
        return;
    }

    for (name, version) in pkgs {
        println!(
            "{}{}{}",
            pad,
            name.as_str().bold().yellow(),
            version
                .as_str()
                .with(Color::Magenta)
                .attribute(Attribute::Dim)
        );
    }
}

fn print_env_block(env: &IndexMap<String, String>, indent: usize) {
    let pad = " ".repeat(indent);
    if env.is_empty() {
        println!("{}{}", pad, "<none>".attribute(Attribute::Dim));
        return;
    }

    for (key, value) in env {
        println!(
            "{}{}={}",
            pad,
            key.as_str().bold().green(),
            value.as_str().with(Color::Magenta)
        );
    }
}

fn print_kv(label: &str, value: impl std::fmt::Display, indent: usize) {
    let pad = " ".repeat(indent);
    println!("{}{} {}", pad, style_label(label), value);
}

fn print_section(label: &str, indent: usize, body: impl FnOnce()) {
    let pad = " ".repeat(indent);
    println!("{}{}", pad, style_label(label));
    body();
}

fn style_label(label: &str) -> String {
    format!("{label}:")
        .with(Color::White)
        .attribute(Attribute::Bold)
        .to_string()
}

fn bool_flag(value: bool) -> String {
    if value {
        "yes".with(Color::Green).to_string()
    } else {
        "no".with(Color::Red).attribute(Attribute::Dim).to_string()
    }
}

fn format_command(cmd: Option<&str>) -> String {
    let Some(cmd) = cmd else {
        return "<none>".to_string();
    };

    if cmd.is_empty() {
        return "<empty>".to_string();
    }

    if cmd.contains("{cmdargs}") {
        let replaced = cmd.replace("{cmdargs}", &"{cmdargs}".magenta().to_string());
        return replaced
            .with(Color::Yellow)
            .attribute(Attribute::Italic)
            .to_string();
    }

    cmd.with(Color::Yellow)
        .attribute(Attribute::Italic)
        .to_string()
}

fn format_path(path: &Path) -> String {
    format!("{}", path.display())
        .with(Color::Magenta)
        .attribute(Attribute::Dim)
        .to_string()
}

fn relative_venv_path(riot_root: &Path, hash: &str) -> std::path::PathBuf {
    let absolute = venv_path(riot_root, hash);
    let base = riot_root.parent().unwrap_or(riot_root);
    if let Ok(relative) = absolute.strip_prefix(base) {
        return relative.to_path_buf();
    }
    absolute
}
