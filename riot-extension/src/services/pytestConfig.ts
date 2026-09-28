import { RtTestRun } from "../types/rtTypes";

/** Preserve suite pytest policy; the dynamic CLI placeholder is invalid in VS Code settings. */
export function pytestConfigurationArgs(
  run: RtTestRun | undefined,
): string[] | undefined {
  if (!run) {
    return undefined;
  }
  const args = run.pytest_args.filter((argument) => argument !== "{cmdargs}");
  return args.length > 0 ? args : undefined;
}
