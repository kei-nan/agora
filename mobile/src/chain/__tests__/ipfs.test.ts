/**
 * Tests `ipfs.ts` — the mobile-side port of desktop's `fetch_ipfs_content` /
 * `fetch_ipfs_content_from` (`desktop/src-tauri/src/commands/chain.rs`). Mirrors that
 * file's own test suite: a CID-derivation known-answer test, a success path where fetched
 * content matches the on-chain hash, and the security-relevant rejection path where it
 * doesn't.
 *
 * `global.fetch` is mocked throughout — these tests never touch the real network.
 */
import { Buffer } from 'buffer';
import { sha256 } from '@noble/hashes/sha2';
import {
  IpfsContentError,
  IpfsGatewayError,
  IpfsHashMismatchError,
  IpfsSizeCapExceededError,
  MAX_IPFS_CONTENT_BYTES,
  fetchIpfsContent,
} from '../ipfs';

/** Builds a minimal fake `Response` backed by `arrayBuffer()` only (no streaming `body`) — matching what React Native's `fetch` polyfill actually exposes, per `ipfs.ts`'s `readCappedBody` doc comment. */
function fakeResponse(options: {
  ok?: boolean;
  status?: number;
  bodyBytes: Uint8Array;
  contentLength?: string | null;
}): Response {
  const { ok = true, status = 200, bodyBytes, contentLength } = options;
  return {
    ok,
    status,
    headers: {
      get: (name: string) => (name.toLowerCase() === 'content-length' ? contentLength ?? null : null),
    },
    arrayBuffer: async () => bodyBytes.buffer.slice(bodyBytes.byteOffset, bodyBytes.byteOffset + bodyBytes.byteLength),
  } as unknown as Response;
}

function hexOf(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString('hex');
}

afterEach(() => {
  jest.restoreAllMocks();
});

describe('fetchIpfsContent', () => {
  it('derives the correct CIDv0 from a known 32-byte hash and requests it from the gateway', async () => {
    // Known-answer vector computed independently (sha256("the actual law text"), then
    // base58btc-encoded with the [0x12, 0x20] sha2-256/32-byte multihash prefix) —
    // matches the CIDv0 desktop's own `hash_to_cid` test fixture would produce for the
    // same input, since both are the identical CIDv0 algorithm.
    const body = 'the actual law text';
    const bodyBytes = new TextEncoder().encode(body);
    const hashBytes = sha256(bodyBytes);
    const hashHex = hexOf(hashBytes);
    const expectedCid = 'QmagUug15yfBFrQzoJD7xfvD1KGGD723Z7Uw2nNXrFLEux';

    let requestedUrl: string | undefined;
    (global as any).fetch = jest.fn(async (url: string) => {
      requestedUrl = url;
      return fakeResponse({ bodyBytes });
    });

    const result = await fetchIpfsContent(hashHex, 'https://example-gateway.test/ipfs');

    expect(requestedUrl).toBe(`https://example-gateway.test/ipfs/${expectedCid}`);
    expect(result).toBe(body);
  });

  it('accepts content whose SHA-256 digest matches the requested on-chain hash', async () => {
    const body = 'Referendum #4: raise the department budget cap by 10%.';
    const bodyBytes = new TextEncoder().encode(body);
    const hashHex = hexOf(sha256(bodyBytes));

    (global as any).fetch = jest.fn(async () => fakeResponse({ bodyBytes }));

    await expect(fetchIpfsContent(hashHex)).resolves.toBe(body);
  });

  it('rejects content that does not match the on-chain hash (compromised/lying gateway)', async () => {
    const realBody = 'the real, honest law text';
    const substitutedBody = 'attacker-substituted content from a compromised gateway';
    // Hash corresponds to `realBody`, but the mock gateway serves `substitutedBody` —
    // the same mismatch scenario desktop's `fetch_ipfs_content_rejects_content_not_matching_the_on_chain_hash` covers.
    const hashHex = hexOf(sha256(new TextEncoder().encode(realBody)));

    (global as any).fetch = jest.fn(async () =>
      fakeResponse({ bodyBytes: new TextEncoder().encode(substitutedBody) }),
    );

    await expect(fetchIpfsContent(hashHex)).rejects.toThrow(IpfsHashMismatchError);
  });

  it('rejects a response exceeding the size cap, without ever returning the oversized content', async () => {
    const oversized = new Uint8Array(MAX_IPFS_CONTENT_BYTES + 1);
    // hashHex is irrelevant here — the cap must reject before hash verification runs.
    const hashHex = hexOf(sha256(oversized));

    (global as any).fetch = jest.fn(async () => fakeResponse({ bodyBytes: oversized }));

    await expect(fetchIpfsContent(hashHex)).rejects.toThrow(IpfsSizeCapExceededError);
  });

  it('fast-path rejects when the gateway declares an over-cap Content-Length, without reading the body', async () => {
    const bodyBytes = new TextEncoder().encode('short');
    const arrayBufferSpy = jest.fn(async () => bodyBytes.buffer);
    (global as any).fetch = jest.fn(async () => ({
      ok: true,
      status: 200,
      headers: { get: (name: string) => (name.toLowerCase() === 'content-length' ? String(MAX_IPFS_CONTENT_BYTES + 1) : null) },
      arrayBuffer: arrayBufferSpy,
    }));

    await expect(fetchIpfsContent(hexOf(sha256(bodyBytes)))).rejects.toThrow(IpfsSizeCapExceededError);
    expect(arrayBufferSpy).not.toHaveBeenCalled();
  });

  it('surfaces a distinguishable gateway error on a non-2xx response', async () => {
    (global as any).fetch = jest.fn(async () =>
      fakeResponse({ ok: false, status: 504, bodyBytes: new Uint8Array() }),
    );

    await expect(fetchIpfsContent('00'.repeat(32))).rejects.toThrow(IpfsGatewayError);
  });

  it('surfaces a distinguishable gateway error when the network request itself fails', async () => {
    (global as any).fetch = jest.fn(async () => {
      throw new TypeError('Network request failed');
    });

    await expect(fetchIpfsContent('00'.repeat(32))).rejects.toThrow(IpfsGatewayError);
  });

  it('rejects a malformed (non-32-byte) hash before ever calling fetch', async () => {
    const fetchSpy = jest.fn();
    (global as any).fetch = fetchSpy;

    await expect(fetchIpfsContent('deadbeef')).rejects.toThrow(IpfsContentError);
    expect(fetchSpy).not.toHaveBeenCalled();
  });
});
