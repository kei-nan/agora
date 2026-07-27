const { getDefaultConfig, mergeConfig } = require('@react-native/metro-config');

/**
 * @polkadot/api and friends assume a couple of Node-shaped modules exist.
 * `buffer` is a real dependency (see package.json) providing the Buffer
 * polyfill wired up in index.js; mapping it here too means any transitive
 * `require('buffer')` inside node_modules resolves to the same polyfill
 * instead of Metro trying (and failing) to resolve Node's built-in module.
 */
/** @type {import('@react-native/metro-config').MetroConfig} */
const config = {
  resolver: {
    extraNodeModules: {
      buffer: require.resolve('buffer'),
    },
  },
};

module.exports = mergeConfig(getDefaultConfig(__dirname), config);
