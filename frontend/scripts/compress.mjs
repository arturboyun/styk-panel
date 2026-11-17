// Writes .br and .gz next to every compressible file in out/, so styk-panel serves them
// precompressed (ServeDir::precompressed_*) instead of compressing on each request.
import { readdir, readFile, writeFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { brotliCompressSync, gzipSync, constants } from "node:zlib";

const ROOT = new URL("../out/", import.meta.url).pathname;
const EXT = /\.(js|css|html|txt|json|svg|map|xml|webmanifest)$/;

async function* walk(dir) {
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) yield* walk(p);
    else if (EXT.test(e.name)) yield p;
  }
}

let n = 0, raw = 0, br = 0;
for await (const p of walk(ROOT)) {
  if ((await stat(p)).size < 512) continue;
  const buf = await readFile(p);
  const b = brotliCompressSync(buf, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: buf.length } });
  await writeFile(p + ".br", b);
  await writeFile(p + ".gz", gzipSync(buf, { level: 9 }));
  n++; raw += buf.length; br += b.length;
}
console.log(`compressed ${n} files: ${(raw / 1e6).toFixed(2)} MB -> ${(br / 1e6).toFixed(2)} MB brotli`);
