# Temporary picomatch mitigation

MIT-licensed npm picomatch 4.0.7 runtime sources. The only runtime change is
the guard at the start of analyzeRepeatedExtglob in lib/parse.js.

GHSA-fc5q-39xc-vjpf affects all published versions through 4.0.7; no patched
release was available on 18 November 2026. The 11 high npm findings all stemmed
from this advisory in the Vite/shadcn tooling dependency chains.

Repeated +(…) and *(…) extglobs with a single branch containing * or ? are
conservatively literalized via the existing risky-extglob fallback, even when
maxExtglobRecursion is disabled. This intentionally narrows advanced glob
semantics. Ordinary file globs, nonrepeated extglobs and literal-only repetition
remain supported. Existing multibranch guards remain unchanged. This mitigates
the reported single-branch issue; arbitrary JavaScript regex is not guaranteed safe.

The local dev dependency and override route picomatch 2 and 4 consumers here.
The upstream version is retained for peer compatibility. npm audit does not scan
vendored source: its zero result is not proof of the patch. CI runs
npm run test:security: timeout-isolated exploit regressions, resolution checks
and glob consumer compatibility checks.

Replace this copy and remove the override once a reviewed upstream release fixes
the advisory, then rerun regression tests and build.

Source: https://www.npmjs.com/package/picomatch/v/4.0.7
Advisory: https://github.com/advisories/GHSA-fc5q-39xc-vjpf
