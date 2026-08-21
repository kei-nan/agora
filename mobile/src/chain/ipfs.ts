/**
 * IPFS content fetching for law/proposal/ruling text, addressed by the on-chain SHA-256
 * hash pallet-constitution / pallet-voting / pallet-courts actually store (`contentHash`,
 * `topicHash`, `ipfsHash`). Mobile has no Tauri/Rust backend, so this is a direct
 * TypeScript port of desktop's `fetch_ipfs_content` / `fetch_ipfs_content_from`
 * (`desktop/src-tauri/src/commands/chain.rs`) rather than a call into desktop's code —
 * same algorithm, same default gateway, same trust boundary.
 *
 * TRUST BOUNDARY (mirrors desktop's own doc comment): an IPFS gateway is just an
 * untrusted HTTP relay — it is not obligated to (and a malicious or compromised one will
 * not) actually return the content addressed by the requested CID. Before this returns
 * content to a caller, it recomputes the SHA-256 digest of the fetched bytes and checks
 * it against the hash the CID was derived from. A mismatch is a hard error, never
 * degraded/unverified content passed through silently — "hash-verified" is the actual
 * trust boundary here, not "got a 200 response."
 *
 * CIDv0 encoding is not reimplemented here — `hashToCid` in zkProving.ts is already a
 * direct port of desktop's `hash_to_cid` (same multihash prefix, same bs58 encoding), so
 * this reuses it rather than duplicating that logic a second time in this file.
 */
import { hexToU8a, u8aEq } from '@polkadot/util';
import { sha256 } from '@noble/hashes/sha2';
import { DEFAULT_IPFS_GATEWAY, hashToCid } from './zkProving';

/**
 * Cap on how much of a gateway response this will buffer. Law and proposal text (plus a
 * ruling's reasoning) is realistically well under this; 8 MiB gives generous headroom for
 * long constitutional documents while still bounding memory use against a malicious or
 * misbehaving gateway trying to stream an unbounded response. Matches desktop's
 * `MAX_IPFS_CONTENT_BYTES`.
 */
export const MAX_IPFS_CONTENT_BYTES = 8 * 1024 * 1024;

/** Matches desktop's 30s `reqwest` client timeout for the gateway fetch. */
export const IPFS_FETCH_TIMEOUT_MS = 30_000;

/** Base error type for every failure this module can throw — lets callers do a single `instanceof` check if they only care that content didn't come back verified, or narrow to a specific subtype for a more precise message. */
export class IpfsContentError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'IpfsContentError';
  }
}

/** The gateway was unreachable, timed out, or returned a non-2xx status. */
export class IpfsGatewayError extends IpfsContentError {
  constructor(message: string) {
    super(message);
    this.name = 'IpfsGatewayError';
  }
}

/** The gateway response exceeded {@link MAX_IPFS_CONTENT_BYTES}, declared up front via `Content-Length` or discovered while reading the body. */
export class IpfsSizeCapExceededError extends IpfsContentError {
  constructor(message: string) {
    super(message);
    this.name = 'IpfsSizeCapExceededError';
  }
}

/**
 * The fetched bytes' SHA-256 digest did not match the on-chain hash the CID was derived
 * from — the core case this module exists to catch. The gateway may be malicious,
 * compromised, or just serving stale/wrong data; either way the content is untrustworthy
 * and must not be shown to the citizen as if it were the real, hash-verified law/proposal
 * text.
 */
export class IpfsHashMismatchError extends IpfsContentError {
  constructor(message: string) {
    super(message);
    this.name = 'IpfsHashMismatchError';
  }
}

/**
 * Fetches and hash-verifies IPFS content for a law, proposal, or ruling by its on-chain
 * SHA-256 hash (hex, with or without a `0x` prefix). Converts the 32-byte digest to a
 * CIDv0 and fetches from `gatewayUrl` (the public gateway by default).
 *
 * Throws (never silently degrades to unverified content or a fabricated placeholder):
 *  - {@link IpfsContentError} directly, if `hashHex` isn't a well-formed 32-byte hash
 *  - {@link IpfsGatewayError} on network failure, timeout, or a non-2xx response
 *  - {@link IpfsSizeCapExceededError} if the response exceeds {@link MAX_IPFS_CONTENT_BYTES}
 *  - {@link IpfsHashMismatchError} if the fetched bytes don't hash to `hashHex`
 *  - {@link IpfsContentError} if the (hash-verified) bytes aren't valid UTF-8
 */
