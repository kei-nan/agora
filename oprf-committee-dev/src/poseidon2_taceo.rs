//! Raw Poseidon2 permutation, `TaceoLabs/noir-poseidon` v0.6.1's `bn254::permutation`
//! variant — **not** the same Poseidon2 instantiation as `noir-lang/poseidon` v0.3.0 (which
//! `pallets/poseidon2-bn254` in the main repo already ports and validates). Changelog entry
//! 75 in this repo found the two are independently parameterized and NOT interchangeable
//! (different round-constant derivation for the same curve/field) — this module is the
//! *other* one, needed because `oprf-nr`'s `dlog.rs` (Chaum-Pedersen challenge, state width
//! 16) and `hash_to_curve.rs` (Elligator2's `hash_to_field`, state width 3) both explicitly
//! `use poseidon2::bn254::permutation`, i.e. this crate, not `noir-lang/poseidon`.
//!
//! All round constants below (`consts` submodule) are transcribed verbatim from
//! `TaceoLabs/noir-poseidon` v0.6.1's `poseidon2/src/bn254/consts.nr` — specifically, a
//! script (not hand-typing) extracted every hex literal from that real vendored source file
//! (see `~/.nargo/github.com/TaceoLabs/noir-poseidon/v0.6.1/` in this dev environment, the
//! same on-disk git-dependency cache `nargo` itself resolved `oprf = { tag = "v1.0.0", ...}`
//! against) and reassembled them into Rust array literals with no manual retyping of any
//! digit. The permutation structure (`permute`, `external`/`internal` linear layers,
//! `sbox`/`sbox_e`) is a line-for-line port of `poseidon2/src/bn254/permutation.nr` and
//! `hash_utils/src/poseidon.nr` from the same tag.
//!
//! # Validation
//!
//! `mod tests` checks `t3`/`t16` against real test vectors embedded in
//! `poseidon2/src/bn254/permutation.nr` itself (`test_3`/`test_16`) — not derived from this
//! port, copied from the upstream test file. Every vector must match bit-for-bit; if it
//! doesn't, that is a hard failure of this module, not a warning.

use ark_bn254::Fr;
use ark_ff::PrimeField;

fn fe(hex_str: &str) -> Fr {
    let bytes = hex::decode(hex_str).expect("round-constant hex literals are valid hex");
    Fr::from_be_bytes_mod_order(&bytes)
}

fn double(x: Fr) -> Fr {
    x + x
}

/// `hash_utils::poseidon::sbox_e` — x^5.
fn sbox_e(x: Fr) -> Fr {
    let mut result = x * x;
    result *= result;
    result * x
}

fn sbox<const T: usize>(mut state: [Fr; T]) -> [Fr; T] {
    for s in state.iter_mut() {
        *s = sbox_e(*s);
    }
    state
}

fn vec_add<const T: usize>(lhs: [Fr; T], rhs: [Fr; T]) -> [Fr; T] {
    let mut out = [Fr::from(0u64); T];
    for i in 0..T {
        out[i] = lhs[i] + rhs[i];
    }
    out
}

/// `hash_utils::poseidon::mds_4` — the width-4 MDS block used both standalone (t4, unused
/// here) and as a building block of the width-16 external round.
fn mds_4(state: [Fr; 4]) -> [Fr; 4] {
    let t0 = state[0] + state[1];
    let t1 = state[2] + state[3];
    let t2 = double(state[1]) + t1;
    let t3 = double(state[3]) + t0;
    let t4 = double(double(t1)) + t3;
    let t5 = double(double(t0)) + t2;
    [t3 + t5, t5, t2 + t4, t4]
}

/// `poseidon2::bn254::permutation::external_3` — t=3's external (full-round) linear layer:
/// `circ(2,1,1)`, i.e. add the sum of all three elements to each.
fn external_3(mut state: [Fr; 3]) -> [Fr; 3] {
    let sum = state[0] + state[1] + state[2];
    for s in state.iter_mut() {
        *s += sum;
    }
    state
}

/// `poseidon2::bn254::permutation::internal_3` — t=3's internal (partial-round) linear
/// layer, diag(1,1,2) + all-ones.
fn internal_3(state: [Fr; 3]) -> [Fr; 3] {
    let sum = state[0] + state[1] + state[2];
    [state[0] + sum, state[1] + sum, double(state[2]) + sum]
}

/// `poseidon2::bn254::permutation::external::<T>` for `T = 16` (four width-4 MDS blocks,
/// diffused across blocks by adding each block's per-position sum to every block).
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

