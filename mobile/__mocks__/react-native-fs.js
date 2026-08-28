/**
 * Manual Jest mock for 'react-native-fs', auto-applied the same way as
 * `./react-native.js` (see that file's doc comment) — required separately
 * because the real react-native-fs reads properties off
 * `require('react-native').NativeModules.RNFSManager` at *import* time
 * (`FS.common.js`: `RNFSFileTypeRegular = RNFSManager.RNFSFileTypeRegular`),
 * which throws on `undefined.RNFSFileTypeRegular` against our minimal
 * `react-native.js` mock's empty `NativeModules` — mocking 'react-native'
 * alone isn't enough to make 'react-native-fs' importable under Jest.
 *
 * Backs `DocumentDirectoryPath` and `CachesDirectoryPath` with a single plain
 * in-memory `Map`, keyed by path — enough to exercise `keystoreWallet.ts`'s
 * and `registrationState.ts`'s exists/readFile/writeFile round-trips. The
 * `Map` lives on `globalThis`, not a module-local variable,
 * specifically so it survives `jest.resetModules()` — tests that simulate
 * an app restart (`keystoreWallet.test.ts`) reset the module registry to
 * clear `keystoreWallet.ts`'s in-memory keypair cache while expecting its
 * *persisted file* to still be there, the same way a real restart clears JS
 * memory but not disk. Call `require('react-native-fs').__reset()` between
 * tests that want a clean filesystem.
 */
function backingStore() {
  if (!globalThis.__mockRNFSFiles) {
    globalThis.__mockRNFSFiles = new Map();
  }
  return globalThis.__mockRNFSFiles;
}

module.exports = {
  DocumentDirectoryPath: '/mock-documents',
  CachesDirectoryPath: '/mock-caches',
  exists: jest.fn((path) => Promise.resolve(backingStore().has(path))),
  readFile: jest.fn((path) => {
    const files = backingStore();
    if (!files.has(path)) return Promise.reject(new Error(`ENOENT: ${path}`));
    return Promise.resolve(files.get(path));
  }),
  writeFile: jest.fn((path, contents) => {
    backingStore().set(path, contents);
    return Promise.resolve();
  }),
  unlink: jest.fn((path) => {
    backingStore().delete(path);
    return Promise.resolve();
  }),
  __reset: () => {
    globalThis.__mockRNFSFiles = new Map();
  },
};
