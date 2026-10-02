// Usage: node scripts/web-headless-compare.mjs WEB_PKG_DIR FRAMES BUNDLE...
// Runs bundles headless in the real web runtime (web/pkg), compiled and
// interpreted, and compares the reports.
import fs from 'fs';
const [pkg, frames, ...bundles] = process.argv.slice(2);
const { default: init, headless_report } = await import(pkg + '/amos_app.js');
await init({ module_or_path: fs.readFileSync(pkg + '/amos_app_bg.wasm') });
let bad = 0;
for (const b of bundles) {
  const data = new Uint8Array(fs.readFileSync(b));
  const c = await headless_report(data, Number(frames), true);
  const i = await headless_report(data, Number(frames), false);
  const same = c.split('\n').slice(1).join('\n') === i.split('\n').slice(1).join('\n');
  if (!same) bad++;
  console.log((same ? 'SAME ' : 'DIFF ') + c.split('\n')[0] + ' ' + b.split('/').pop() + (same ? '' : '\n' + c + '\n---\n' + i));
}
console.log(bad ? `${bad} different` : 'all identical');
