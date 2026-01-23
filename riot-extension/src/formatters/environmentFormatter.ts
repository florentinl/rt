/**
 * Utility for formatting environment display names and metadata
 */

import { EnvironmentNames, RtExecutionContext, RtVenv } from '../types/rtTypes';

export class EnvironmentDisplayFormatter {
    /**
     * Format a record of entries (packages or env vars) as a display string
     */
    formatEntries(map: Record<string, string>, maxEntries = 2): string | undefined {
        const entries = Object.entries(map);
        if (entries.length === 0) {
            return undefined;
        }

        entries.sort(([a], [b]) => a.localeCompare(b));
        const shown = entries
            .slice(0, maxEntries)
            .map(([key, value]) => `${key}=${value === '' ? 'latest' : value}`);

        const remaining = entries.length - shown.length;
        const tail = remaining > 0 ? ` +${remaining} more` : '';
        return `${shown.join(', ')}${tail}`;
    }

    /**
     * Get unique packages that differ from shared packages
     */
    getUniquePackages(venv: RtVenv): Record<string, string> {
        const diff: Record<string, string> = {};
        for (const [key, value] of Object.entries(venv.pkgs)) {
            if (!(key in venv.shared_pkgs) || venv.shared_pkgs[key] !== value) {
                diff[key] = value;
            }
        }
        return Object.keys(diff).length > 0 ? diff : venv.pkgs;
    }

    /**
     * Get environment variables that differ from shared env vars
     */
    getContextEnvDiff(ctx: RtExecutionContext, venv: RtVenv): Record<string, string> {
        const diff: Record<string, string> = {};
        for (const [key, value] of Object.entries(ctx.env)) {
            if (!(key in venv.shared_env) || venv.shared_env[key] !== value) {
                diff[key] = value;
            }
        }
        return diff;
    }

    /**
     * Normalize Python version string to semantic versioning format
     */
    normalizePythonVersion(version: string): string {
        const parts = version
            .split('.')
            .map((part) => part.match(/^\d+/)?.[0])
            .filter((part): part is string => Boolean(part))
            .map((part) => Number.parseInt(part, 10))
            .slice(0, 3);

        if (parts.length === 0) {
            return version;
        }

        while (parts.length < 3) {
            parts.push(0);
        }

        return parts.join('.');
    }

    /**
     * Build display names for an environment
     */
    buildDisplayNames(venv: RtVenv, ctx: RtExecutionContext): EnvironmentNames {
        const pkgDetail = this.formatEntries(this.getUniquePackages(venv));
        const envDetail = this.formatEntries(this.getContextEnvDiff(ctx, venv));

        const details = [pkgDetail, envDetail].filter((item): item is string => Boolean(item));
        if (details.length === 0) {
            details.push(ctx.hash);
        }

        const separator = ' | ';
        const displayName = `${venv.name} (${venv.python})${separator}${details.join(separator)}`;

        const firstDetail = details[0];
        const shortTail = details.length > 1 ? `${firstDetail} +${details.length - 1} more` : firstDetail;
        const shortDisplayName = `${venv.name} (${venv.python})${separator}${shortTail}`;

        return { displayName, shortDisplayName };
    }
}
