# Vendored `types-envier`

PEP 561 stub-only package for `envier`, shipped in the `rt-runtime` wheel under
`purelib/envier-stubs`.

| | |
|---|---|
| Upstream project | `types-envier` (DataDog) |
| Upstream version | `0.6.1.0` |
| Stubs target | `envier==0.6.1` |
| License | MIT (see `LICENSE`) |

## Why it is vendored

`envier` ships neither `py.typed` nor inline annotations, so every
`ddtrace.settings`-style configuration attribute resolves to `Unknown` in the
editor. The dedicated runtime wheel owns these stubs alongside rt's startup and
pytest integration, so uv installs and removes the complete runtime as one
ordinary package.

Stub-only packages must be laid out non-editably to be picked up reliably. The
runtime wheel installs `envier-stubs` directly into `site-packages` as wheel
`purelib` data.

## Updating

Copy the stub sources verbatim so `mypy.stubtest` results stay comparable with
upstream, then refresh the table above:

```console
cp ~/dd/envier-typing/envier-stubs/*.pyi rt/_runtime_project/purelib/envier-stubs/
cp ~/dd/envier-typing/LICENSE rt/_runtime_project/vendor/types-envier/LICENSE
```

The runtime project's `purelib/**` uv cache key detects added, removed, or
changed stubs. Bumping the `rt` and `rt-runtime` versions invalidates existing
venvs, so the refreshed wheel is installed on the next build.
