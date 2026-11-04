import { build, context } from 'esbuild';

const watching = process.argv.includes('--watch');

/** @type {import('esbuild').BuildOptions} */
const options = {
  entryPoints: ['src/extension.ts'],
  bundle: true,
  external: ['vscode'],
  format: 'cjs',
  platform: 'node',
  target: 'node18',
  outfile: 'dist/extension.js',
  sourcemap: true,
  logLevel: 'info',
};

if (watching) {
  const ctx = await context(options);
  await ctx.watch();
} else {
  await build(options);
}
