# Vendored `types-envier`

PEP 561 stub-only package for `envier`, embedded into the `rt` binary with
`include_str!` and written into every generated venv that gets `envier` (see
`src/commands/build.rs`).

| | |
|---|---|
| Upstream project | `types-envier` (DataDog) |
| Upstream version | `0.6.1.0` |
| Stubs target | `envier==0.6.1` |
| License | MIT (see `LICENSE`) |

## Why it is vendored

`envier` ships neither `py.typed` nor inline annotations, so every
`ddtrace.settings`-style configuration attribute resolves to `Unknown` in the
editor. `envier` reaches a riot venv as a transitive dependency of the
dd-trace-py dev install rather than as a riotfile package, so there is no
requirements line to add the stubs to - `rt` injects them directly into the
execution context's `site-packages`, next to the `pytest_rt` plugin.

Stub-only packages must be laid out non-editably to be picked up reliably, which
is exactly what copying the `envier-stubs` directory does.

## Updating

Copy the stub sources verbatim so `mypy.stubtest` results stay comparable with
upstream, then refresh the table above:

```console
cp ~/dd/envier-typing/envier-stubs/*.pyi vendor/types-envier/envier-stubs/
cp ~/dd/envier-typing/LICENSE vendor/types-envier/LICENSE
```

Adding or removing a `.pyi` file also requires updating the `ENVIER_STUBS` table
in `src/commands/build.rs`, since the contents are embedded at compile time.
Bumping the `rt` version invalidates every cached venv, so the refreshed stubs
are picked up automatically on the next build.
