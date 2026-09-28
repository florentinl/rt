/** Machine-readable suitespec model returned by `rt list --json`. */

export interface RtSuiteIndex {
  readonly schema_version: 1;
  readonly suites: RtSuite[];
}

export interface RtSuite {
  readonly id: string;
  readonly name: string;
  readonly patterns: string[];
  readonly source_files: string[];
  readonly services: string[];
  readonly suite_env: Record<string, string>;
  readonly environments: RtTestEnvironment[];
}

export interface RtTestEnvironment {
  readonly hash: string;
  readonly name: string;
  /** Added to the version 1 index after the initial suitespec release. */
  readonly display_name?: string;
  readonly integration_name: string;
  readonly python: string;
  readonly direct_dependencies: string[];
  readonly resolved_pkgs: Record<string, string>;
  readonly display_pkgs: Record<string, string>;
  readonly lockfile: string;
  /** Physical dependency environment shared by every run. */
  readonly venv_path: string;
  readonly python_path: string;
  readonly activate_path: string;
  readonly runs: RtTestRun[];
}

export interface RtTestRun {
  /** Stable selector accepted by `rt build` and `_is_venv_ready`. */
  readonly id: string;
  readonly index: number;
  readonly command: string;
  readonly env: Record<string, string>;
  /** Variables whose values or presence differ between sibling runs. */
  readonly differing_env?: Record<string, string | null>;
  /** Authoritative pytest arguments from the suitespec command, without `{cmdargs}`. */
  readonly pytest_args: string[];
  readonly pytest_targets: string[];
  /** Fingerprint of every input that affects the immutable run profile. */
  readonly profile_fingerprint: string;
  readonly profile_path: string;
  readonly display_name: string;
  readonly short_display_name: string;
  /** Logical profile environment. Its launcher selects `profile_path`. */
  readonly venv_path: string;
  readonly python_path: string;
  readonly activate_path: string;
}

export const buildManagerId = (extensionId: string, name: string): string =>
  `${extensionId}:${name.toLowerCase().replace(/[^a-zA-Z0-9-_]/g, "_")}`;
