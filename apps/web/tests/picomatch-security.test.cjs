const { test } = require('node:test')
const assert = require('node:assert/strict')
const { spawnSync } = require('node:child_process')
const { createRequire } = require('node:module')
const picomatch = require('picomatch')

test('every tooling dependency resolves to the patched matcher', () => {
  const root = require.resolve('picomatch/lib/parse')
  for (const dependency of ['vite', 'tinyglobby', 'fdir', 'micromatch', '@dotenvx/dotenvx']) {
    assert.equal(createRequire(require.resolve(dependency)).resolve('picomatch/lib/parse'), root, dependency)
  }
})

test('single-branch repeated wildcards are literalized and cannot stall matching', () => {
  const result = spawnSync(process.execPath, ['-e', `
    const assert = require('node:assert/strict');
    const pm = require('picomatch');
    for (const pattern of ['+(*)', '*(*)', '+(*a)', '*(*a)', '+(*?)', '+(@(*a))']) {
      for (const options of [{}, { capture: true }, { maxExtglobRecursion: false }]) {
        const matcher = pm(pattern, options);
        assert.equal(matcher('a'.repeat(100) + '!'), false, pattern);
        assert.equal(matcher(pattern), true, pattern);
      }
    }
  `], { cwd: process.cwd(), encoding: 'utf8', timeout: 3000 })
  assert.ifError(result.error)
  assert.equal(result.status, 0, result.stderr)
})

test('ordinary globs and literal extglobs retain matching behavior', () => {
  for (const [pattern, input, expected] of [
    ['src/**/*.tsx', 'src/components/button.tsx', true],
    ['src/**/*.tsx', 'src/main.rs', false],
    ['*.{ts,tsx}', 'main.tsx', true],
    ['+(ab)', 'abab', true],
    ['@(foo|bar)', 'bar', true],
    ['!(*.test).ts', 'main.ts', true],
    ['!(*.test).ts', 'main.test.ts', false],
  ]) assert.equal(picomatch.isMatch(input, pattern), expected, pattern)
  assert.deepEqual(require('micromatch')(['a.ts', 'b.tsx', 'c.rs'], '*.{ts,tsx}'), ['a.ts', 'b.tsx'])
  const { Project } = require('ts-morph')
  const project = new Project({ useInMemoryFileSystem: true })
  project.createSourceFile('/src/example.ts', 'export const value = 1')
  assert.equal(project.getSourceFiles('/src/**/*.ts').length, 1)
})
