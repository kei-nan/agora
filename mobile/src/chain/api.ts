import { ApiPromise, WsProvider } from '@polkadot/api';

const NODE_WS = process.env.NODE_WS ?? 'ws://127.0.0.1:9944';

let _api: ApiPromise | null = null;

export async function getApi(): Promise<ApiPromise> {
  if (_api?.isConnected) return _api;
  const provider = new WsProvider(NODE_WS);
  _api = await ApiPromise.create({ provider });
  return _api;
}

export async function disconnect() {
  await _api?.disconnect();
  _api = null;
}
