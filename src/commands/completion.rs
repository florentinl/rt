use std::process::Command;

use clap::CommandFactory;
use clap_complete::{generate, Shell};
use pyo3::{exceptions::PySystemExit, PyErr, PyResult};

use crate::Cli;

pub fn run(shell: Shell) -> PyResult<()> {
    let output = Command::new("rt")
        .env("COMPLETE", shell.to_string())
        .output()
        .map_err(|e| {
            eprintln!("error: failed to generate completions: {e}");
            PyErr::new::<PySystemExit, _>(1)
        })?;

    let dynamic_cmp = String::from_utf8(output.stdout).map_err(|e| {
        eprintln!("error: invalid UTF-8 in completion output: {e}");
        PyErr::new::<PySystemExit, _>(1)
    })?;

    println!("{dynamic_cmp}");

    if matches!(shell, Shell::Zsh) {
        let mut buf = Vec::new();
        generate(Shell::Zsh, &mut Cli::command(), "rt", &mut buf);

        let static_cmp = String::from_utf8(buf)
            .map_err(|e| {
                eprintln!("error: invalid UTF-8 in static completion output: {e}");
                PyErr::new::<PySystemExit, _>(1)
            })?
            .replace("_default", "_do_nothing");

        println!(
            "{static_cmp}
            _do_nothing() {{ return 1 }}
            functions -c _rt _rt_static
            _rt() {{
                local before=${{compstate[nmatches]:-0}}
                _rt_static
                local after=${{compstate[nmatches]:-0}}
                if (( after == before )); then
                    _clap_dynamic_completer_rt
                fi
            }}"
        );
    }

    Ok(())
}
