/**
 * Manual Jest mock for the 'react-native' package. Automatically substituted
 * for ANY test that transitively imports 'react-native' — Jest applies a
 * `__mocks__/<name>.js` file adjacent to `node_modules` for node_modules
 * packages without needing a `jest.mock('react-native')` call in the test
 * file itself (unlike mocks for this codebase's own relative-path modules).
 *
 * This exists because this repo's jest.config.js runs under a plain 'node'
 * testEnvironment with no react-native preset, and the real 'react-native'
 * package can't be loaded under that at all — its entry point uses Flow
 * syntax Jest's default transform doesn't handle (confirmed directly: a
 * throwaway test importing the real package failed with "Cannot use import
 * statement outside a module" pointing at `node_modules/react-native/index.js`
 * before this mock existed).
 *
 * Deliberately minimal: only what this codebase's native-module bridge
 * wrappers actually touch (`src/native/nfcPassportReader.ts`,
 * `src/native/keystoreSigner.ts`). `NativeModules` starts empty each test
 * file gets its own fresh module registry/global scope, so mutations one
 * test makes (e.g. `NativeModules.KeystoreSigningModule = {...}`) don't leak
 * into other test *files*, but do leak between tests within the same file —
 * tests that add to it should remove what they added in an `afterEach`.
 */
const NativeModules = {};

const Platform = {
  OS: 'android',
  select: (spec) => (Object.prototype.hasOwnProperty.call(spec, Platform.OS) ? spec[Platform.OS] : spec.default),
};

module.exports = { NativeModules, Platform };
