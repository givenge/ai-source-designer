import { promises as fs } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');
const workerScript = path.join(repoRoot, 'playwright', 'worker', 'index.mjs');
const fixturesDir = path.join(repoRoot, 'fixtures');

function usage() {
  console.error(
    'Usage: pnpm smoke:live -- <url> [--out artifacts/live-smoke/<name>] [--label smoke-name]',
  );
}

function parseArgs(argv) {
  const args = argv.slice(2);
  const config = {
    url: '',
    outDir: '',
    label: '',
  };

  for (let index = 0; index < args.length; index += 1) {
    const value = args[index];
    if (!config.url && !value.startsWith('--')) {
      config.url = value;
      continue;
    }

    if (value === '--out') {
      config.outDir = args[index + 1] ?? '';
      index += 1;
      continue;
    }

    if (value === '--label') {
      config.label = args[index + 1] ?? '';
      index += 1;
    }
  }

  return config;
}

function timestamp() {
  return new Date().toISOString().replaceAll(':', '-');
}

async function runWorker(url, outputDir) {
  const payload = JSON.stringify({
    id: 1,
    method: 'sampleSite',
    params: {
      targetUrl: url,
      outputDir,
      fixturesDir,
    },
  });

  const child = spawn('node', [workerScript], {
    cwd: repoRoot,
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  let stdout = '';
  let stderr = '';

  child.stdout.on('data', (chunk) => {
    stdout += chunk.toString();
  });

  child.stderr.on('data', (chunk) => {
    stderr += chunk.toString();
  });

  child.stdin.write(payload);
  child.stdin.end();

  const exitCode = await new Promise((resolve) => {
    child.on('close', resolve);
  });

  if (exitCode !== 0) {
    throw new Error(stderr || stdout || `worker exited with code ${exitCode}`);
  }

  return JSON.parse(stdout);
}

function buildModelPrompt(sample) {
  const pages = sample.pages
    .map((page) => `- role=${page.role} url=${page.url} html=${page.htmlPath ?? 'n/a'}`)
    .join('\n');
  const network = (sample.networkSamples ?? [])
    .map((item) => `- ${item.method} ${item.requestUrl} -> ${item.responsePath}`)
    .join('\n');

  return [
    'You are checking whether a sampled public website looks promising for AI Source Designer.',
    'Return JSON only with keys:',
    'siteLikelyCompatible (boolean), confidence (number), notes (string[]), nextSteps (string[]), possibleSearchSignals (string[]).',
    `Base URL: ${sample.baseUrl}`,
    `Pages:\n${pages || '- none'}`,
    `Network:\n${network || '- none'}`,
  ].join('\n');
}

async function runModelSmoke(sample, outputDir) {
  const baseUrl = process.env.AI_SOURCE_DESIGNER_BASE_URL;
  const model = process.env.AI_SOURCE_DESIGNER_MODEL;
  const apiKey = process.env.AI_SOURCE_DESIGNER_API_KEY;

  if (!baseUrl || !model || !apiKey) {
    return {
      skipped: true,
      reason:
        'Missing AI_SOURCE_DESIGNER_BASE_URL / AI_SOURCE_DESIGNER_MODEL / AI_SOURCE_DESIGNER_API_KEY',
    };
  }

  const response = await fetch(`${baseUrl.replace(/\/$/, '')}/chat/completions`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model,
      temperature: 0.1,
      response_format: { type: 'json_object' },
      messages: [
        {
          role: 'system',
          content: 'Return strict JSON only.',
        },
        {
          role: 'user',
          content: buildModelPrompt(sample),
        },
      ],
    }),
  });

  if (!response.ok) {
    throw new Error(`model smoke failed: ${response.status} ${await response.text()}`);
  }

  const json = await response.json();
  const content = json?.choices?.[0]?.message?.content ?? '';
  const parsed = JSON.parse(content);
  const filePath = path.join(outputDir, 'model-smoke.json');
  await fs.writeFile(filePath, JSON.stringify(parsed, null, 2), 'utf8');
  return {
    skipped: false,
    outputFile: filePath,
    result: parsed,
  };
}

async function main() {
  const { url, outDir, label } = parseArgs(process.argv);

  if (!url) {
    usage();
    process.exit(1);
  }

  const outputDir =
    outDir || path.join(repoRoot, 'artifacts', 'live-smoke', label || timestamp());
  await fs.mkdir(outputDir, { recursive: true });

  console.log(`Sampling ${url}`);
  console.log(`Artifacts => ${outputDir}`);

  const workerResponse = await runWorker(url, outputDir);
  if (workerResponse.error) {
    throw new Error(workerResponse.error);
  }

  const sample = workerResponse.result;
  const sampleSummaryPath = path.join(outputDir, 'sample-summary.json');
  await fs.writeFile(sampleSummaryPath, JSON.stringify(sample, null, 2), 'utf8');

  console.log(`Sampled ${sample.pages.length} page(s), ${sample.networkSamples.length} network sample(s).`);
  console.log(`Sample summary => ${sampleSummaryPath}`);

  const smoke = await runModelSmoke(sample, outputDir);
  if (smoke.skipped) {
    console.log(`Model smoke skipped: ${smoke.reason}`);
  } else {
    console.log(`Model smoke result => ${smoke.outputFile}`);
    console.log(JSON.stringify(smoke.result, null, 2));
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
