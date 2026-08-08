/**
 * Polyfills for @polkadot/api on React Native. Mirrors `mobile/index.js` exactly —
 * same polyfills, same ordering requirement (must run before anything that
 * transitively imports @polkadot/api or @polkadot/util-crypto). See that file's doc
 * comment for the full rationale; it applies unchanged here.
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

global.Buffer = global.Buffer || Buffer;

AppRegistry.registerComponent('AgoraCommittee', () => App);
