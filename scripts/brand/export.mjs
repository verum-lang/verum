// Rebuild portable brand formats from the approved raster and editable SVG masters.
// Requires Node.js, librsvg (rsvg-convert) and libwebp (cwebp).
import { mkdir, readFile, writeFile, unlink } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../assets/brand/', import.meta.url));
const output = join(root, 'exports');
await mkdir(output, { recursive: true });
function run(command, args) {
  const result = spawnSync(command, args, { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status})`);
}
for (const size of [16, 24, 32, 48, 64, 128, 180, 192, 256, 512]) {
  run('rsvg-convert', ['-w', String(size), '-h', String(size), join(root, 'verum-mark.svg'), '-o', join(output, `verum-mark-${size}.png`)]);
}
const mono = await readFile(join(root, 'verum-mark-mono.svg'), 'utf8');
for (const [theme, colour] of [['light', '#172331'], ['dark', '#eef5ff']]) {
  await writeFile(join(output, `verum-mark-${theme}.svg`), mono.replace('currentColor', colour));
}
run('rsvg-convert', ['-f', 'pdf', join(root, 'verum-mark.svg'), '-o', join(output, 'verum-mark.pdf')]);
run('rsvg-convert', [join(root, 'verum-social.svg'), '-o', join(output, 'social-card.png')]);

// Rendering controls export size without modifying the approved raster master.
const wrapper = join(root, '.emblem-render.svg');
try {
  const master = await readFile(join(root, 'verum-emblem.png'));
  if (!master.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) throw new Error('The emblem master must be PNG');
  const width = master.readUInt32BE(16), height = master.readUInt32BE(20);
  await writeFile(wrapper, `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><image width="${width}" height="${height}" xlink:href="verum-emblem.png"/></svg>`);
  for (const size of [512, 1024]) {
    const png = join(output, `verum-emblem-${size}.png`);
    run('rsvg-convert', ['-w', String(size), '-h', String(size), wrapper, '-o', png]);
    run('cwebp', ['-quiet', '-q', '92', png, '-o', join(output, `verum-emblem-${size}.webp`)]);
  }
} finally {
  await unlink(wrapper);
}

// ICO directory with standard PNG payloads; no platform-specific icon utility.
const sizes = [16, 32, 48, 256];
const images = await Promise.all(sizes.map(size => readFile(join(output, `verum-mark-${size}.png`))));
const header = Buffer.alloc(6 + 16 * images.length);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(images.length, 4);
let offset = header.length;
for (let i = 0; i < images.length; i++) {
  const entry = 6 + 16 * i;
  header[entry] = header[entry + 1] = sizes[i] === 256 ? 0 : sizes[i];
  header.writeUInt16LE(1, entry + 4);
  header.writeUInt16LE(32, entry + 6);
  header.writeUInt32LE(images[i].length, entry + 8);
  header.writeUInt32LE(offset, entry + 12);
  offset += images[i].length;
}
await writeFile(join(output, 'favicon.ico'), Buffer.concat([header, ...images]));
console.log('Exported Verum SVG, PNG, WebP, ICO and PDF assets.');
