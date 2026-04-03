/**
 * Shared type definitions for Riot extension
 */

export type RtExecutionContext = {
    hash: string;
    venv_path: string;
    python_path: string;
    activate_path: string;
    display_name: string;
    short_display_name: string;
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
    resolved_pkgs: Record<string, string>;
    display_pkgs: Record<string, string>;
    execution_contexts: RtExecutionContext[];
};

export const normalizeManagerName = (name: string): string =>
    name.toLowerCase().replace(/[^a-zA-Z0-9-_]/g, '_');

export const buildManagerId = (extensionId: string, name: string): string =>
    `${extensionId}:${normalizeManagerName(name)}`;
