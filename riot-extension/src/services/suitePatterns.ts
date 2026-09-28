import * as path from "path";

/** Apply suitespec's positive patterns and `!` exclusions to one path. */
export function matchesSuitePatterns(
  relativePath: string,
  patterns: readonly string[],
): boolean {
  const positive = patterns.filter((pattern) => !pattern.startsWith("!"));
  const excluded = patterns
    .filter((pattern) => pattern.startsWith("!"))
    .some((pattern) => matchesSuitePattern(relativePath, pattern.slice(1)));
  return (
    !excluded &&
    positive.some((pattern) => matchesSuitePattern(relativePath, pattern))
  );
}

/** Match a workspace-relative path against an expanded suitespec pattern. */
export function matchesSuitePattern(
  relativePath: string,
  pattern: string,
): boolean {
  const relative = toPosix(relativePath);
  const normalizedPattern = toPosix(pattern).replace(/^\.\//, "");
  if (normalizedPattern === "" || normalizedPattern === ".") {
    return true;
  }

  if (!/[?*[]/.test(normalizedPattern)) {
    return (
      relative === normalizedPattern || relative.startsWith(`${normalizedPattern}/`)
    );
  }
  return globToRegExp(normalizedPattern).test(relative);
}

function toPosix(value: string): string {
  return path
    .normalize(value.replace(/\\/g, "/"))
    .split(path.sep)
    .join("/")
    .replace(/^\.\//, "");
}

function globToRegExp(glob: string): RegExp {
  let source = "^";
  for (let index = 0; index < glob.length; index += 1) {
    const char = glob[index];
    if (char === "*") {
      if (glob[index + 1] === "*") {
        index += 1;
        source += glob[index + 1] === "/" ? "(?:.*/)?" : ".*";
        if (glob[index + 1] === "/") {
          index += 1;
        }
      } else {
        // Suitespec uses Python fnmatch, where '*' also spans path separators.
        source += ".*";
      }
    } else if (char === "?") {
      source += ".";
    } else {
      source += char.replace(/[\\^$+.()|{}]/g, "\\$&");
    }
  }
  return new RegExp(`${source}$`);
}
