import { build } from 'esbuild';
import { readFile, writeFile, mkdir, copyFile, readdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';

await mkdir('dist', { recursive: true });
const result = await build({
  entryPoints: ['src/main.ts'], bundle: true, minify: true,
  target: ['es2022'], outfile: 'dist/app.js', metafile: true,
  legalComments: 'external', charset: 'utf8',
});
await copyFile('index.html', 'dist/index.html');
// Ship the licenses of every dependency actually included in the offline bundle.
const packages = new Map();
for (const input of Object.keys(result.metafile.inputs)) {
  if (!input.includes('node_modules/')) continue;
  let directory = dirname(resolve(input));
  while (directory !== dirname(directory)) {
    try {
      const pkg = JSON.parse(await readFile(join(directory, 'package.json'), 'utf8'));
      if (pkg.name && pkg.version) { packages.set(pkg.name, { directory, pkg }); break; }
    } catch { /* Walk from bundled module to its package root. */ }
    directory = dirname(directory);
  }
}
const notices = ['Verum Workbench — bundled third-party notices\n'];
for (const [, { directory, pkg }] of [...packages].sort()) {
  const licenseFiles = (await readdir(directory)).filter(name => /^(license|copying)(\.|$)/i.test(name));
  if (!licenseFiles.length) throw new Error(`Missing license file for ${pkg.name}`);
  notices.push(`${pkg.name} ${pkg.version} (${pkg.license ?? 'see license'})\n${'='.repeat(72)}`);
  for (const file of licenseFiles) notices.push(await readFile(join(directory, file), 'utf8'));
}
await writeFile('dist/licenses.txt', notices.join('\n\n'));