export async function fetchIpfsContent(
  hashHex: string,
  gatewayUrl: string = DEFAULT_IPFS_GATEWAY,
): Promise<string> {
  let hashBytes: Uint8Array;
  try {
    hashBytes = hexToU8a(hashHex.startsWith('0x') ? hashHex : `0x${hashHex}`);
  } catch (e) {
    throw new IpfsContentError(
      `fetchIpfsContent: invalid hash hex: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
  if (hashBytes.length !== 32) {
    throw new IpfsContentError(
      `fetchIpfsContent: expected a 32-byte hash, got ${hashBytes.length} bytes`,
    );
  }

  const cid = hashToCid(hashBytes);
  const url = `${gatewayUrl}/${cid}`;

  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), IPFS_FETCH_TIMEOUT_MS);

  let response: Response;
  try {
    response = await fetch(url, { signal: controller.signal });
  } catch (e: any) {
    if (e?.name === 'AbortError') {
      throw new IpfsGatewayError(
        `fetchIpfsContent: gateway request timed out after ${IPFS_FETCH_TIMEOUT_MS}ms`,
      );
    }
    throw new IpfsGatewayError(
      `fetchIpfsContent: gateway unreachable: ${e instanceof Error ? e.message : String(e)}`,
    );
  } finally {
    clearTimeout(timeoutId);
  }

  if (!response.ok) {
    throw new IpfsGatewayError(`fetchIpfsContent: gateway returned ${response.status}`);
  }

  // A malicious gateway can lie about (or omit) Content-Length, so this is only a
  // fast-path rejection, not the enforcement mechanism — the real cap is applied while
  // reading the body below. Mirrors desktop's own two-layer check.
  const declaredLength = response.headers?.get?.('content-length');
  if (declaredLength && Number(declaredLength) > MAX_IPFS_CONTENT_BYTES) {
    throw new IpfsSizeCapExceededError(
      `fetchIpfsContent: gateway response too large: ${declaredLength} bytes exceeds ${MAX_IPFS_CONTENT_BYTES}-byte cap`,
    );
  }

  const body = await readCappedBody(response);

  const digest = sha256(body);
  if (!u8aEq(digest, hashBytes)) {
    throw new IpfsHashMismatchError(
      'fetchIpfsContent: IPFS content integrity check failed — fetched content does not match on-chain hash',
    );
  }

  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(body);
  } catch (e) {
    throw new IpfsContentError(
      `fetchIpfsContent: IPFS content is not valid UTF-8: ${e instanceof Error ? e.message : String(e)}`,
    );
  }
}

/**
 * Reads `response`'s body into a single `Uint8Array`, enforcing
 * {@link MAX_IPFS_CONTENT_BYTES} as it goes wherever the runtime exposes a streaming
 * `ReadableStream` body (true of Node's `fetch`, used by this module's own tests, and of
 * any future RN runtime that gains streaming body support) — matching desktop's
 * per-chunk enforcement rather than buffering an unbounded response first.
 *
 * React Native's `fetch` (a `whatwg-fetch`-style polyfill over `XMLHttpRequest`, as of RN
 * 0.74) does not expose a streaming `response.body`: the underlying XHR already buffers
 * the full response before `arrayBuffer()` resolves, so there is no way to bound memory
 * mid-transfer on that platform. The fallback below still enforces the real cap on the
 * result before it is trusted or returned — it just can't do so incrementally the way the
 * streaming branch (and desktop's Rust code) does.
 */
async function readCappedBody(response: Response): Promise<Uint8Array> {
  const body = response.body as unknown as { getReader?: () => any } | null;
  if (body && typeof body.getReader === 'function') {
    const reader = body.getReader();
    const chunks: Uint8Array[] = [];
    let total = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      const chunk: Uint8Array = value;
      total += chunk.length;
      if (total > MAX_IPFS_CONTENT_BYTES) {
        await reader.cancel?.().catch(() => undefined);
        throw new IpfsSizeCapExceededError(
          `fetchIpfsContent: gateway response exceeds ${MAX_IPFS_CONTENT_BYTES}-byte cap`,
        );
      }
      chunks.push(chunk);
    }
    const out = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      out.set(chunk, offset);
      offset += chunk.length;
    }
    return out;
  }

  const buf = await response.arrayBuffer();
  const bytes = new Uint8Array(buf);
  if (bytes.length > MAX_IPFS_CONTENT_BYTES) {
    throw new IpfsSizeCapExceededError(
      `fetchIpfsContent: gateway response too large: ${bytes.length} bytes exceeds ${MAX_IPFS_CONTENT_BYTES}-byte cap`,
    );
  }
  return bytes;
}
