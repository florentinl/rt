import * as path from "path";
import * as vscode from "vscode";
import {
  GetPackagesOptions,
  Package,
  PackageChangeKind,
  PackageManager,
  PackageManagementOptions,
  PythonEnvironment,
} from "./api";
import {
  SuitespecEnvironmentRecord,
  SuitespecEnvManager,
} from "./suitespecEnvManager";
import {
  buildPackageMetadata,
  getDirectPackageNames as directPackageNames,
  normalizePackageName,
} from "./services/packageMetadata";
import { buildManagerId } from "./types/rtTypes";

export class SuitespecPackageManager implements PackageManager {
  readonly name = "rt-packages";
  readonly displayName = "Suitespec dependencies (read-only)";
  readonly description = "Dependencies declared by suitespec and resolved by its lockfile";
  readonly tooltip =
    "Suite environments are immutable; edit the relevant suitespec.yml and refresh its lockfile";
  readonly log: vscode.LogOutputChannel;

  private readonly managerId: string;
  private readonly packageCache = new Map<string, Package[]>();
  private readonly pkgEmitter = new vscode.EventEmitter<{
    environment: PythonEnvironment;
    manager: PackageManager;
    changes: { kind: PackageChangeKind; pkg: Package }[];
  }>();

  readonly onDidChangePackages = this.pkgEmitter.event;

  constructor(
    log: vscode.LogOutputChannel,
    private readonly envManager: SuitespecEnvManager,
    extensionId: string,
  ) {
    this.log = log;
    this.managerId = buildManagerId(extensionId, this.name);
  }

  manage(
    _environment: PythonEnvironment,
    _options: PackageManagementOptions,
  ): Promise<void> {
    const error = new Error(
      "Suite environments are immutable. Edit suitespec.yml, refresh its lockfile, then refresh or rebuild the environment.",
    );
    error.name = "SuitespecEnvironmentImmutableError";
    return Promise.reject(error);
  }

  async refresh(environment: PythonEnvironment): Promise<void> {
    const before = this.packageCache.get(environment.envId.id) ?? [];
    const after = await this.loadPackages(environment, true);
    if (!after) {
      if (before.length > 0) {
        this.pkgEmitter.fire({
          environment,
          manager: this,
          changes: before.map((pkg) => ({ kind: PackageChangeKind.remove, pkg })),
        });
      }
      return;
    }
    const changes = getPackageChanges(before, after);
    if (changes.length > 0) {
      this.pkgEmitter.fire({ environment, manager: this, changes });
    }
  }

  async getPackages(
    environment: PythonEnvironment,
    options?: GetPackagesOptions,
  ): Promise<Package[] | undefined> {
    const forceRefresh = options?.skipCache === true;
    if (!forceRefresh) {
      const cached = this.packageCache.get(environment.envId.id);
      if (cached) {
        return cached;
      }
    }
    return this.loadPackages(environment, forceRefresh);
  }

  getPackageWatchTargets(environment: PythonEnvironment): vscode.RelativePattern[] {
    const record = this.envManager.getKnownEnvironmentRecord(environment);
    if (!record) {
      return [];
    }
    const targets = [...record.suite.source_files, record.testEnvironment.lockfile];
    return targets.map((target) => {
      const absolute = path.isAbsolute(target)
        ? target
        : path.join(record.workspaceRoot, target);
      return new vscode.RelativePattern(path.dirname(absolute), path.basename(absolute));
    });
  }

  async getDirectPackageNames(
    environment: PythonEnvironment,
  ): Promise<Set<string> | undefined> {
    const record = await this.envManager.getEnvironmentRecord(environment, false);
    return record ? directPackageNames(record.testEnvironment) : undefined;
  }

  async clearCache(): Promise<void> {
    this.packageCache.clear();
    await this.envManager.clearCache();
  }

  private async loadPackages(
    environment: PythonEnvironment,
    forceRefresh: boolean,
  ): Promise<Package[] | undefined> {
    const record = await this.envManager.getEnvironmentRecord(
      environment,
      forceRefresh,
    );
    if (!record) {
      this.packageCache.delete(environment.envId.id);
      return undefined;
    }
    const packages = this.buildPackages(record);
    this.packageCache.set(environment.envId.id, packages);
    return packages;
  }

  private buildPackages(record: SuitespecEnvironmentRecord): Package[] {
    return buildPackageMetadata(record.testEnvironment, record.suite.id).map(
      (pkg) => ({
        ...pkg,
        displayName: pkg.name,
        pkgId: {
          id: normalizePackageName(pkg.name),
          managerId: this.managerId,
          environmentId: record.environment.envId.id,
        },
      }),
    );
  }
}

function getPackageChanges(
  before: Package[],
  after: Package[],
): { kind: PackageChangeKind; pkg: Package }[] {
  const key = (pkg: Package) =>
    `${normalizePackageName(pkg.name)}==${pkg.version ?? ""}`;
  const beforeKeys = new Set(before.map(key));
  const afterKeys = new Set(after.map(key));
  return [
    ...after
      .filter((pkg) => !beforeKeys.has(key(pkg)))
      .map((pkg) => ({ kind: PackageChangeKind.add, pkg })),
    ...before
      .filter((pkg) => !afterKeys.has(key(pkg)))
      .map((pkg) => ({ kind: PackageChangeKind.remove, pkg })),
  ];
}
