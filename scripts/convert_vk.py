#!/usr/bin/env python3
"""
Convert a Rarimo / snarkjs JSON verification key to ark-serialize compressed binary.

Usage:
    python3 scripts/convert_vk.py <input_vk.json> <output.bin>

Download the VK files from:
    https://github.com/rarimo/passport-zk-circuits/blob/main/sha256_verification_key.json
    https://github.com/rarimo/passport-zk-circuits/blob/main/sha1_verification_key.json

Then generate the runtime assets:
    python3 scripts/convert_vk.py sha256_verification_key.json runtime/assets/vk_sha256.bin
    python3 scripts/convert_vk.py sha1_verification_key.json  runtime/assets/vk_sha1.bin

Output format (matches VerifyingKey<Bn254>::deserialize_compressed in ark-groth16 0.4):
    alpha_g1     32 bytes  G1 compressed
    beta_g2      64 bytes  G2 compressed
    gamma_g2     64 bytes  G2 compressed
    delta_g2     64 bytes  G2 compressed
    IC_len        8 bytes  u64 little-endian (= nPublic + 1)
    IC[0..n]  32*n bytes  G1 compressed each

Compression format (ark-serialize LE with flags in high bits of last byte):
  G1 (32 bytes):
    bytes 0-31 = x in little-endian (254-bit Fq, top 2 bits always 0 for valid points)
    byte 31 bit 7 (0x80) = YIsNegative flag  (set when y * 2 >= P)
    byte 31 bit 6 (0x40) = Infinity flag      (not set for regular points)

  G2 (64 bytes):
    bytes  0-31 = x.c0 in little-endian
    bytes 32-63 = x.c1 in little-endian   (x = c0 + c1*u in Fq2)
    byte  63 bit 7 (0x80) = YIsNegative flag
    byte  63 bit 6 (0x40) = Infinity flag
    is_negative for Fq2: c1 != 0 ? c1*2 >= P : c0*2 >= P
"""

import json
import struct
import sys

# BN254 / alt_bn128 base-field prime
P = 21888242871839275222246405745257275088696311157297823662689037894645226208583


def _fq_is_negative(n: int) -> bool:
    """True when n is in the 'upper half' of Fq: n * 2 >= P (ark-serialize convention)."""
    return n * 2 >= P


def _compress_g1(point) -> bytes:
    """
    Compress a G1 affine point [x_str, y_str, '1'] to 32 bytes (ark-serialize format).
    snarkjs encodes projective coords with z='1' meaning the point is already affine.
    """
    x = int(point[0])
    y = int(point[1])
    assert int(point[2]) == 1, "expected affine (z=1) snarkjs point"

    b = bytearray(x.to_bytes(32, 'little'))
    b[31] &= 0x3F           # clear top 2 flag bits
    if _fq_is_negative(y):
        b[31] |= 0x80       # YIsNegative
    return bytes(b)


def _compress_g2(point) -> bytes:
    """
    Compress a G2 affine point [[x0,x1],[y0,y1],['1','0']] to 64 bytes.
    Fq2 element: x = x0 + x1*u, serialized as c0 (32 LE bytes) || c1 (32 LE bytes).
    """
    x0 = int(point[0][0])
    x1 = int(point[0][1])
    y0 = int(point[1][0])
    y1 = int(point[1][1])

    # is_negative for Fq2 (ark-ff convention): check c1 first, then c0 if c1 == 0
    y_is_neg = _fq_is_negative(y1) if y1 != 0 else _fq_is_negative(y0)

    b = bytearray(x0.to_bytes(32, 'little') + x1.to_bytes(32, 'little'))
    b[63] &= 0x3F
    if y_is_neg:
        b[63] |= 0x80
    return bytes(b)


def convert_vk(vk: dict) -> bytes:
    """Return the binary ark-serialize representation of a snarkjs VerifyingKey."""
    buf = bytearray()

    buf += _compress_g1(vk['vk_alpha_1'])
    buf += _compress_g2(vk['vk_beta_2'])
    buf += _compress_g2(vk['vk_gamma_2'])
    buf += _compress_g2(vk['vk_delta_2'])

    ic = vk['IC']
    buf += struct.pack('<Q', len(ic))   # u64 LE length prefix (ark-serialize Vec encoding)
    for pt in ic:
        buf += _compress_g1(pt)

    return bytes(buf)


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        sys.exit(1)

    in_path, out_path = sys.argv[1], sys.argv[2]
    with open(in_path) as f:
        vk = json.load(f)

    binary = convert_vk(vk)

    with open(out_path, 'wb') as f:
        f.write(binary)

    n = len(vk['IC'])
    print(f"Wrote {len(binary)} bytes to {out_path}")
    print(f"  alpha_g1 (G1):          32 bytes")
    print(f"  beta_g2  (G2):          64 bytes")
    print(f"  gamma_g2 (G2):          64 bytes")
    print(f"  delta_g2 (G2):          64 bytes")
    print(f"  IC ({n} points, G1):  {8 + 32 * n} bytes  (8 length + {32}×{n})")
    print(f"  nPublic: {n - 1}")


if __name__ == '__main__':
    main()
