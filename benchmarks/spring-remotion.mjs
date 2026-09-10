import {registerHooks} from 'node:module';
import {existsSync} from 'node:fs';
import {fileURLToPath, pathToFileURL} from 'node:url';
const root = pathToFileURL(process.cwd() + '/vendor/remotion-4.0.495/packages/core/src/');
registerHooks({resolve(specifier, context, nextResolve) {
  if (specifier.startsWith('.') && context.parentURL?.startsWith(root.href)) {
    const url = new URL(specifier, context.parentURL);
    const path = fileURLToPath(url).replace(/\.js$/, '.ts');
    if (existsSync(path)) return nextResolve(pathToFileURL(path).href, context);
    if (existsSync(path + '.ts')) return nextResolve(pathToFileURL(path + '.ts').href, context);
  }
  return nextResolve(specifier, context);
}});
const {spring, measureSpring} = await import(new URL('spring/index.ts', root));
const [scenario, count, preheat] = process.argv.slice(2);
const iterations = Number(count), warmup = Number(preheat);
function sample(index) {
  const config = {damping:10, mass:1, stiffness:100, overshootClamping:false};
  if (scenario === 'varied') {
    config.damping = 6 + (index % 32) * 0.25;
    config.mass = 1 + (index % 4) * 0.1;
    config.stiffness = 80 + (index % 8) * 10;
  }
  switch (scenario) {
    case 'varied': return spring({frame:index % 120, fps:30, config, durationInFrames:90});
    case 'repeat': return spring({frame:60, fps:30, config});
    case 'cold':
    case 'sequential': return spring({frame:index % 300, fps:30, config});
    case 'duration': return spring({frame:index % 300, fps:30, config, durationInFrames:60, delay:12});
    case 'seek': return spring({frame:((index * 137) % 600) + 0.5, fps:30, config});
    case 'measure': return measureSpring({fps:30, config, threshold:0.005});
    default: throw new Error('unknown scenario');
  }
}
let warmChecksum = 0;
for (let i=0; i<warmup; i++) warmChecksum += sample(i);
const start = process.hrtime.bigint();
let checksum = 0;
for (let i=0; i<iterations; i++) checksum += sample(i);
const ns = Number(process.hrtime.bigint() - start);
if (!Number.isFinite(warmChecksum)) throw new Error('invalid warmup');
console.log(JSON.stringify({ns_per_call:ns/iterations, checksum}));