/// `poseidon2::bn254::permutation::diag_mat_mul` specialized via `internal_16`'s call site:
/// `state[i] = state[i] * diag[i] + sum(state)`.
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

/// `poseidon2::bn254::permute_bn254` — the shared round structure for every state width:
/// one external mix, 4 full rounds (rc + sbox + external mix), `r_p` partial rounds (rc on
/// `state[0]` + sbox on `state[0]` + internal mix), 4 more full rounds.
fn permute<const T: usize, const R: usize>(
    mut state: [Fr; T],
    first_full_rc: [[Fr; T]; 4],
    partial_rc: [Fr; R],
    second_full_rc: [[Fr; T]; 4],
    external: impl Fn([Fr; T]) -> [Fr; T],
    internal: impl Fn([Fr; T]) -> [Fr; T],
) -> [Fr; T] {
    state = external(state);
    for r in 0..4 {
        state = vec_add(state, first_full_rc[r]);
        state = sbox(state);
        state = external(state);
    }
    for r in 0..R {
        state[0] += partial_rc[r];
        state[0] = sbox_e(state[0]);
        state = internal(state);
    }
    for r in 0..4 {
        state = vec_add(state, second_full_rc[r]);
        state = sbox(state);
        state = external(state);
    }
    state
}

/// `poseidon2::bn254::permutation::t3`.
pub fn t3(input: [Fr; 3]) -> [Fr; 3] {
    permute::<3, 56>(
        input,
        consts::x5_3_first_full_rc(),
        consts::x5_3_partial_rc(),
        consts::x5_3_second_full_rc(),
        external_3,
        internal_3,
    )
}

/// `poseidon2::bn254::permutation::t16`.
pub fn t16(input: [Fr; 16]) -> [Fr; 16] {
    let diag = consts::internal_diag16();
    permute::<16, 57>(
        input,
        consts::x5_16_first_full_rc(),
        consts::x5_16_partial_rc(),
        consts::x5_16_second_full_rc(),
        external_16,
        |s| internal_16(s, diag),
    )
}

