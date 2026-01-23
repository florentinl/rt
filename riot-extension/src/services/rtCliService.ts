/**
 * Service for executing RT CLI commands
 * Handles command execution, parsing, and error handling
 */

import * as cp from 'child_process';
import * as vscode from 'vscode';
import { RtVenv } from '../types/rtTypes';

const MAX_BUFFER_BYTES = 20 * 1024 * 1024; // 20MB to handle large rt list outputs

export interface RtCommandOptions {
    cwd?: string;
    signal?: AbortSignal;
}

export class RtCliService {
    constructor(
        private readonly rtPath: string | undefined,
        private readonly log: vscode.LogOutputChannel,
    ) {}

    /**
     * Execute an RT CLI command
     */
    async execute(args: string[], options?: RtCommandOptions): Promise<string> {
        return new Promise((resolve, reject) => {
            let finished = false;
            let child: cp.ChildProcess | undefined;

            const cleanup = () => {
                if (options?.signal) {
                    options.signal.removeEventListener('abort', onAbort);
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

            const command = this.rtPath ?? 'rt';
            child = cp.execFile(
                command,
                args,
                { cwd: options?.cwd, maxBuffer: MAX_BUFFER_BYTES },
                (err, stdout, stderr) => {
                    if (finished) {
                        return;
                    }
                    finished = true;
                    cleanup();

                    if (stderr?.trim()) {
                        this.logLine(stderr.trim());
                    }

                    if (err) {
                        const message = err instanceof Error ? err.message : String(err);
                        this.logLine(`rt ${args.join(' ')} failed: ${message}`);
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
                    options.signal.addEventListener('abort', onAbort, { once: true });
                }
            }
        });
    }

    /**
     * List all Riot virtual environments
     */
    async listEnvironments(cwd: string, signal?: AbortSignal): Promise<RtVenv[]> {
        const stdout = await this.execute(['list', '--json'], { cwd, signal });
        return this.parseVenvs(stdout);
    }

    /**
     * Build a Riot environment by hash
     */
    async buildEnvironment(hash: string, cwd: string, options?: {
        forceReinstall?: boolean;
        signal?: AbortSignal;
    }): Promise<void> {
        const args = ['build', hash];
        if (options?.forceReinstall) {
            args.push('--force-reinstall');
        }
        await this.execute(args, { cwd, signal: options?.signal });
    }

    /**
     * Parse and validate RT venv JSON output
     */
    private parseVenvs(json: string): RtVenv[] {
        try {
            const parsed = JSON.parse(json);

            if (!Array.isArray(parsed)) {
                this.logLine('rt list returned non-array response');
                return [];
            }

            // Validate and normalize each venv
            return parsed
                .filter((item): item is RtVenv => this.isValidRtVenv(item))
                .map(venv => this.normalizeVenv(venv));
        } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            this.logLine(`Failed to parse rt list output: ${message}`);
            throw err;
        }
    }

    /**
     * Type guard for RtVenv
     */
    private isValidRtVenv(value: unknown): value is RtVenv {
        if (typeof value !== 'object' || value === null) {
            return false;
        }

        const obj = value as Record<string, unknown>;
        return (
            typeof obj.hash === 'string' &&
            typeof obj.venv_path === 'string' &&
            typeof obj.name === 'string' &&
            typeof obj.python === 'string' &&
            Array.isArray(obj.execution_contexts)
        );
    }

    /**
     * Normalize venv data with defaults
     */
    private normalizeVenv(venv: RtVenv): RtVenv {
        return {
            ...venv,
            shared_pkgs: venv.shared_pkgs ?? {},
            shared_env: venv.shared_env ?? {},
            pkgs: venv.pkgs ?? {},
        };
    }

    private logLine(message: string): void {
        this.log.appendLine(`[riot] ${message}`);
    }
}
