// Usage: node scripts/web-bench.mjs WEB_PKG_DIR FRAMES RUNS BUNDLE...
// Times bundles in the real web runtime (web/pkg) under node, interpreted
// and compiled (best of RUNS; includes instantiation), e.g. with the bundles
// of `cargo run -p amos-build --example bundle_synthetic -- DIR`.
import fs from 'fs';
const [pkg, frames, runs, ...bundles] = process.argv.slice(2);
const { default: init, headless_report } = await import(pkg + '/amos_app.js');
await init({ module_or_path: fs.readFileSync(pkg + '/amos_app_bg.wasm') });
const time = async (data, compiled) => {
  let best = Infinity;
  for (let r = 0; r < Number(runs); r++) {
    const t = performance.now();
    await headless_report(data, Number(frames), compiled);
    best = Math.min(best, performance.now() - t);
  }
  return best;
};
let ti = 0, tc = 0;
for (const b of bundles) {
  const data = new Uint8Array(fs.readFileSync(b));
  const i = await time(data, false);
  const c = await time(data, true);
  ti += i; tc += c;
  const name = b.split('/').pop().replace('.amospak', '');
  console.log(`${name.padEnd(48)} ${i.toFixed(1).padStart(9)} ${c.toFixed(1).padStart(9)} ${(i / c).toFixed(1).padStart(6)}x`);
}
console.log(`total ${ti.toFixed(1)} ${tc.toFixed(1)} ${(ti / tc).toFixed(2)}x`);
