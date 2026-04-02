/**
 * Service for executing RT CLI commands
 */

import * as cp from "child_process";
import * as vscode from "vscode";
import { RtVenv } from "../types/rtTypes";

const MAX_BUFFER_BYTES = 20 * 1024 * 1024; // 20MB to handle large rt list outputs

export interface RtCommandOptions {
  cwd?: string;
  signal?: AbortSignal;
}

export class RtCliService {
  constructor(private readonly log: vscode.LogOutputChannel) {}

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

      const command = "rt";
      child = cp.execFile(
        command,
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
            this.log.appendLine(`[riot] ${stderr.trim()}`);
          }

          if (err) {
            const message = err instanceof Error ? err.message : String(err);
            this.log.appendLine(`[riot] rt ${args.join(" ")} failed: ${message}`);
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
   * List all Riot virtual environments
   */
  async listEnvironments(cwd: string, signal?: AbortSignal): Promise<RtVenv[]> {
    const stdout = await this.execute(["list", "--json"], { cwd, signal });
    return JSON.parse(stdout);
  }

  /**
   * Build a Riot environment by hash
   */
  async buildEnvironment(
    hash: string,
    cwd: string,
    options?: {
      forceReinstall?: boolean;
      signal?: AbortSignal;
    },
  ): Promise<void> {
    const args = ["build", hash];
    if (options?.forceReinstall) {
      args.push("--force-reinstall");
    }
    await this.execute(args, { cwd, signal: options?.signal });
  }
}
