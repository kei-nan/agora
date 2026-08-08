// Tests that exercise module-scope caching (keystoreSigner.test.ts,
// keystoreWallet.test.ts, identity.test.ts's signing-key-selection block)
// use `jest.resetModules()` + re-require to get a fresh module instance per
// scenario — see those files' doc comments. That re-registers @polkadot/*
// packages with `detectPackage`'s internal counter every time, which by
// design logs a "multiple versions" `console.warn` on every re-registration
// past the first — even though it's the exact same version every time, just
// re-imported. This flag (documented in @polkadot/util's own
// detectPackage.js) suppresses that specific same-version-repeat case
// without hiding a real mismatched-version warning if one ever occurs.
process.env.POLKADOTJS_DISABLE_ESM_CJS_WARNING = '1';

module.exports = {
  testEnvironment: 'node',
  testMatch: ['<rootDir>/src/**/*.test.ts'],
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'json', 'node'],
  transform: {
    '^.+\\.[tj]sx?$': 'babel-jest',
  },
  // React Native's own global — real app code (api.ts, RegisterScreen.tsx,
  // identity.ts) reads it directly with no import, same as at runtime.
  // Defaults `true` (a dev build) here since that's the common case for
  // these tests; individual tests that need to exercise release-build
  // behavior (e.g. identity.ts's dev-mnemonic-fallback gate) flip
  // `(global as any).__DEV__ = false` themselves and restore it afterward.
  globals: {
    __DEV__: true,
  },
};
