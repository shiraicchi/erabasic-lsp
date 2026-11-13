import esbuild from 'esbuild';

await esbuild.build({
  entryPoints: ['src/extension.ts'],
  bundle: true,
  outfile: 'out/extension.js',
  platform: 'node',
  target: 'node16',
  external: ['vscode'],
  format: 'cjs',
  sourcemap: true,
  sourcesContent: false,
  banner: {
    js: '/* EraBasic LSP Extension */'
  }
});
