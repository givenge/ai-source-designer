import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { promises as fs } from 'node:fs';
import { createRequire } from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const runtimeDir = path.join(repoRoot, 'src-tauri', 'resources', 'playwright');
const nodeVersion = 'v22.23.4';
// shortcut: bundles the host architecture; add target downloads when cross-compiling.
const archiveName = `node-${nodeVersion}-${process.platform === 'win32' ? 'win' : process.platform}-${process.arch}.${process.platform === 'win32' ? 'zip' : 'tar.gz'}`;
const nodeName = process.platform === 'win32' ? 'node.exe' : 'node';
const nodePath = path.join(runtimeDir, nodeName);
const distUrl = `https://nodejs.org/dist/${nodeVersion}`;

async function download(filename) {
  const response = await fetch(`${distUrl}/${filename}`, {
    signal: AbortSignal.timeout(120_000),
  });
  assert(response.ok, `Failed to download ${filename}: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

let installedRuntime = '';
try {
  installedRuntime = execFileSync(nodePath,
    ['-p', 'process.version + ":" + process.platform + ":" + process.arch'],
    { encoding: 'utf8', stdio: 'pipe' }).trim();
} catch {
  // A missing or incompatible binary is downloaded below.
}
await fs.mkdir(path.join(runtimeDir, 'worker'), { recursive: true });
if (installedRuntime !== `${nodeVersion}:${process.platform}:${process.arch}`) {
  const archive = await download(archiveName);
  const checksums = (await download('SHASUMS256.txt')).toString('utf8');
  const expectedHash = checksums.split('\n')
    .map((line) => line.trim().split(/\s+/))
    .find(([, filename]) => filename === archiveName)?.[0];
  assert.equal(createHash('sha256').update(archive).digest('hex'), expectedHash,
    'Node.js download checksum mismatch');

  const temporaryDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aisd-node-'));
  try {
    const archivePath = path.join(temporaryDir, archiveName);
    await fs.writeFile(archivePath, archive);
    const archiveRoot = archiveName.replace(/\.(zip|tar\.gz)$/, '');
    const binaryMember = `${archiveRoot}/${process.platform === 'win32' ? nodeName : `bin/${nodeName}`}`;
    execFileSync('tar', ['-xf', archivePath, '-C', temporaryDir, binaryMember, `${archiveRoot}/LICENSE`]);
    await fs.copyFile(path.join(temporaryDir, binaryMember), nodePath);
    await fs.copyFile(path.join(temporaryDir, archiveRoot, 'LICENSE'), path.join(runtimeDir, 'NODE-LICENSE'));
    await fs.chmod(nodePath, 0o755);
  } finally {
    await fs.rm(temporaryDir, { recursive: true, force: true });
  }
}

const require = createRequire(import.meta.url);
const playwrightRequire = createRequire(require.resolve('playwright/package.json'));
for (const name of ['playwright', 'playwright-core']) {
  const packageDir = path.dirname(playwrightRequire.resolve(`${name}/package.json`));
  const destination = path.join(runtimeDir, 'node_modules', name);
  await fs.rm(destination, { recursive: true, force: true });
  await fs.cp(packageDir, destination, { recursive: true, dereference: true });
}
await fs.copyFile(path.join(repoRoot, 'playwright', 'worker', 'index.mjs'),
  path.join(runtimeDir, 'worker', 'index.mjs'));

const runtimeEnv = {
  ...process.env,
  PLAYWRIGHT_BROWSERS_PATH: path.join(runtimeDir, 'browsers'),
};
execFileSync(nodePath, [path.join(runtimeDir, 'node_modules', 'playwright', 'cli.js'),
  'install', 'chromium', '--only-shell'], { env: runtimeEnv, stdio: 'inherit' });
execFileSync(nodePath, ['--input-type=module', '-e', `
  import assert from 'node:assert/strict';
  import { chromium } from 'playwright';
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<title>worker-runtime</title>');
    assert.equal(await page.title(), 'worker-runtime');
  } finally {
    await browser.close();
  }
`], { cwd: runtimeDir, env: runtimeEnv, stdio: 'inherit' });
console.log(`Bundled and verified Node.js ${nodeVersion}, Playwright and Chromium: ${runtimeDir}`);
