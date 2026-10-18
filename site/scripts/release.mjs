// Writes site/data/release.json from the latest GitHub release, so the
// download buttons, sizes and file names follow each release.
// On any failure it keeps the committed file and says so: a build must not
// fail because the GitHub API is slow or rate limited.
// Usage: node site/scripts/release.mjs [--strict]
import { readFile, writeFile } from "node:fs/promises";

const repo = process.env.RENDERD_REPO || "Ad1th/Renderd";
const out = new URL("../data/release.json", import.meta.url);
const strict = process.argv.includes("--strict");

const want = {
  mac: /^renderd-host-.*-aarch64-apple-darwin\.tar\.gz$/,
  win_x64: /^renderd-viewer-.*-x86_64-pc-windows-msvc\.zip$/,
  win_arm64: /^renderd-viewer-.*-aarch64-pc-windows-msvc\.zip$/,
  sums: /^SHA256SUMS\.txt$/,
};

async function latest() {
  // For tests: read a saved API answer instead of asking GitHub.
  if (process.env.RENDERD_RELEASE_FIXTURE) return JSON.parse(await readFile(process.env.RENDERD_RELEASE_FIXTURE, "utf8"));
  const headers = { Accept: "application/vnd.github+json", "User-Agent": "renderd-site" };
  if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  const res = await fetch(`https://api.github.com/repos/${repo}/releases/latest`, {
    headers,
    signal: AbortSignal.timeout(15000),
  });
  if (!res.ok) throw new Error(`GitHub API answered ${res.status}`);
  return res.json();
}

function toData(r) {
  const files = {};
  for (const [key, re] of Object.entries(want)) {
    const a = r.assets.find((x) => re.test(x.name));
    if (!a) throw new Error(`release ${r.tag_name} has no asset for ${key}`);
    files[key] = { name: a.name, url: a.browser_download_url };
    if (key !== "sums") files[key].size = a.size;
  }
  const date = new Date(r.published_at);
  return {
    tag: r.tag_name,
    version: r.tag_name.replace(/^v/, ""),
    date: date.toISOString().slice(0, 10),
    date_human: date.toLocaleDateString("en-GB", { day: "numeric", month: "long", year: "numeric", timeZone: "UTC" }),
    notes_url: r.html_url,
    ...files,
  };
}

try {
  const data = toData(await latest());
  const json = JSON.stringify(data, null, 2) + "\n";
  const old = await readFile(out, "utf8").catch(() => "");
  if (old !== json) await writeFile(out, json);
  console.log(`release.json: ${data.tag}${old === json ? " (no change)" : " (updated)"}`);
} catch (err) {
  console.warn(`release.json: kept the committed file (${err.message})`);
  if (strict) process.exit(1);
}
