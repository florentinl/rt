import { defineConfig } from "@vscode/test-cli";

export default defineConfig({
  files: "out/test/**/*.vscode.test.js",
  version: "1.134.0",
  launchArgs: ["--user-data-dir=/private/tmp/riot-vscode-test-user"],
  mocha: {
    timeout: 30_000,
  },
});
