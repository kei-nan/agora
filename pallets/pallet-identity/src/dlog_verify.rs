//! Chaum-Pedersen DLog-equality proof verification for OPRF committee mailbox responses
//! (`submit_oprf_response` in `lib.rs`).
//!
//! # Why this exists
//! `oprf-committee-dev/src/dlog.rs` already implements (and validates against real
//! known-answer vectors from `TaceoLabs/oprf-nr`) exactly the relation a committee member's
//! response needs to satisfy. This pallet cannot depend on that crate directly, though: it is
//! deliberately its own workspace, `std`-only, explicitly documented as "must never be pulled
//! into [the runtime] by accident" (see its `Cargo.toml`) — dev/test tooling, not something a
//! no_std runtime pallet links against. This module is therefore an independent no_std port of
//! the same math, validated against the *same* upstream known-answer vectors `dlog.rs` uses
//! (see `mod tests` below), so the two are provably checking the same relation even though
//! neither depends on the other.
//!
//! # What's ported here, and from where
//! - BabyJubJub curve arithmetic (`Point`): ported from `oprf-committee-dev/src/babyjubjub.rs`
//!   (itself a port of `oprf-nr`'s `babyjubjub/src/lib.nr`). Curve equation
//!   `a*x^2 + y^2 = 1 + d*x^2*y^2`, `a = 168700`, `d = 168696`, over `ark_bn254::Fr`
//!   coordinates (BabyJubJub is an embedded curve over BN254's scalar field).
//! - The raw width-16 Poseidon2 permutation (`t16`), used *only* for the Chaum-Pedersen
//!   Fiat-Shamir challenge: ported from `oprf-committee-dev/src/poseidon2_taceo.rs` (itself a
//!   port of `TaceoLabs/noir-poseidon` v0.6.1's `poseidon2::bn254::permutation::t16`). This is
//!   a **different** Poseidon2 instantiation from the sibling `poseidon2-bn254` crate this
//!   pallet already depends on (that one ports `noir-lang/poseidon` v0.3.0, used for
//!   `oprf_pk_hashes`/`committee_slot_for`/`calculate_param_commitment`) — the two are not
//!   interchangeable; see `poseidon2_taceo.rs`'s own module docs in `oprf-committee-dev` for
//!   why. `hash_committee_pubkey` below deliberately uses the *other* one
//!   (`poseidon2_bn254::hash_bytes`), matching `circuits/oprf-identity-anchor/anchor/src/main.nr`'s
//!   `oprf_pk_hashes[i] = Poseidon2::hash([pk_i.x, pk_i.y], 2)`, which imports
//!   `poseidon::poseidon2::Poseidon2` (`noir-lang/poseidon`), not the TaceoLabs one.
//! - The DLog-equality relation itself (`challenge`/`verify`): ported from
//!   `oprf-committee-dev/src/dlog.rs`. Only the verifier side is ported — `dlog.rs::generate`
//!   (the committee's *proving* side) has no reason to exist on-chain.
//!
//! # Scalar handling: no `BABYJUBJUB_Fr` reduction needed for verification
//! `dlog.rs::verify` explicitly reduces the Fiat-Shamir challenge `e` mod `BABYJUBJUB_Fr`
//! before using it as a scalar-multiplication exponent, with a comment noting this is exact
//! (not an approximation) because "group-scalar action on a subgroup-order point is inherently
//! periodic mod that subgroup's order". That reduction is a **performance** optimization (a
//! smaller exponent means fewer doublings in double-and-add) — for any point `p` in the
//! prime-order subgroup and any non-negative integer `k`, `p^k == p^(k mod ord(p))` by
//! definition of group order, with or without reducing `k` first. This module's
//! `Point::scalar_mul` therefore takes the raw big-endian byte encoding of `e`/`s` directly (up
//! to the full 32-byte/~254-bit BN254 field width, no explicit `BABYJUBJUB_Fr` reduction step)
//! — mathematically identical result to `dlog.rs`, and it avoids needing an arbitrary-precision
//! integer type in this no_std pallet just for one reduction. `check_sub_group` (needed to
//! reject small-subgroup/cofactor points, per `dlog.rs`'s own `check_sub_group` calls on `b`
//! and `c`) still needs `BABYJUBJUB_Fr` itself as a fixed 32-byte constant
//! (`BABYJUBJUB_FR_ORDER_BE`) to scalar-multiply by, but that is a single `scalar_mul` call
//! with a hardcoded exponent, not a modular-reduction routine.

use ark_bn254::Fr;
use ark_ff::{BigInteger, Field, PrimeField};

// ── BN254 scalar field canonical-encoding check ─────────────────────────────────────────

/// BN254 scalar field modulus `r`, big-endian. Duplicated from `runtime::verifier` /
/// `runtime::anchor_verifier` rather than imported — those two already duplicate it between
/// each other for the same reason (a three-line check not worth a shared crate just for this).
const BN254_FR_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

fn is_canonical_fr(value: &[u8; 32]) -> bool {
    *value < BN254_FR_MODULUS_BE
}

/// Decodes a big-endian 32-byte array to `Fr`, rejecting non-canonical (`>=` modulus)
/// encodings so there is exactly one valid byte encoding per field element (no
/// reduction-based ambiguity at the extrinsic boundary).
fn decode_fr(bytes: &[u8; 32]) -> Option<Fr> {
    if !is_canonical_fr(bytes) {
        return None;
    }
    Some(Fr::from_be_bytes_mod_order(bytes))
}

fn fr_to_be_bytes(x: &Fr) -> [u8; 32] {
    let mut out = [0u8; 32];
    let be = x.into_bigint().to_bytes_be();
    out.copy_from_slice(&be);
    out
}

/// Parses a round-constant hex literal into `Fr`. Mirrors `pallets/poseidon2-bn254`'s own
/// `fe` helper (same crate, same `hex`-with-`alloc` no_std pattern) — see that crate's module
/// docs for why this is a real port (not hand-derived constants) and safe under `no_std`.
fn fe(hex_str: &str) -> Fr {
    let bytes = hex::decode(hex_str).expect("round-constant hex literals are valid hex");
    Fr::from_be_bytes_mod_order(&bytes)
}

// ── BabyJubJub (`oprf-nr`'s `babyjubjub/src/lib.nr`) ────────────────────────────────────

fn curve_a() -> Fr {
    Fr::from(168700u64)
}
fn curve_d() -> Fr {
    Fr::from(168696u64)
}

