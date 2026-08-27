import { RtVenv } from "../types/rtTypes";

interface RiotPackageMetadata {
  readonly name: string;
  readonly version?: string;
  readonly tooltip: string;
  readonly isTransitive: boolean;
}

/** Normalize a distribution name according to Python's package-name rules. */
export function normalizePackageName(name: string): string {
  return name.toLowerCase().replace(/[-_.]+/g, "-");
}

/** Build package metadata from Riot's declarative and resolved package sets. */
export function buildPackageMetadata(venv: RtVenv): RiotPackageMetadata[] {
  const direct = normalizedEntries(venv.pkgs);
  const resolved = normalizedEntries(venv.resolved_pkgs);
  const names = new Map<string, string>();

  for (const name of Object.keys(venv.display_pkgs)) {
    names.set(normalizePackageName(name), name);
  }
  for (const name of Object.keys(venv.pkgs)) {
    names.set(normalizePackageName(name), name);
  }
  for (const name of Object.keys(venv.resolved_pkgs)) {
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
      ? `Installed transitive dependency${version ? `: ${version}` : ""}`
      : buildDirectTooltip(name, declared, version);

    return { name, version, tooltip, isTransitive };
  });
}

export function getDirectPackageNames(venv: RtVenv): Set<string> {
  return new Set(Object.keys(venv.pkgs).map(normalizePackageName));
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
  name: string,
  constraint: string,
  version: string | undefined,
): string {
  const declared = constraint ? `${name}${constraint}` : `${name} (latest)`;
  return [
    "Direct dependency declared by Riot",
    `Declared: ${declared}`,
    `Installed: ${version ?? "not resolved"}`,
  ].join("\n");
}
