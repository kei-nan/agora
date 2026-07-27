/**
 * Polyfills for @polkadot/api on React Native (Hermes has none of these
 * Node/browser globals natively). Order matters — each of these must run
 * before anything that transitively imports @polkadot/api or
 * @polkadot/util-crypto, which is why this file leads with them before the
 * App import below.
 *
 *  1. react-native-get-random-values — provides crypto.getRandomValues(),
 *     which @polkadot/util's RNG helpers require.
 *  2. fast-text-encoding — Hermes has no TextEncoder/TextDecoder; this
 *     assigns polyfilled versions onto global as a side effect of import.
 *  3. buffer — Buffer isn't a JS global outside Node; @polkadot/api and its
 *     dependencies expect it to be. Assigned onto global below.
 *
 * @polkadot/util-crypto's wasm-crypto also needs a WebAssembly global, which
 * Hermes does not provide. wasm-crypto detects this at runtime and falls
 * back to its asm.js build automatically — @polkadot/wasm-crypto-asmjs is
 * listed in package.json so that fallback bundle is present in
 * node_modules for Metro to resolve; no explicit import is needed here.
 */
import 'react-native-get-random-values';
import 'fast-text-encoding';
import { Buffer } from 'buffer';
import { AppRegistry } from 'react-native';
import App from './src/App';

global.Buffer = global.Buffer || Buffer;

AppRegistry.registerComponent('Agora', () => App);
