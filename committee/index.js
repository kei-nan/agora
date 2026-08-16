/**
 * Polyfills for @polkadot/api on React Native. Mirrors `mobile/index.js` exactly —
 * same polyfills, same ordering requirement (must run before anything that
 * transitively imports @polkadot/api or @polkadot/util-crypto). See that file's doc
 * comment for the full rationale; it applies unchanged here.
 *
 * `import 'react-native-get-random-values'` also does double duty for
 * `src/crypto/wasmCommitteeCrypto.ts` (changelog #084): that module sources its
 * Chaum-Pedersen proof nonce from `crypto.getRandomValues`, which this import is what
 * actually installs on `global` in a real RN app — see that file's `freshSeed()` doc
 * comment for why it doesn't re-import the polyfill itself.
 *
 * No native `android/`/`ios/` project has been scaffolded for this app yet (out of
 * scope for this task), so `AppRegistry.registerComponent` below has nothing to
 * attach to today — this file exists so the entry point is in place once that native
 * scaffolding lands, matching `mobile/index.js`'s shape so the two apps stay easy to
 * compare.
 */
import 'react-native-get-random-values';
import 'fast-text-encoding';
import { Buffer } from 'buffer';
import { AppRegistry } from 'react-native';
import App from './src/App';
import { setCommitteeCrypto } from './src/crypto/CommitteeCrypto';
import { wasmCommitteeCrypto } from './src/crypto/wasmCommitteeCrypto';

global.Buffer = global.Buffer || Buffer;

// Installs the real OPRF crypto core (changelog #084) — without this, every
// `submitRound1`/`submitRound2` call (`chain/oprfCommittee.ts`) would still hit
// `notImplementedCommitteeCrypto`'s throwing stub.
setCommitteeCrypto(wasmCommitteeCrypto);

AppRegistry.registerComponent('AgoraCommittee', () => App);
