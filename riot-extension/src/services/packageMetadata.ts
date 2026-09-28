import { RtTestEnvironment } from "../types/rtTypes";

interface SuitePackageMetadata {
  readonly name: string;
  readonly version?: string;
  readonly tooltip: string;
  readonly isTransitive: boolean;
}

/** Normalize a distribution name according to Python's package-name rules. */
export function normalizePackageName(name: string): string {
  return name.toLowerCase().replace(/[-_.]+/g, "-");
}

/** Build package metadata from suitespec declarations and the resolved lock. */
export function buildPackageMetadata(
  environment: RtTestEnvironment,
  suiteId?: string,
): SuitePackageMetadata[] {
  const direct = new Map(
    environment.direct_dependencies.map((requirement) => [
      requirementName(requirement),
      requirement,
    ]),
  );
  const resolved = normalizedEntries(environment.resolved_pkgs);
  const names = new Map<string, string>();

  for (const name of Object.keys(environment.display_pkgs)) {
    names.set(normalizePackageName(name), name);
  }
  for (const requirement of environment.direct_dependencies) {
    const name = requirementDisplayName(requirement);
    names.set(requirementName(requirement), name);
  }
  for (const name of Object.keys(environment.resolved_pkgs)) {
    const normalized = normalizePackageName(name);
    if (!names.has(normalized)) {
      names.set(normalized, name);
    }
  }

  return Array.from(names, ([normalized, name]) => {
    const declared = direct.get(normalized);
    const version = resolved.get(normalized) || undefined;
    const isTransitive = declared === undefined;
    const tooltip = isTransitive
      ? [
          "Transitive dependency resolved from the suitespec lockfile",
          `Installed: ${version ?? "not resolved"}`,
          `Lockfile: ${environment.lockfile}`,
        ].join("\n")
      : buildDirectTooltip(declared, version, suiteId, environment.name);

    return { name, version, tooltip, isTransitive };
  });
}

export function getDirectPackageNames(
  environment: RtTestEnvironment,
): Set<string> {
  return new Set(environment.direct_dependencies.map(requirementName));
}

function normalizedEntries(
  entries: Record<string, string>,
): Map<string, string> {
  return new Map(
    Object.entries(entries).map(([name, value]) => [
      normalizePackageName(name),
      value,
    ]),
  );
}

function buildDirectTooltip(
  requirement: string,
  version: string | undefined,
  suiteId: string | undefined,
  environmentName: string,
): string {
  return [
    `Direct dependency declared by suitespec${suiteId ? ` for ${suiteId}/${environmentName}` : ""}`,
    `Declared: ${requirement}`,
    `Installed: ${version ?? "not resolved"}`,
  ].join("\n");
}

function requirementName(requirement: string): string {
  return normalizePackageName(requirementDisplayName(requirement));
}

function requirementDisplayName(requirement: string): string {
  const match = /^\s*([A-Za-z0-9][A-Za-z0-9._-]*)/.exec(requirement);
  return match?.[1] ?? requirement.trim();
}