/// `BABYJUBJUB_Fr` from `oprf-nr`'s `babyjubjub/src/lib.nr` — BabyJubJub's own prime-order
/// subgroup order (decimal
/// `2736030358979909402780800718157159386076813972158567259200215660948447373041`), **not**
/// BN254's scalar field modulus above. Used only by `Point::check_sub_group`.
const BABYJUBJUB_FR_ORDER_BE: [u8; 32] = [
    0x06, 0x0c, 0x89, 0xce, 0x5c, 0x26, 0x34, 0x05, 0x37, 0x0a, 0x08, 0xb6, 0xd0, 0x30, 0x2b, 0x0b,
    0xab, 0x3e, 0xed, 0xb8, 0x39, 0x20, 0xee, 0x0a, 0x67, 0x72, 0x97, 0xdc, 0x39, 0x21, 0x26, 0xf1,
];

/// Domain separator for the Chaum-Pedersen proof (`DS_DLOG` in
/// `circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`, decimal
/// `1523098184080632582082867317389990410064981862`) — the field-encoded ASCII string
/// `"DLOG Equality Proof"`, kept byte-identical to ZKPassport's `DS_DLOG` per that file's docs.
fn ds_dlog() -> Fr {
    fe("00000000000000000000000000444c4f4720457175616c6974792050726f6f66")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Point {
    x: Fr,
    y: Fr,
}

impl Point {
    fn identity() -> Self {
        Point { x: Fr::from(0u64), y: Fr::from(1u64) }
    }

    /// `BABYJUBJUB_GENERATOR_X`/`_Y` from `oprf-nr`'s `babyjubjub/src/lib.nr`.
    fn generator() -> Self {
        Point {
            x: fe("0bb77a6ad63e739b4eacb2e09d6277c12ab8d8010534e0b62893f3f6bb957051"),
            y: fe("25797203f7a0b24925572e1cd16bf9edfce0051fb9e133774b3c257a872d7d8b"),
        }
    }

    fn is_identity(&self) -> bool {
        self.x == Fr::from(0u64) && self.y == Fr::from(1u64)
    }

    fn is_on_curve(&self) -> bool {
        let lhs = curve_a() * self.x * self.x + self.y * self.y;
        let rhs = Fr::from(1u64) + curve_d() * self.x * self.x * self.y * self.y;
        lhs == rhs
    }

    fn negate(&self) -> Self {
        Point { x: -self.x, y: self.y }
    }

    /// Twisted Edwards point addition (complete formula for this curve's parameters).
    fn add(&self, other: &Point) -> Point {
        let beta = self.x * other.y;
        let gamma = self.y * other.x;
        let delta = (-curve_a() * self.x + self.y) * (other.x + other.y);
        let tau = beta * gamma;

        let den_x = Fr::from(1u64) + curve_d() * tau;
        let den_y = Fr::from(1u64) - curve_d() * tau;

        // Only non-invertible for malformed/off-curve inputs; callers always check
        // `is_on_curve` before relying on `add`'s output, but fail closed to the identity
        // rather than panic if that invariant is ever violated.
        let (Some(den_x_inv), Some(den_y_inv)) = (den_x.inverse(), den_y.inverse()) else {
            return Point::identity();
        };

        let x = (beta + gamma) * den_x_inv;
        let y = (delta + curve_a() * beta - gamma) * den_y_inv;
        Point { x, y }
    }

    fn double(&self) -> Point {
        self.add(self)
    }

    fn subtract(&self, other: &Point) -> Point {
        self.add(&other.negate())
    }

    /// Variable-base scalar multiplication, MSB-first double-and-add, over the raw big-endian
    /// byte encoding of the scalar (see this module's docs for why no modular reduction is
    /// needed for verification).
    fn scalar_mul(&self, scalar_be: &[u8; 32]) -> Point {
        let mut result = Point::identity();
        for byte in scalar_be.iter() {
            for i in (0..8).rev() {
                result = result.double();
                if (byte >> i) & 1 == 1 {
                    result = result.add(self);
                }
            }
        }
        result
    }

    /// Whether `self` lies in the prime-order (`BABYJUBJUB_Fr`) subgroup, rather than merely
    /// on the curve (which includes small-order/cofactor points too).
    fn check_sub_group(&self) -> bool {
        self.scalar_mul(&BABYJUBJUB_FR_ORDER_BE).is_identity()
    }
}

fn decode_point(bytes: &[u8; 64]) -> Option<Point> {
    let x_bytes: [u8; 32] = bytes[0..32].try_into().ok()?;
    let y_bytes: [u8; 32] = bytes[32..64].try_into().ok()?;
    let x = decode_fr(&x_bytes)?;
    let y = decode_fr(&y_bytes)?;
    Some(Point { x, y })
}

// ── Raw width-16 Poseidon2 permutation (`TaceoLabs/noir-poseidon` v0.6.1's `bn254::t16`) ──
// Used only inside `challenge` below. Round-constant tables are transcribed verbatim from
// `oprf-committee-dev/src/poseidon2_taceo.rs`, which itself transcribed them verbatim from
// upstream — see that file's module docs for provenance. `mod tests` below checks `t16`
// against the same upstream known-answer vectors that file uses.

fn double_fr(x: Fr) -> Fr {
    x + x
}

fn sbox_e(x: Fr) -> Fr {
    let mut result = x * x;
    result *= result;
    result * x
}

fn sbox16(mut state: [Fr; 16]) -> [Fr; 16] {
    for s in state.iter_mut() {
        *s = sbox_e(*s);
    }
    state
}

fn vec_add16(lhs: [Fr; 16], rhs: [Fr; 16]) -> [Fr; 16] {
    let mut out = [Fr::from(0u64); 16];
    for i in 0..16 {
        out[i] = lhs[i] + rhs[i];
    }
    out
}

fn mds_4(state: [Fr; 4]) -> [Fr; 4] {
    let t0 = state[0] + state[1];
    let t1 = state[2] + state[3];
    let t2 = double_fr(state[1]) + t1;
    let t3 = double_fr(state[3]) + t0;
    let t4 = double_fr(double_fr(t1)) + t3;
    let t5 = double_fr(double_fr(t0)) + t2;
    [t3 + t5, t5, t2 + t4, t4]
}

fn external_16(mut state: [Fr; 16]) -> [Fr; 16] {
    let mut mds_parts = [[Fr::from(0u64); 4]; 4];
    for i in 0..4 {
        let offset = 4 * i;
        mds_parts[i] =
            mds_4([state[offset], state[offset + 1], state[offset + 2], state[offset + 3]]);
    }
    for i in 0..4 {
        for j in 0..4 {
            state[i * 4 + j] = mds_parts[i][j];
        }
    }
    let mut stored = [Fr::from(0u64); 4];
    for l in 0..4 {
        stored[l] = mds_parts[0][l];
        for j in 1..4 {
            stored[l] += mds_parts[j][l];
        }
    }
    for i in 0..4 {
        for j in 0..4 {
            state[i * 4 + j] += stored[j];
        }
    }
    state
}

fn internal_16(mut state: [Fr; 16], diag: [Fr; 16]) -> [Fr; 16] {
    let mut sum = Fr::from(0u64);
    for s in state.iter() {
        sum += s;
    }
    for i in 0..16 {
        state[i] = state[i] * diag[i] + sum;
    }
    state
}

fn permute16<const R: usize>(
    mut state: [Fr; 16],
    first_full_rc: [[Fr; 16]; 4],
    partial_rc: [Fr; R],
    second_full_rc: [[Fr; 16]; 4],
) -> [Fr; 16] {
    let diag = consts::internal_diag16();
    state = external_16(state);
    for r in 0..4 {
        state = vec_add16(state, first_full_rc[r]);
        state = sbox16(state);
        state = external_16(state);
    }
    for r in 0..R {
        state[0] += partial_rc[r];
        state[0] = sbox_e(state[0]);
        state = internal_16(state, diag);
    }
    for r in 0..4 {
        state = vec_add16(state, second_full_rc[r]);
        state = sbox16(state);
        state = external_16(state);
    }
    state
}

fn t16(input: [Fr; 16]) -> [Fr; 16] {
    permute16::<57>(
        input,
        consts::x5_16_first_full_rc(),
        consts::x5_16_partial_rc(),
        consts::x5_16_second_full_rc(),
    )
}

mod consts {
    use super::fe;
    use ark_bn254::Fr;

    // AUTO-EXTRACTED from TaceoLabs/noir-poseidon v0.6.1 poseidon2/src/bn254/consts.nr, via
    // `oprf-committee-dev/src/poseidon2_taceo.rs` (constant transcription only, not retyped or
    // derived by hand — see that file's module docs).

    pub(crate) fn x5_16_first_full_rc() -> [[Fr; 16]; 4] {
        [
            [fe("11a8c50ae2baf9f5e8b3c672c7326f002cdec557448fdfa263f0adfbaaceacff"), fe("062af6c4373daac18754f00ee840e8920e3f715aa7d14233948e54596ac34e43"), fe("067668742769c8dc002f62ad79b936464ee18b541f9d560a1df662a1de735099"), fe("092566abceb5edc4fdd60f23c5c605a7b85a6282a1cf7920eaf4f34587722904"), fe("1dce678af089b569d6b90eb80d3d937f98f952e8f4928a6f6d41c4ddeea8ac8a"), fe("0a0f0bd983e55175db6027fc884726d4c79e1bf6e6b65eb7a6051d02caa2adc3"), fe("2ab887d5bef4f2bb90472c6da5531555ab3e5b692c21b57855d4cd77c08b73a1"), fe("0e29f9015a7443d2be603384abb29dd7bf930d1a28686cfdbc347e9163870b73"), fe("21b3a5f0e068934a7436d1d28e8cf0bc61d712e158a8bb2c42a57125d8727ad2"), fe("0e1f8ea955cc9e2efd16f26fe45422f1749a9fd2fc10ff4ed3431ba73ea1c126"), fe("2130513365c023ad8997f530b8f5077c7bc5408dc8177b0379d25eb46266d06a"), fe("1996a13e24a307419ef17eaa2120d6a3510e7d0daff70d3b73e1c3da47d9e81a"), fe("2ae828a9edb591d66810146f3e0efe95336e919fb8924f65807d2e20b70ffe32"), fe("2f617fc3ddba10cfc459bef0adcb6dcf024b50924f5a9719707ea9cfaceeba86"), fe("26034af9d21fea59b35e1edd6bc6ad24e39bb2309014f2df434b01d52360ad6a"), fe("0861d9a9b50eda74e99bc741bab3c8d7c36e4dc3828ef88a8822ad1d97cb2dc1")],
            [fe("09d307c6251380f12959318cb1b6b6bc8b5f0e87c8aaa5b9a9c02a747f82230a"), fe("2d7249c015893681c5b6406a5f3ed9913181fa18cf0c9cc5404d002612e5f6ee"), fe("300c394ab0169e3579b604d3e4c8344ffa05164a7fc66d1d0fec5e1b0aba7c3c"), fe("053246a30507749b78daca7f5e1bb40d8dd095a228d4754f587a522ae3bebc5f"), fe("2bb8c89e9959b6bc4af0814110a58e9f2de278cb4ce3a3e85b5df69f20bc6c85"), fe("03d087490008dce2b785898ea7d63aa073e5b01645c59c49ce136275cf5983b5"), fe("1b1860486b80b33467e250e5fee700e42ea8b1010ae9720e0dba074d0a711483"), fe("07c2df19e33502e0d6407204240d9ae9d53671a8acb8275c87a878c8d31ae6a9"), fe("2681360e637f307e9d3dd46d8d9ac32d1eaa80fb008a164c495cdcba8f80575b"), fe("26e40b31dea8fd07591fe4cd5046b8995ad2b38cfff12bb5d9556049062f8c60"), fe("142db0b3205d81b91e289e8640447ce61d0a5624e7a80276b2aafc6c020bd45b"), fe("11481ae8ce1f4453b29a23cdf06bb4da1344fa0858cc366eab6e2871f6820bbf"), fe("14afa414edfcf985dc050516d4e6e6f61b32fdf2ec6faee7eae103be602c2d9d"), fe("07fcf3ec2df0db6971c71924e8adb1f96f8d53fe7de3676a7704f621ac470f5b"), fe("21daa5e36df00136f54419d5e4c09ec09bba58903173c55822e51d65b0779e6f"), fe("2e324f627abadd10f08206430befea7ae4b5ed5e965ec1380af6f883eeac7d3f")],
            [fe("0b5bb384cfccbd7b191e651901f6193f949a3c8a226cec1fde2a43d8829dbe86"), fe("069ddb007faabaf73350dc4eadb907bd2d0d4a214f6654b30756e8ad18df5f18"), fe("208b465f89b447783ea3fb14cbb235891ae8c4e5a5031365f470bab6cf86dd3c"), fe("136297f9e831fb9640b024db98afd0a4eba714daa31944c4e7613dd78fc7e14c"), fe("10a7859a7db68fae99f59a4a784cf518f4e7b9cf94ad7aadfc8b089f3b3ed9e8"), fe("0862b4da27d415d0ee6ee421b3dd98ecb3188489103fc6028684d166a5212ebc"), fe("233af2967f6f7740ff8feb103ff33416f7d2a2f08aeb48e6a84347789fa4404c"), fe("2ae069b5948ab8f0ab759617732fabf28f734dfd5732f7f22d0797078908d592"), fe("00583f3b42880263ba0069d278a8d6c633913f930510ea5e8bc4fdb6e0d654b8"), fe("12e2f29632a6e47b536e42912519b2f5f6086fd92ba211af7776be12b342730a"), fe("06dbb6c7b8448c820ae128612b7fc4698a74cfa8bb058a90742d4188b3063bf1"), fe("18509380d353243decd53f8b29e3e830dce894b5ed590eeeab18b1b92f3a6da7"), fe("0c6fc99e5d3a8ba836d2a51632a7efa4a2732c5be32ff1bb8988e97357cfe561"), fe("042196cef447997203de5aa95cd5a8158ddfb11a8ae83c733cb3ce942f1e3952"), fe("2d741b558a9a39fc48442b13f7a09394d878d41b7963105175b5eacc0762d341"), fe("28561383e49fc55b465f9a55c56872a9da39cc1a8601c984fc142f519e01107a")],
            [fe("283c11ef394149faf2dabef893843ad8afc42d8322002648b15c7a6b2f8e36da"), fe("16e5502b7577231018c8b8a59b3d9d60952d4e169fa38a080ce1860cd771ea5d"), fe("1dc343ebb1999bdd849b519b6c88c25ed40fa673b44a37ae895f1ee2efa97458"), fe("192b8afb63d8d9357b5d74136d341c2811f611336acfeb20a0e4a83fc726cb1e"), fe("1a30b60940afd0871265b329ec9ae86a61df686cbb09b6b57adcd2eefbe243b8"), fe("20a00f04fb239f151d607d4a38fda2ef13fcfdf2cb7290c86b19bdedae4048fc"), fe("0293b42083e8d2ae737e112e012df4927fa61a56753e4cd9ada1690e5ed529da"), fe("2db103a89e5cb3b42ac01913fe5c8de3cfe959ea6033bfc3a17451906560f807"), fe("1cb81d2dfa938c9397a2da95a68184983ce0fff321071470250b781cdd80c73c"), fe("03f1355c9f18f37d7837813b19946787301f68b7b3720f02cd358bc50c9ad216"), fe("13af73a19617a92625a1c30743bc9775a4e8b0b9f3cef088f39bbccf12caa3f5"), fe("114938f09f618d6176cff87ec5d1532a265ac4a87e21d68ee61dae8d9a554924"), fe("0acb455d2f8dd661528e732ea07ef93c9e03630f51903b86bc6835afcc03a98a"), fe("151adb5dd2d8fa77437afe73aed319d84dd546839f9d7b687e825d9b23dc1744"), fe("25548523d039ac346af19b64d52549e24dea02ea0d5476a0395c4c438f613464"), fe("09cc75c22a37ffa264db2540ae85d1ef6965111461eabc9fc652a879b4bdd140")],
        ]
    }

    pub(crate) fn x5_16_partial_rc() -> [Fr; 57] {
        [fe("0f1139934f4fb0ff2f4bd0b7d28544a1c8938de92dcf965e1a1781c86aa8e6c2"), fe("2b6f9a7fe52ad45ae8857683ea066cdba3f7c3dc7da78411c83583070ac12d47"), fe("18fcc896be2e9edcfd06d0ef23c523956148f3abcffcb0802e57c65dfa7eb6d8"), fe("13ae7493c450aec181999c3fb9c0b4d4a5a595672b2cab4181a5b52ec2fccf68"), fe("21376c58b83138f960981bcf86f680c79081c95f6914477fb5d6cfc7497c1526"), fe("142335a55e77462aaac6aebd3f92ccb12e15204546af29eed4f5e73600a4bf8c"), fe("0c5ddae16b04dc051c9f419c25b7f6408a3d653ebda349ce9332593b371fdf33"), fe("2689b4678392a84600cf0b6fb25c9b35c065b6958aaa056ae8b099fa61e87b75"), fe("2c57afc39ed2d8ec9b9cf1c685f7b73c0ac1d0540460ef91ee30e56b84607be5"), fe("1b78b860308b6b50845c744c96a2237fc6ea46a05d4884dbe8a5e797432c269a"), fe("13cc94f496327f946bd5bf504ab86714bc345f836fcb9b1fe307c40d62c3be3b"), fe("24d6de093538a86baacd50c2ba65c212bbfd46c88acf6adc911e678bfc7c1014"), fe("2258067f017ee12ae23944f9b71c0e8f39d165a22995c8a12a76bb340c853aeb"), fe("14b3d102f33c05e6ea23d0159d2bf879ca8bca2ee547e25bde0d3289295542fc"), fe("2ab89c7fffc98dc5ba3374ee0ec8c58607e45015b08cb6d13d282d13fdcd9cbb"), fe("2b7b6c147957fe1e074237cb272f8b38fadf548ab37c7cca248c2b6e7f6ef184"), fe("22e4d3e1b886adcd0f5b0cf73c1a9bf1d3db30305651dc858b75c2c22c1b4b0d"), fe("0b68fa9b0d5df7dce1621bebf659a18285204c82effc3a17a52c8f869a04386f"), fe("1634373a4427e6ed6822633c676f76244b7323df66094f7dac02febbabf93ce6"), fe("161816eae4a5a59647769db36bd32d91e36a4d2881180921c6ae1c774da8a88c"), fe("0e8dc7cd6f5219b8f0a202d5d9142eb80144fb4945777175cc21a5de7f93734d"), fe("125c7c9c18bfc84d8298305590ffbb6a6ec51389eb2678522ba53dc7b6a9c989"), fe("0fa705e5fb58d754efb803b9c24ded86fdf44f0b9027858cb7468889b9b793f9"), fe("2db7b16fa0d8cafdb13856e67e232531b6b64b181717b56e918d895d1f6de779"), fe("20ee21a8e99de49e0f23484346d51ca84c763fd3bc42ed49302ce517efb7c3d1"), fe("2464170cd57c89626cd9f1c25ea604103cf5c7c6b6384d63cccc2539fd227b10"), fe("18d92ee6e4ad5ed8671a3a902e471b7b17423a0a123a069f5ce724d219900e6e"), fe("06e7eb25aa2f77c9f64e6e997569593b1b1aada9d97b4fa8d5aa58eaca28ee3d"), fe("245f3670a6e3be9104fc999f977e05dab46a0434e682272afdfe33b431489422"), fe("2338e44c82e527c3f9b4064e9691a01b01426c264a5eab102627ca1798a96731"), fe("025ba964c43c4dd03b90be052c367b5988b0f613d67afe23caecb7535b02d8c2"), fe("227a926359ecb99dbf2fbb5bcfeb66b443b73074de47f75d5285a63d5e9bb8d0"), fe("080e987035c7ab8091afdab9737d67f7b29d5a055cfe3e80da76c5ed40036fcd"), fe("287d9727fa9787cb13d2c6c4f861fe6948ce2085e817b17f2d01c7ac5d9a71e2"), fe("0cff9c7671547d0403a7c6dd36a818db81d84f73c6618b28b9adc117de6ea286"), fe("231cbd6dbd1339319bdb4549090f7f907c46f520aaea6c3e669b1b933bc978df"), fe("003fe17bbf0d32f5eef248969461584ff7718c1a84e79daf88c813868f09a10b"), fe("13768255aa238077f21510e708027b2a9379b9bbbd34e832220ef0c741d7bc76"), fe("19f3f23131739230955fa717eda73f9da0bf596b1dff0c23ed24cb2f95044160"), fe("2b24106b3883d29d76325e0ea82d0e34c742b992fe78f91e09c7e308e8d37ddc"), fe("000fe2d40c9e86c9d35e148a06ea4a3410f29e7519e394259e0f6afbfb6519fc"), fe("27ae2a5837cf9c79ccabe4f7f61252ec8e6bbb272005297cca83cc40b4110e89"), fe("15684fefda12b32ba4873564c3502a67a2076134f71ff66f4e2ee5315b02a19c"), fe("21045f6063ec9e2e6e7040c0949a46235693805af8e75a29760dd49455e61dda"), fe("00ff37a0311730e3ce35c3ae5467f47401add02962daa4cebcf2cae53085eb10"), fe("12a8f68dad75db509547b97c08699ef05dc6e7a871553dac62f3d9bee87e14dd"), fe("0444811de68c064af36942e4ae9059284f0dfe86650c72cacfba8c8a06e920ff"), fe("21c1763df1a7206705b65d0942acec0c4cf61e60782ffae540b6c41dd6a43d50"), fe("2575340455a05474d748608031d21ea1502e27c6fd348f9992cf8881c2d81e69"), fe("08f501b2028eb549372061ad3423e3f0f71bc45b89870be397c5c5e01b19760e"), fe("05b62275eb9fa36c94136efb0b3767bde46ff1d35b811759aa2b54c90dabbb54"), fe("0519ec5dc1a9b538923f0c3354603bd0c7d7acd9b9ce817b70bb98acc4744d58"), fe("1549a0a9856793b1fb73049189f34b2e0ed9e08c48875359aa899dcee91e1e82"), fe("06191e3ff7936bd97b948b095a6bb2e458b33a4f2a731b5a06c347f1f6b46d6d"), fe("2a3d690c70b4930341afbbea186b0394a3a7dd7d911f8106db6c96476395f25b"), fe("04d39a7a0b5f372041d717d422dc3b862a8a8b6ed1a174efb24b923c9505c5b0"), fe("083b47892d403e287bbf80d319866f50adc5a5b50f81f1708986078e64065b26")]
    }

    pub(crate) fn x5_16_second_full_rc() -> [[Fr; 16]; 4] {
        [
            [fe("1c4b724162943f08ad657557a638bf3e7bec8e57d5db00b9767682c8213132d8"), fe("2035614227e301894056bf197033ea74e568e22a4644da4234fad6010f9195f1"), fe("2867e28d25a0560e620af7e734e3155528406285834db300d3f88a5c63158545"), fe("2d9aba98839b67ae817c673f35059c574d229529650b0b718e0a89de46bc9b77"), fe("2620dcb35abd4a2c1c34219eed205f8cd095a96c00cb66dbe106127615562823"), fe("189693ff51f37b66cdb414147465ba7aebbed3006853a8b6695b46d088da9f9d"), fe("0dcf0bb67aa309914b0aee36c52661723c0b3af0abef68d43896bab66a111d80"), fe("207abac676cab81bdee421468577de3d55df734bcfc5e7b0966c7c05f6676ed5"), fe("072751669cc40c66a1e56cacd6a57ebef404b8089d8bcb827bed335b3a81f68b"), fe("2dd0616e2fab6505dc6a0ef4470139d33dace70f7102a179c6c5f45e197a3b31"), fe("2b6d0ca50c9b10229774a059bee1fc4d090c879bdc49fd79a6e97be85de5421c"), fe("1315947727a368a3713ac6b5968cb50ae30ed2f26546062eb3be935284b2d7ac"), fe("2c50018a609e805dd45428015180eef905c885d10f4bdc57bf822bcdbb7af7bc"), fe("1a51c15bbedba963fb108a0b5470851f0e9896f6ebb1b3b6e51ed5c6c5e26ed4"), fe("221a70968ceadb2b1a3949215db03d70457982d2e5bdce6edd224837ed952757"), fe("11536753adfe665c99ccb6a38cbd9d06c3c7b7122b51dc063628a669dded0b36")],
            [fe("031897f30796e50a12b212bd146ed58fb50b443ae9a2dbfdb4f6b8fe5c61479e"), fe("1f56af932c2012a38d77352c6ddb68fa13c1c9c14ba96fe7e6d170ffed0d759a"), fe("1ecd95dae8cb508bcf72e6584898d2108854749c3cc686e4b02f8fe57aed95df"), fe("02fabaa828c27c721f68c33d9d2b881210704a41d9c28f0a4af02e67fd345caf"), fe("1da2fc3d073c37c2921af16e44330ee72caec36643d75e766c7437c59b12d6e4"), fe("1ee20f40ea03cb4d5620d4422f732f547c198e40fef3839370868c44a0eb5d17"), fe("1c79db7d2a94ac6cc15285a8e5de39e4e93b574c4d7274e08b5f27e98c96a9cc"), fe("039323133de3519bd46223b43f8905772323e7111b398cab542b4fbdbd4b4d8d"), fe("02e21c346778fed85751f95cca3da1e67f96fb5d4c4a400ec81e324f99ce88e3"), fe("174199bd6b9babd9961f9076e8e09c47808ea337a2e83b4b80d7685e1f01912b"), fe("29a0cc2bd15ed76a356d76fd2c33cc8df84e81068ac799e8a9eca0a8e2836d14"), fe("05136a95e080ef56c23c099a7f8569667584655eb8d8cef8dc46ad90943f484a"), fe("2820e3357abf5aeedf9a457603a627e9bdd807df2734bd6e81051b009f13093c"), fe("205e6b631bad8731629ac0af44645716f0f9dd549d7be0aa4ecaa82fda20276c"), fe("1eb9ab4dd7426e30f809c9929f84ea59d6136daa5f7022847ac8e01f04ee91e2"), fe("2d89f59253f5b9a24e0188c1ffe0fe6d1585c30c019bba8b4c13fb65441d8b0c")],
            [fe("2bba2d4afc396925d03d39ea49fd7d23174a3ee023a3625b7d937936b78496f3"), fe("0e59a58b602d4ca9e2ca2a10796a2356188a39eafa39e5525f53f560d3349cbd"), fe("1cc183780561724fe8db9d8f8c456c32c8c29556c12c0157ef8fbc9f00e789a7"), fe("054120821f04fd778dde8968f55ba1f8621a17aac4c0dae7fd3e19b5ffe7ed32"), fe("1f02893a858c61d860abd638710bc13aa6ae1ee2f3227513c92378c9d74d04a9"), fe("0371fd918bcd2d93ca93864f1dfb8c851c2074ed5b06fd1339ba1eedd20a4078"), fe("1f3fb61afc1ccc181b5f33139f59cdbee76778726b9a73fbc196e745afd3f10a"), fe("1f6ab61feaa716f4311adca796e444e9636abebb501dd7224692fdfa0d64c9fd"), fe("0bbc08a17c0f31e73a0049d3464897167566880bbc884eab974c63b37e663589"), fe("2f6e6c68f97cffb2035d973014678081f8a8f0fede19b7d5702d6b5e368f265c"), fe("10eae0663059eba3e2b842c9076076b5241b4977dec5912f145d891fe58e9b93"), fe("0f234bec6cc14051fbb4ebaadc99aaca497c1c2d5fef51e5edb6e8174fc737b5"), fe("11d7a67da5230703f013ee574653d7e02dcb7fdf07f39f21410fda7ffcb09f9a"), fe("0fee562c5444f1e094b4a6bd732b720771f9ba98db7dcf8231b8d17646843432"), fe("0a076467e3a9603a7763d3202616302c719e0cc05870d0a8458842e30ce047d3"), fe("1c733d34815220317facf6f54ea32006e2d3986aae8c72add1b2e1196234ddf9")],
            [fe("186d4f686994b1e7790b2ecba770c373ee1d70fd63b71073e65ecad88dd5a3a9"), fe("12d1d277d3ccb4997d6cd545981b73bb48fab183bf9de1db16e0d1cbcf44c313"), fe("16169740abc733a5753234257d3189d594d1ee5b2ba7ea9445707842d6805bdd"), fe("2566d99b2fc31cb583aa146e9dd39ccc09436a45ade8a57c9f4c2f4daf9632b1"), fe("146f2ba6b24e1f31962cbadae6b36fc4a896cc6eb99d63d9859a2a4f91aadcda"), fe("2ad5aee9f6379bb5e275b6c2ac947fb0d54216c85a6c786b2b5ba133a1ce17ea"), fe("2fcdf08ef110e18c877c4e7e28c4f902bface389a0b955af669065d0766d83a7"), fe("2d9f57de99fac20c55f402f06d9c9d3a8f49d62fb106e4dd8cca84cbd685c06a"), fe("1c039fc13e4161998ed60fc909e194ebceb1e7a4ee755d7201ff96ecf3632e03"), fe("17f2ad6ecaaf3e5db04b6aea8d17bcb58cfe4a1024ecdba3ef5d57dbfd020202"), fe("196ec1e27eab458b378961284b6a98a6766cd6b379b822868715e826e0a21406"), fe("109afa2d34c4fbac99becfa70f20b2087f7158413a02604b386684f3f848a896"), fe("148bfccbaaf2f7e951ef266b8c11a8c19bfe013749c38f5bcfe608d3282fb37c"), fe("0a25d11a8d1ed5c87e03b684b7b6d34140063cb4f77e312ec5ac12da98911e40"), fe("2479d850bb1a9b9143f126da89063a8a3e5d0a9d5917e2bfa5927d8eeb70c526"), fe("115107e62902f6facbd8a1caa2a2f67a15301e5c034cb99c764f6402ca331781")],
        ]
    }

    pub(crate) fn internal_diag16() -> [Fr; 16] {
        [fe("269aaf7c0e0ae1a709c1b7cd137c366a3ef21c0ca7d9fb2b33b5a1ae235768e4"), fe("30543ee04032614e317229edfaf3b27da10dd0792f35ecb2fb82a20c30eb1de3"), fe("017416b13160b7d8d73ffd44efc75ce642f1d002e332ad4bd68469b8b83c5fc4"), fe("09b103f438a43f1aabb6bc5d3490d3c443d773b966d902d36c81490614939eaf"), fe("08f9e81ea21aa882da55bde42c830d261462c4489451ab181513614983fcdb30"), fe("026d2cf77cf485777fb797f7c3bf17acafcb3679549ac98acb6e430eb53e4be5"), fe("0652442bfa09590b710b3273f0d3c3de61defe08359aa8289b63f36eec1d7a7b"), fe("0d6e46bf1e3725ff884f82602321db7d05c152349b4cd1117195e5f778f9c27b"), fe("285754e689291a5f02e4a3c9b07359d3fc33a687a755f842cc45a037774d0542"), fe("09a4884b8ce2a5dc8eee7e181526dd65567e70aa4cb62c3d128e7d94345a4dc4"), fe("06af44dac4ca6cc95e692a20907607defa711623ca94934bea9d70bd555a594d"), fe("0f8b7738afe6bd0d66cb58970bf7484be2c67a4519d1406f074ae165ab5d2ad5"), fe("294dbe90e673accdcc6d7211bb0ac3aab902a88476ef7f7ac6fe3ba7b128c71a"), fe("05c3f9cecad533b14bace3f9d7d7713ccf40c9429b4fce2cfa3aa4ee3d4ae039"), fe("26cbff872ac3df2a3787878f24ee28b6ad4f1dcab41126b80f4038f40510d7e1"), fe("1ba0b493c987b9c1424ede9239ba100dc005e717aa71ca6d6a605561e379bdce")]
    }
}

// ── Chaum-Pedersen DLog-equality (`oprf-committee-dev/src/dlog.rs`'s `verify`) ────────────

/// Recomputes the Fiat-Shamir challenge exactly as `dlog.rs`/`dlog.nr`'s
/// `verify_dlog_equality` does:
/// `Poseidon2_t16([ds_dlog, a.x,a.y, b.x,b.y, c.x,c.y, G.x,G.y, r1.x,r1.y, r2.x,r2.y, 0,0,0])[1]`.
fn challenge(
    ds_dlog: Fr,
    a: &Point,
    b: &Point,
    c: &Point,
    generator: &Point,
    r1: &Point,
    r2: &Point,
) -> Fr {
    let input = [
        ds_dlog, a.x, a.y, b.x, b.y, c.x, c.y, generator.x, generator.y, r1.x, r1.y, r2.x, r2.y,
        Fr::from(0u64), Fr::from(0u64), Fr::from(0u64),
    ];
    t16(input)[1]
}

/// The DLEQ relation: `a = x*G` and `c = x*b` for the same secret `x`, i.e. whoever produced
/// `c` used the same secret key as the one behind public key `a`, applied to `b`. `a` is the
/// committee's claimed OPRF public key, `b` is the query being answered, `c` is the claimed
/// OPRF evaluation of that query.
fn verify(e: Fr, s: Fr, a: &Point, b: &Point, c: &Point, ds_dlog: Fr) -> bool {
    if !(a.is_on_curve() && b.is_on_curve() && c.is_on_curve()) {
        return false;
    }
    if a.is_identity() || b.is_identity() || c.is_identity() {
        return false;
    }
    if !(b.check_sub_group() && c.check_sub_group()) {
        return false;
    }

    let generator = Point::generator();
    let s_bytes = fr_to_be_bytes(&s);
    let e_bytes = fr_to_be_bytes(&e);

    let gs = generator.scalar_mul(&s_bytes);
    let ae = a.scalar_mul(&e_bytes);
    let r1 = gs.subtract(&ae);

    let bs = b.scalar_mul(&s_bytes);
    let ce = c.scalar_mul(&e_bytes);
    let r2 = bs.subtract(&ce);

    if r1.is_identity() || r2.is_identity() {
        return false;
    }

    let recomputed = challenge(ds_dlog, a, b, c, &generator, &r1, &r2);
    recomputed == e
}

// ── Public entry points used by `submit_oprf_response` ─────────────────────────────────

/// Verifies a committee member's Chaum-Pedersen DLog-equality proof that `response_point` is a
/// genuine OPRF evaluation of `query_point` under the secret key behind `committee_pubkey`.
///
/// Binding to a specific query is implicit and structural, not a separate check: `query_point`
/// must be the *actual* `blinded_query` on file for the `query_id` being answered (the caller —
/// `submit_oprf_response` — is responsible for reading that from `PendingOprfQueries` storage
/// rather than trusting a caller-supplied value). A `dlog_proof` computed for a different query
/// point cannot satisfy the relation checked here against *this* `query_point`, by the
/// soundness of the Chaum-Pedersen protocol — so a response cannot be replayed against a query
/// other than the one it was actually computed for.
///
/// `dlog_proof` must be exactly 64 bytes: `e || s`, two big-endian canonical BN254
/// scalar-field elements (see `OprfResponseRecord`'s doc comment in `lib.rs`). Returns `false`
/// (never panics) for any malformed input: wrong-length proof, non-canonical field encoding,
/// off-curve or identity points, points outside the prime-order subgroup, or a proof that
/// simply doesn't verify.
pub fn verify_oprf_response(
    committee_pubkey: [u8; 64],
    query_point: [u8; 64],
    response_point: [u8; 64],
    dlog_proof: &[u8],
) -> bool {
    if dlog_proof.len() != 64 {
        return false;
    }
    let Some(e_bytes): Option<[u8; 32]> = dlog_proof[0..32].try_into().ok() else {
        return false;
    };
    let Some(s_bytes): Option<[u8; 32]> = dlog_proof[32..64].try_into().ok() else {
        return false;
    };
    let Some(e) = decode_fr(&e_bytes) else { return false };
    let Some(s) = decode_fr(&s_bytes) else { return false };
    let Some(pk) = decode_point(&committee_pubkey) else { return false };
    let Some(b) = decode_point(&query_point) else { return false };
    let Some(c) = decode_point(&response_point) else { return false };

    verify(e, s, &pk, &b, &c, ds_dlog())
}

/// `Poseidon2(pk.x, pk.y)` (the *sibling* `poseidon2-bn254` crate's sponge, matching
/// `circuits/oprf-identity-anchor/anchor/src/main.nr`'s `oprf_pk_hashes[i] = Poseidon2::hash([pk_i.x,
/// pk_i.y], 2)` — see this module's docs for why that's a different Poseidon2 instantiation
/// from the `t16` permutation above). `submit_oprf_response` checks this against the
/// governance-approved `OprfCommitteeKeys` entry for the response's `(scheme_version,
/// committee_slot)` before trusting the raw `committee_pubkey` point at all.
pub fn hash_committee_pubkey(committee_pubkey: [u8; 64]) -> [u8; 32] {
    let x: [u8; 32] = committee_pubkey[0..32].try_into().expect("slice is 32 bytes");
    let y: [u8; 32] = committee_pubkey[32..64].try_into().expect("slice is 32 bytes");
    poseidon2_bn254::hash_bytes(&[x, y])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fe_dec(dec: &str) -> Fr {
        // Small helper for the decimal-form constants the upstream KAT vectors use.
        let mut acc = Fr::from(0u64);
        for ch in dec.chars() {
            let digit = ch.to_digit(10).expect("decimal digit");
            acc = acc * Fr::from(10u64) + Fr::from(digit as u64);
        }
        acc
    }

    /// `poseidon2::bn254::permutation::test_16`, `t16([0..15])` — same vector
    /// `oprf-committee-dev/src/poseidon2_taceo.rs` validates `t16` against.
    #[test]
    fn t16_matches_upstream_vector_0_to_15() {
        let mut input = [Fr::from(0u64); 16];
        for i in 0..16u64 {
            input[i as usize] = Fr::from(i);
        }
        let out = t16(input);
        let expected = [
            "0fc2e6b758f493969e1d860f9a44ee3bdffdf796f382aa4ffb16fa4e9bcc333f",
            "0c118155a0dfeca3f91faf14a350511228ac33743be91249c6e0b3a635a50de4",
            "1a02b3a6571f22bb6392322d3f9f5de145b4f00bdf483072ce6188c30ba0f83d",
            "26631df6b2522ecde57413cd680ed590ded356e1c680f865f45be8eb960d1e06",
            "250ac4dfed40dc37bac9abe46f7bff3a80481d52a157ac80a1e5d39a5ed60e18",
            "17160980d8e7d9cb31addaf294cf047768bffd9fe433e8903b4ed262ee913f5b",
            "1d708a9f0995c2e0cd2f55e5dc795126f7191a0eb934ac8172bf54e520361ff6",
            "20721a18915e96e37e12c9697427f34d6a366787ea94ea65565c36813a0d77a3",
            "08671a9e58105eed9ac673249dcf22f08f098e3c6eb28f9eaa55d67d755972d0",
            "01e879484303c6d057128fbcc3a4222c779a62d3666df65d4e0b64c8031d7cc4",
            "239e2ce87955ebe19aaad000b38725b729f51175ab7d688f15d997edf0e3b7fc",
            "06be612f42b3ebdbade3fe199338c9118eb6b5fb760bda96e45443f130a8b2de",
            "11b2c04b4eb9e4844e5ddbb19b56059a815ed5d69405ba51786961235d5f073c",
            "006da33e2d57616c0ffc855b48d225a1237c3d80fc7e6b6e73b74e162b85c8a8",
            "0ef50c2615882523c6c73a69b4371332a066b2dc4b9630f186db47e3bfca88c8",
            "0e2ceb1f8fde5f80be1f41bd239fabdc2f6133a6a98920a55c42891c3a925152",
        ];
        for i in 0..16 {
            assert_eq!(out[i], fe(expected[i]), "mismatch at index {i}");
        }
    }

    /// `dlog.nr`'s `test_verify_dlog_equality` — the same real, fixed known-answer vector
    /// `oprf-committee-dev/src/dlog.rs`'s own test validates against.
    #[test]
    fn verify_matches_upstream_kat() {
        let ds_dlog_val = ds_dlog();
        let oprf_query_key = Point {
            x: fe("2003f27260a0b5ee81b84f66f8bf2761ea9557262a4bcd16db5ca7abdeee1885"),
            y: fe("1eb45d38c97f7e65ac1b76d234db3237d2860f2b25c43e020693ef92b5a5f793"),
        };
        let oprf_response_blinded = Point {
            x: fe_dec("6882462243439192795495492197995100450516328082301652413647059141168822449465"),
            y: fe_dec("11410248488379662098266045802345135482683496756414401793793460258484335221028"),
        };
        let oprf_pk = Point {
            x: fe_dec("16048296497646113681290127133582586009660277510307938775951186660467382774945"),
            y: fe_dec("13451097916688865791218925679662796109386737920791997438101375513111619197164"),
        };
        let dlog_e = fe_dec("5609293693019386176508931649877337091590878173635241438306548223920379307458");
        let dlog_s = fe_dec("1167493435914595771361530871033173621661932035514996719837354510862251986174");

        assert!(verify(
            dlog_e,
            dlog_s,
            &oprf_pk,
            &oprf_query_key,
            &oprf_response_blinded,
            ds_dlog_val
        ));

        // The top-level entry point should accept the same vector when given as byte arrays.
        let mut pk_raw = [0u8; 64];
        pk_raw[0..32].copy_from_slice(&fr_to_be_bytes(&oprf_pk.x));
        pk_raw[32..64].copy_from_slice(&fr_to_be_bytes(&oprf_pk.y));
        let mut query_raw = [0u8; 64];
        query_raw[0..32].copy_from_slice(&fr_to_be_bytes(&oprf_query_key.x));
        query_raw[32..64].copy_from_slice(&fr_to_be_bytes(&oprf_query_key.y));
        let mut response_raw = [0u8; 64];
        response_raw[0..32].copy_from_slice(&fr_to_be_bytes(&oprf_response_blinded.x));
        response_raw[32..64].copy_from_slice(&fr_to_be_bytes(&oprf_response_blinded.y));
        let mut proof = [0u8; 64];
        proof[0..32].copy_from_slice(&fr_to_be_bytes(&dlog_e));
        proof[32..64].copy_from_slice(&fr_to_be_bytes(&dlog_s));

        assert!(verify_oprf_response(pk_raw, query_raw, response_raw, &proof));
        // Sanity: mutating any single byte of the proof breaks verification.
        let mut bad_proof = proof;
        bad_proof[0] ^= 0x01;
        assert!(!verify_oprf_response(pk_raw, query_raw, response_raw, &bad_proof));
    }

    #[test]
    fn wrong_query_point_is_rejected() {
        let ds_dlog_val = ds_dlog();
        let oprf_pk = Point {
            x: fe_dec("16048296497646113681290127133582586009660277510307938775951186660467382774945"),
            y: fe_dec("13451097916688865791218925679662796109386737920791997438101375513111619197164"),
        };
        let oprf_query_key = Point {
            x: fe("2003f27260a0b5ee81b84f66f8bf2761ea9557262a4bcd16db5ca7abdeee1885"),
            y: fe("1eb45d38c97f7e65ac1b76d234db3237d2860f2b25c43e020693ef92b5a5f793"),
        };
        let oprf_response_blinded = Point {
            x: fe_dec("6882462243439192795495492197995100450516328082301652413647059141168822449465"),
            y: fe_dec("11410248488379662098266045802345135482683496756414401793793460258484335221028"),
        };
        let dlog_e = fe_dec("5609293693019386176508931649877337091590878173635241438306548223920379307458");
        let dlog_s = fe_dec("1167493435914595771361530871033173621661932035514996719837354510862251986174");

        // Sanity: the real query point verifies.
        assert!(verify(dlog_e, dlog_s, &oprf_pk, &oprf_query_key, &oprf_response_blinded, ds_dlog_val));

        // A different query point (still a valid subgroup point — the generator) must NOT
        // verify: the proof was computed for `oprf_query_key`, not this one.
        let wrong_query = Point::generator();
        assert!(!verify(dlog_e, dlog_s, &oprf_pk, &wrong_query, &oprf_response_blinded, ds_dlog_val));
    }
}