mod consts {
    use super::fe;
    use ark_bn254::Fr;

// AUTO-EXTRACTED from TaceoLabs/noir-poseidon v0.6.1 poseidon2/src/bn254/consts.nr
// (constant transcription only, produced by a script that greps the real source file
// verbatim — not retyped or derived by hand). See poseidon2_taceo.rs module docs.

pub(crate) fn x5_3_first_full_rc() -> [[Fr; 3]; 4] {
    [
        [fe("1d066a255517b7fd8bddd3a93f7804ef7f8fcde48bb4c37a59a09a1a97052816"), fe("29daefb55f6f2dc6ac3f089cebcc6120b7c6fef31367b68eb7238547d32c1610"), fe("1f2cb1624a78ee001ecbd88ad959d7012572d76f08ec5c4f9e8b7ad7b0b4e1d1")],
        [fe("0aad2e79f15735f2bd77c0ed3d14aa27b11f092a53bbc6e1db0672ded84f31e5"), fe("2252624f8617738cd6f661dd4094375f37028a98f1dece66091ccf1595b43f28"), fe("1a24913a928b38485a65a84a291da1ff91c20626524b2b87d49f4f2c9018d735")],
        [fe("22fc468f1759b74d7bfc427b5f11ebb10a41515ddff497b14fd6dae1508fc47a"), fe("1059ca787f1f89ed9cd026e9c9ca107ae61956ff0b4121d5efd65515617f6e4d"), fe("02be9473358461d8f61f3536d877de982123011f0bf6f155a45cbbfae8b981ce")],
        [fe("0ec96c8e32962d462778a749c82ed623aba9b669ac5b8736a1ff3a441a5084a4"), fe("292f906e073677405442d9553c45fa3f5a47a7cdb8c99f9648fb2e4d814df57e"), fe("274982444157b86726c11b9a0f5e39a5cc611160a394ea460c63f0b2ffe5657e")],
    ]
}

pub(crate) fn x5_3_partial_rc() -> [Fr; 56] {
    [fe("1a1d063e54b1e764b63e1855bff015b8cedd192f47308731499573f23597d4b5"), fe("26abc66f3fdf8e68839d10956259063708235dccc1aa3793b91b002c5b257c37"), fe("0c7c64a9d887385381a578cfed5aed370754427aabca92a70b3c2b12ff4d7be8"), fe("1cf5998769e9fab79e17f0b6d08b2d1eba2ebac30dc386b0edd383831354b495"), fe("0f5e3a8566be31b7564ca60461e9e08b19828764a9669bc17aba0b97e66b0109"), fe("18df6a9d19ea90d895e60e4db0794a01f359a53a180b7d4b42bf3d7a531c976e"), fe("04f7bf2c5c0538ac6e4b782c3c6e601ad0ea1d3a3b9d25ef4e324055fa3123dc"), fe("29c76ce22255206e3c40058523748531e770c0584aa2328ce55d54628b89ebe6"), fe("198d425a45b78e85c053659ab4347f5d65b1b8e9c6108dbe00e0e945dbc5ff15"), fe("25ee27ab6296cd5e6af3cc79c598a1daa7ff7f6878b3c49d49d3a9a90c3fdf74"), fe("138ea8e0af41a1e024561001c0b6eb1505845d7d0c55b1b2c0f88687a96d1381"), fe("306197fb3fab671ef6e7c2cba2eefd0e42851b5b9811f2ca4013370a01d95687"), fe("1a0c7d52dc32a4432b66f0b4894d4f1a21db7565e5b4250486419eaf00e8f620"), fe("2b46b418de80915f3ff86a8e5c8bdfccebfbe5f55163cd6caa52997da2c54a9f"), fe("12d3e0dc0085873701f8b777b9673af9613a1af5db48e05bfb46e312b5829f64"), fe("263390cf74dc3a8870f5002ed21d089ffb2bf768230f648dba338a5cb19b3a1f"), fe("0a14f33a5fe668a60ac884b4ca607ad0f8abb5af40f96f1d7d543db52b003dcd"), fe("28ead9c586513eab1a5e86509d68b2da27be3a4f01171a1dd847df829bc683b9"), fe("1c6ab1c328c3c6430972031f1bdb2ac9888f0ea1abe71cffea16cda6e1a7416c"), fe("1fc7e71bc0b819792b2500239f7f8de04f6decd608cb98a932346015c5b42c94"), fe("03e107eb3a42b2ece380e0d860298f17c0c1e197c952650ee6dd85b93a0ddaa8"), fe("2d354a251f381a4669c0d52bf88b772c46452ca57c08697f454505f6941d78cd"), fe("094af88ab05d94baf687ef14bc566d1c522551d61606eda3d14b4606826f794b"), fe("19705b783bf3d2dc19bcaeabf02f8ca5e1ab5b6f2e3195a9d52b2d249d1396f7"), fe("09bf4acc3a8bce3f1fcc33fee54fc5b28723b16b7d740a3e60cef6852271200e"), fe("1803f8200db6013c50f83c0c8fab62843413732f301f7058543a073f3f3b5e4e"), fe("0f80afb5046244de30595b160b8d1f38bf6fb02d4454c0add41f7fef2faf3e5c"), fe("126ee1f8504f15c3d77f0088c1cfc964abcfcf643f4a6fea7dc3f98219529d78"), fe("23c203d10cfcc60f69bfb3d919552ca10ffb4ee63175ddf8ef86f991d7d0a591"), fe("2a2ae15d8b143709ec0d09705fa3a6303dec1ee4eec2cf747c5a339f7744fb94"), fe("07b60dee586ed6ef47e5c381ab6343ecc3d3b3006cb461bbb6b5d89081970b2b"), fe("27316b559be3edfd885d95c494c1ae3d8a98a320baa7d152132cfe583c9311bd"), fe("1d5c49ba157c32b8d8937cb2d3f84311ef834cc2a743ed662f5f9af0c0342e76"), fe("2f8b124e78163b2f332774e0b850b5ec09c01bf6979938f67c24bd5940968488"), fe("1e6843a5457416b6dc5b7aa09a9ce21b1d4cba6554e51d84665f75260113b3d5"), fe("11cdf00a35f650c55fca25c9929c8ad9a68daf9ac6a189ab1f5bc79f21641d4b"), fe("21632de3d3bbc5e42ef36e588158d6d4608b2815c77355b7e82b5b9b7eb560bc"), fe("0de625758452efbd97b27025fbd245e0255ae48ef2a329e449d7b5c51c18498a"), fe("2ad253c053e75213e2febfd4d976cc01dd9e1e1c6f0fb6b09b09546ba0838098"), fe("1d6b169ed63872dc6ec7681ec39b3be93dd49cdd13c813b7d35702e38d60b077"), fe("1660b740a143664bb9127c4941b67fed0be3ea70a24d5568c3a54e706cfef7fe"), fe("0065a92d1de81f34114f4ca2deef76e0ceacdddb12cf879096a29f10376ccbfe"), fe("1f11f065202535987367f823da7d672c353ebe2ccbc4869bcf30d50a5871040d"), fe("26596f5c5dd5a5d1b437ce7b14a2c3dd3bd1d1a39b6759ba110852d17df0693e"), fe("16f49bc727e45a2f7bf3056efcf8b6d38539c4163a5f1e706743db15af91860f"), fe("1abe1deb45b3e3119954175efb331bf4568feaf7ea8b3dc5e1a4e7438dd39e5f"), fe("0e426ccab66984d1d8993a74ca548b779f5db92aaec5f102020d34aea15fba59"), fe("0e7c30c2e2e8957f4933bd1942053f1f0071684b902d534fa841924303f6a6c6"), fe("0812a017ca92cf0a1622708fc7edff1d6166ded6e3528ead4c76e1f31d3fc69d"), fe("21a5ade3df2bc1b5bba949d1db96040068afe5026edd7a9c2e276b47cf010d54"), fe("01f3035463816c84ad711bf1a058c6c6bd101945f50e5afe72b1a5233f8749ce"), fe("0b115572f038c0e2028c2aafc2d06a5e8bf2f9398dbd0fdf4dcaa82b0f0c1c8b"), fe("1c38ec0b99b62fd4f0ef255543f50d2e27fc24db42bc910a3460613b6ef59e2f"), fe("1c89c6d9666272e8425c3ff1f4ac737b2f5d314606a297d4b1d0b254d880c53e"), fe("03326e643580356bf6d44008ae4c042a21ad4880097a5eb38b71e2311bb88f8f"), fe("268076b0054fb73f67cee9ea0e51e3ad50f27a6434b5dceb5bdde2299910a4c9")]
}

pub(crate) fn x5_3_second_full_rc() -> [[Fr; 3]; 4] {
    [
        [fe("1acd63c67fbc9ab1626ed93491bda32e5da18ea9d8e4f10178d04aa6f8747ad0"), fe("19f8a5d670e8ab66c4e3144be58ef6901bf93375e2323ec3ca8c86cd2a28b5a5"), fe("1c0dc443519ad7a86efa40d2df10a011068193ea51f6c92ae1cfbb5f7b9b6893")],
        [fe("14b39e7aa4068dbe50fe7190e421dc19fbeab33cb4f6a2c4180e4c3224987d3d"), fe("1d449b71bd826ec58f28c63ea6c561b7b820fc519f01f021afb1e35e28b0795e"), fe("1ea2c9a89baaddbb60fa97fe60fe9d8e89de141689d1252276524dc0a9e987fc")],
        [fe("0478d66d43535a8cb57e9c1c3d6a2bd7591f9a46a0e9c058134d5cefdb3c7ff1"), fe("19272db71eece6a6f608f3b2717f9cd2662e26ad86c400b21cde5e4a7b00bebe"), fe("14226537335cab33c749c746f09208abb2dd1bd66a87ef75039be846af134166")],
        [fe("01fd6af15956294f9dfe38c0d976a088b21c21e4a1c2e823f912f44961f9a9ce"), fe("18e5abedd626ec307bca190b8b2cab1aaee2e62ed229ba5a5ad8518d4e5f2a57"), fe("0fc1bbceba0590f5abbdffa6d3b35e3297c021a3a409926d0e2d54dc1c84fda6")],
    ]
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// `poseidon2::bn254::permutation::test_3`, vector 1 of 9 (`t3([0,1,2])`).
    #[test]
    fn t3_matches_upstream_vector_0_1_2() {
        let out = t3([Fr::from(0u64), Fr::from(1u64), Fr::from(2u64)]);
        assert_eq!(out[0], fe("0bb61d24daca55eebcb1929a82650f328134334da98ea4f847f760054f4a3033"));
        assert_eq!(out[1], fe("303b6f7c86d043bfcbcc80214f26a30277a15d3f74ca654992defe7ff8d03570"));
        assert_eq!(out[2], fe("1ed25194542b12eef8617361c3ba7c52e660b145994427cc86296242cf766ec8"));
    }

    /// `poseidon2::bn254::permutation::test_3`, a random-input vector (not just small ints).
    #[test]
    fn t3_matches_upstream_random_vector() {
        let out = t3([
            fe("2c6422c33190d036a17bd4281738ad60a6b4544c1020da1c0c84880a0ddc71c4"),
            fe("245cd98e5af9a6ebb35945b092c7e877ab9549c8919940250956a0bfedb457ab"),
            fe("0b43c424171231016dfe2072518b825a18c759383dba4e09a47bcd8b1a55da21"),
        ]);
        assert_eq!(out[0], fe("0b6f503d74ca8c80934b48d8d9e41c239ea6bcee17f658d416a0b72fd7daf1b8"));
        assert_eq!(out[1], fe("2845997bb81ad9d29f0b7ba57550cb7160b6930c70c92287207c7b5f65b2814b"));
        assert_eq!(out[2], fe("0a97e625f336a7c5e51bb2881e3b4e224f6e2e01ae5d698fa19446dbc407ac3f"));
    }

    /// `poseidon2::bn254::permutation::test_16`, `t16([0..15])`.
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

    /// `poseidon2::bn254::permutation::test_16`, the first random-input vector.
    #[test]
    fn t16_matches_upstream_random_vector() {
        let input = [
            "2ef5bd70482d1d5bce16a04a5220175878dc715c51070672fd79df2f36fb719d",
            "015425a96665c5004e9210cf6828d8019c057cd587408a27347d951d9dffc2d9",
            "1551585c9c4cd84962ce750509aa7c10f6070167cf02f413b1ebb99b46070f9d",
            "13b3e781956e21bfa1ab9b729c8870cd6834165331261b613ed155bec20c705c",
            "1efcc38f5d29388649ac90a53f2f18199b3b85b18a817e0ecc35fc8bc0c8f935",
            "301c505d503072bf6273b63ddb8ab6decf288fe0ca24e01e9d29d1c2b445840f",
            "1d0f515363f3f1d7a730fa40ee5adfc9dbc64ee624f748578583724b9c950d4d",
            "08d0a79b8f7f9900e653719a0fa83eddea4cb60912ab072a7c53c5e06be06f57",
            "257634dc118a69195eba44a7d683b806b13b6954284c00df03fa6fec498d7136",
            "1c3a1e671b80722c3093704597f0682648d4ed2edf527c394437f83455c6dc9f",
            "2b9b5fa799f3e7e3b25f5789aea0ad85cf688df558df55cd5aa01e08ad61a0f3",
            "2976d9a19d3daaa5d4754b07212dd1790e1a2e792bb1f3f3c1fdf29cb9876a0c",
            "024e57dba7a40fcd9c055126821db288671ca25c7a2d331f0a44aafd281c420c",
            "2ace2b5e9b3840a1bf285ba01256a92c847585a0387d82959331e1ff831fe7eb",
            "095eee248104564b4123f8b16cd2e659a84ee02453ee37267a42d29fb4698efe",
            "023a74b7eec73c64bf4c824bf7b15004252f23594d733e8b5e3df99057a1a402",
        ];
        let mut in_fe = [Fr::from(0u64); 16];
        for i in 0..16 {
            in_fe[i] = fe(input[i]);
        }
        let out = t16(in_fe);
        let expected = [
            "199f8c0ddd4fc19fbabcce889b860aea0840b953d7cb9897a24c03aa91321f30",
            "0488d23f20fce69d1c4f405f407987c50e605487002856e3d100a531fbef76b9",
            "263a13c6a63a36b7f83690126f3da9083932a4c310fc8273faf7fc7d4106a61b",
            "0c819a478981b1e3056aa6eacb7ca03c75df4ef8106dea2e920c6a30ee6eb295",
            "186a0cfec51820f97f3c64ca9a3f9fe84d728c3fc65298308db8c252e7763d2a",
            "1de58fac635c3539da015d1ae4f775af54ca361c1bb33949146ebc92ec08ed05",
            "22cdb2286d74a378ab0e73265d8ea805e70487bab78b09624fae0111e72e7c17",
            "2a530cf261e7fa2c5610fd39d40e5363d238b66127bc4da42d3c3e41252f2746",
            "1435915959547c0b78257f1ceaddf744d24f20054b06901686a409f3fb7fdd02",
            "07d3142e793fc9b5d09fb56da48bdbaf9f3b0c0a60e65c22faea8abcd81fc68a",
            "0b68ef91843d5f6a4d2c9274f6ddaa7bd6d78677502d1f2f6b220016fa569b92",
            "08473d26da0495891ba80a6b8a306c62d0084260690ccd9f5528a11bf25d18aa",
            "2a6c3ab5f6ec231bfab2908184025cf5107cd9231d9e24bfe4676f93e4232528",
            "1e7b9102952e2c43c201ca85d589c2ee2f15a0fc44d3c1b14674044b4380f9bc",
            "3013bd07b9ff2be6b049d89513b0a65965980986383a64cd5dee34a1f5952917",
            "02bfe073ffc8a0c7cc5c2887333dfa9252df70099500399034bf491bb4cee13a",
        ];
        for i in 0..16 {
            assert_eq!(out[i], fe(expected[i]), "mismatch at index {i}");
        }
    }
}
