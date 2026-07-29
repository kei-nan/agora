/**
 * Hand-written type surface for asn1.js's `decoded()` output — only the
 * shape sodParser.ts actually reads. asn1.js itself is untyped upstream JS
 * (see its own header comment); this is not a full ASN.1 type system, just
 * enough to keep sodParser.ts honest about which fields it depends on.
 */
export interface Asn1Node {
  /** e.g. "SEQUENCE", "OCTET_STRING", "INTEGER", "OBJECT_IDENTIFIER", "BIT_STRING", "SET", "[0]" */
  name: string;
  /** Human-readable content summary (asn1.js's own formatting — exact text varies by tag type). */
  content: string | null;
  /** Full hex dump of this node's encoded bytes (tag+length+value), lowercase, no separators. */
  dump: string;
  header: number;
  length: number;
  tagLen: number;
  sub: Asn1Node[] | null;
}

export class Hex {
  static decode(a: string | number[] | Uint8Array): Uint8Array;
}

export class Base64 {
  static unarmor(a: string): Uint8Array;
}

/** Decodes a hex or base64 DER/BER blob into a simplified ASN.1 tree. */
export function decoded(val: string): Asn1Node;
