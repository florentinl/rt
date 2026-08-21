import * as path from "path";

/** Return whether a workspace-relative file belongs to a Riot pytest target. */
export function matchesPytestTarget(
  relativePath: string,
  target: string,
): boolean {
  const rel = trimTrailingSeparators(path.normalize(relativePath));
  const targetPath = target.split("::", 1)[0];
  const normalizedTarget = trimTrailingSeparators(path.normalize(targetPath));

  if (normalizedTarget === "" || normalizedTarget === ".") {
    return true;
  }
  return rel === normalizedTarget || rel.startsWith(normalizedTarget + path.sep);
}

function trimTrailingSeparators(value: string): string {
  let end = value.length;
  while (end > 0 && (value[end - 1] === "/" || value[end - 1] === "\\")) {
    end -= 1;
  }
  return value.slice(0, end);
}
