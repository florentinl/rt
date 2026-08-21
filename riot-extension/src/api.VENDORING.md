# Python Environments API vendoring

`api.ts` is vendored verbatim from the Microsoft Python Environments extension.
Do not format or edit it locally.

- Repository: `https://github.com/microsoft/vscode-python-environments`
- Source: `src/api.ts`
- Commit: `9a2c11497d9b1a201965c4d5b311de055dddda59`
- API package version: `1.3.0`
- Git blob: `13504a2d34a577b23fc2f80448f19944a692de06`

To verify a candidate update before replacing the file:

```shell
git -C /path/to/vscode-python-environments rev-parse <commit>:src/api.ts
git -C /path/to/vscode-python-environments show <commit>:src/api.ts | git hash-object --stdin
```

Both hashes must agree. After copying the exact source, run the extension type,
lint, package, and integration checks.

Keep `@renovatebot/pep440` on the major used by the upstream repository. The
vendored CommonJS API currently consumes 4.x; 5.x is ESM-only. Keep TypeScript
within the version range supported by the current `typescript-eslint` release.
