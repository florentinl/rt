/**
 * Service for executing RT CLI commands
 */

import * as cp from "child_process";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { RtSuite, RtSuiteIndex } from "../types/rtTypes";

const MAX_BUFFER_BYTES = 20 * 1024 * 1024; // 20MB to handle large rt list outputs

export interface RtCommandOptions {
  cwd?: string;
  signal?: AbortSignal;
}

export class RtCliService {
  private readonly command: string;

  constructor(
    private readonly log: vscode.LogOutputChannel,
    extensionPath?: string,
  ) {
    this.command = resolveRtCommand(extensionPath, log);
  }

  /**
   * Execute an RT CLI command
   */
  async execute(args: string[], options?: RtCommandOptions): Promise<string> {
    return new Promise((resolve, reject) => {
      let finished = false;
      let child: cp.ChildProcess | undefined;

      const cleanup = () => {
        if (options?.signal) {
          options.signal.removeEventListener("abort", onAbort);
        }
      };

      const onAbort = () => {
        if (finished) {
          return;
        }
        finished = true;
        cleanup();
        child?.kill();
        reject(new vscode.CancellationError());
      };

      child = cp.execFile(
        this.command,
        args,
        {
          cwd: options?.cwd,
          maxBuffer: MAX_BUFFER_BYTES,
          env: { ...process.env, DD_FAST_BUILD: "1" },
        },
        (err, stdout, stderr) => {
          if (finished) {
            return;
          }
          finished = true;
          cleanup();

          if (stderr?.trim()) {
            this.log.appendLine(`[rt] ${stderr.trim()}`);
          }

          if (err) {
            const message = err instanceof Error ? err.message : String(err);
            this.log.appendLine(`[rt] rt ${args.join(" ")} failed: ${message}`);
            reject(err);
            return;
          }

          resolve(stdout);
        },
      );

      if (options?.signal) {
        if (options.signal.aborted) {
          onAbort();
        } else {
          options.signal.addEventListener("abort", onAbort, { once: true });
        }
      }
    });
  }

  /**
   * List suitespec suites and their concrete run profiles.
   */
  async listSuites(cwd: string, signal?: AbortSignal): Promise<RtSuite[]> {
    const stdout = await this.execute(["list", "--json"], { cwd, signal });
    const index = JSON.parse(stdout) as Partial<RtSuiteIndex>;
    if (index.schema_version !== 1 || !Array.isArray(index.suites)) {
      throw new Error(
        `Unsupported rt list schema: expected schema_version 1, got ${JSON.stringify(index.schema_version)}`,
      );
    }
    return index.suites;
  }

  /**
   * Build a suite environment or logical run profile by selector.
   */
  async buildEnvironment(
    selector: string,
    cwd: string,
    options?: {
      forceReinstall?: boolean;
      signal?: AbortSignal;
    },
  ): Promise<void> {
    const args = ["build", selector];
    if (options?.forceReinstall) {
      args.push("--force-reinstall");
    }
    await this.execute(args, { cwd, signal: options?.signal });
  }

  /** Check whether rt considers a logical run profile complete. */
  async isEnvironmentReady(
    selector: string,
    profileFingerprint: string,
    cwd: string,
  ): Promise<boolean> {
    const stdout = (
      await this.execute(
        ["_is_venv_ready", selector, "--profile-fingerprint", profileFingerprint],
        { cwd },
      )
    ).trim();
    if (stdout === "true") {
      return true;
    }
    if (stdout === "false") {
      return false;
    }
    throw new Error(`Unexpected rt readiness response: ${JSON.stringify(stdout)}`);
  }
}

function resolveRtCommand(
  extensionPath: string | undefined,
  log: vscode.LogOutputChannel,
): string {
  if (extensionPath) {
    const bundled = path.join(extensionPath, "runtime", "bin", "rt");
    try {
      fs.accessSync(bundled, fs.constants.X_OK);
      log.appendLine(`[rt] Using bundled rt binary at ${bundled}`);
      return bundled;
    } catch {
      log.appendLine(
        `[rt] Bundled rt binary not found at ${bundled}; falling back to 'rt' on PATH`,
      );
    }
  }
  return "rt";
}
