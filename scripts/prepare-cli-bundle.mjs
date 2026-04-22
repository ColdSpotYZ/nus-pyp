import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");
const srcTauriDir = path.join(repoRoot, "src-tauri");
const resourcesDir = path.join(srcTauriDir, "resources", "cli");

const commandName = process.platform === "win32" ? "nus-exam-papers.exe" : "nus-exam-papers";
const cargoBinaryName = process.platform === "win32" ? "nus_exam_papers_cli.exe" : "nus_exam_papers_cli";

mkdirSync(resourcesDir, { recursive: true });
rmSync(path.join(resourcesDir, commandName), { force: true });

execFileSync(
  "cargo",
  ["build", "--release", "--bin", "nus_exam_papers_cli"],
  {
    cwd: srcTauriDir,
    stdio: "inherit",
  },
);

const builtBinaryPath = path.join(srcTauriDir, "target", "release", cargoBinaryName);
const bundledBinaryPath = path.join(resourcesDir, commandName);

copyFileSync(builtBinaryPath, bundledBinaryPath);

console.log(`Bundled CLI resource prepared at ${bundledBinaryPath}`);
