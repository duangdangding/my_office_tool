import { readFileSync } from "node:fs";

const root = new URL("../", import.meta.url);
const readJson = (path) => JSON.parse(readFileSync(new URL(path, root), "utf8"));
const packageJson = readJson("package.json");
const packageLock = readJson("package-lock.json");
const tauriConfig = readJson("src-tauri/tauri.conf.json");
const cargoToml = readFileSync(new URL("src-tauri/Cargo.toml", root), "utf8");
const cargoVersion = cargoToml.match(/^version\s*=\s*"([^"]+)"$/m)?.[1];
const version = packageJson.version;
const tag = process.argv[2];

const versions = {
  "package.json": version,
  "package-lock.json": packageLock.version,
  "package-lock.json packages['']": packageLock.packages?.[""]?.version,
  "src-tauri/Cargo.toml": cargoVersion,
  "src-tauri/tauri.conf.json": tauriConfig.version,
};

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version ?? "")) {
  throw new Error(`版本号必须是 SemVer，例如 0.1.0；当前为 ${version ?? "空"}`);
}

const mismatches = Object.entries(versions).filter(([, value]) => value !== version);
if (mismatches.length > 0) {
  throw new Error(`版本号未同步：\n${Object.entries(versions).map(([file, value]) => `- ${file}: ${value ?? "未找到"}`).join("\n")}`);
}

if (tag) {
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag)) {
    throw new Error(`标签格式应为 vX.Y.Z，例如 v0.1.0；当前为 ${tag}`);
  }
  if (tag !== `v${version}`) {
    throw new Error(`标签 ${tag} 与项目版本 v${version} 不一致`);
  }
}

console.log(`发版检查通过：${Object.keys(versions).length} 处版本均为 ${version}${tag ? `，标签为 ${tag}` : ""}。`);
