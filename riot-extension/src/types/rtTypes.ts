/**
 * Shared type definitions for Riot extension
 */

export type RtExecutionContext = {
    hash: string;
    venv_path: string;
    command?: string;
    pytest_target?: string;
    env: Record<string, string>;
    create: boolean;
    skip_dev_install: boolean;
};

export type RtVenv = {
    hash: string;
    venv_path: string;
    name: string;
    python: string;
    pkgs: Record<string, string>;
    shared_pkgs: Record<string, string>;
    shared_env: Record<string, string>;
    execution_contexts: RtExecutionContext[];
};

export interface EnvironmentNames {
    displayName: string;
    shortDisplayName: string;
}

export const normalizeManagerName = (name: string): string =>
    name.toLowerCase().replace(/[^a-zA-Z0-9-_]/g, '_');

export const buildManagerId = (extensionId: string, name: string): string =>
    `${extensionId}:${normalizeManagerName(name)}`;
