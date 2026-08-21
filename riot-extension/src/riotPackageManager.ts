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
import { RiotEnvManager, RiotEnvironmentRecord } from "./riotEnvManager";
import {
  buildPackageMetadata,
  getDirectPackageNames as directPackageNames,
  normalizePackageName,
} from "./services/packageMetadata";
import { buildManagerId } from "./types/rtTypes";

export class RiotPackageManager implements PackageManager {
  readonly name = "rt";
  readonly displayName = "Riot (read-only)";
  readonly description = "Packages declared by Riot";
  readonly tooltip =
    "Riot environments are immutable; edit riotfile.py to change packages";
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
    private readonly envManager: RiotEnvManager,
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
      "Riot environments are immutable. Edit riotfile.py, then refresh or rebuild the environment.",
    );
    error.name = "RiotEnvironmentImmutableError";
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
          changes: before.map((pkg) => ({
            kind: PackageChangeKind.remove,
            pkg,
          })),
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

  getPackageWatchTargets(
    environment: PythonEnvironment,
  ): vscode.RelativePattern[] {
    const record = this.envManager.getKnownEnvironmentRecord(environment);
    if (!record) {
      return [];
    }

    const riotRoot = path.dirname(record.venv.venv_path);
    const declarationRoot = path.dirname(riotRoot);
    const requirementsDirectory = path.join(riotRoot, "requirements");
    return [
      new vscode.RelativePattern(declarationRoot, "riotfile.py"),
      new vscode.RelativePattern(declarationRoot, "rt.toml"),
      new vscode.RelativePattern(requirementsDirectory, `${record.venv.hash}.txt`),
    ];
  }

  async getDirectPackageNames(
    environment: PythonEnvironment,
  ): Promise<Set<string> | undefined> {
    const record = await this.envManager.getEnvironmentRecord(environment, false);
    return record ? directPackageNames(record.venv) : undefined;
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

  private buildPackages(record: RiotEnvironmentRecord): Package[] {
    return buildPackageMetadata(record.venv).map((pkg) => ({
      ...pkg,
      displayName: pkg.name,
      pkgId: {
        id: normalizePackageName(pkg.name),
        managerId: this.managerId,
        environmentId: record.environment.envId.id,
      },
    }));
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
