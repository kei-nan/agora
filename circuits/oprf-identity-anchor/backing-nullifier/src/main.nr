//! Agora **backing-nullifier** circuit — a Semaphore/nullifier-scheme proof, *not* a
//! passport-parsing circuit.
//!
//! # What this proves
//!
//! "I know `backing_root_secret` (derived at registration per `derive_backing_root_term`'s
//! doc comment — see `../anchor`/`../disclosure`) and a `slot_index` in
//! `0..max_backings_per_citizen` such that `Poseidon2(DS_BACKING_COMMITMENT,
//! backing_root_secret)` is a leaf of the backing-commitment tree
//! `pallets/pallet-identity/src/lib.rs` publishes, at some root that was genuinely current
//! within that pallet's `BackingRootHistoryWindowBlocks` retention window — and I derive
//! `backing_nullifier = Poseidon2(DS_BACKING_NULLIFIER, backing_root_secret, slot_index)`."
//!
//! # Why this is a standalone circuit, unlike every other one in this workspace
//!
//! Every other circuit here (`anchor`/`disclosure`/`migrate`/`migrate-disclosure`/
//! `delegate-query`/`delegate-persona`) either parses a passport DG1 directly or exists to
//! ride inside ZKPassport's outer proof as a disclosure subproof, because — per `disclosure`'s
//! own module docs — a standalone proof's `comm_in` is otherwise an unauthenticated value the
//! prover could pick freely.
//!
//! This circuit touches no passport data at all: `backing_root_secret` is a value the
//! citizen's wallet already derived and cached once, at registration (see `derive_backing_root_
//! term`'s doc comment for why reusing the registration OPRF evaluation is safe here). There is
//! nothing for a standalone proof to be unsafely unbound *from* — the entire statement is "I
//! know a secret whose derived commitment is a leaf of this published tree", exactly the shape
//! of a Semaphore group-membership proof, and it is verified the same way `crate::verifier`
//! verifies the outer ZKPassport proof: a genuine standalone UltraHonk pairing check against
//! this circuit's own verification key (see `runtime/src/backing_nullifier_verifier.rs`), not
//! folded into any outer proof's `param_commitments`.
//!
//! # Binding the target delegate (anti-front-running)
//!
//! `delegate_persona_id` (the delegate this backing action targets, from
//! `../delegate-persona`) is a plain `pub Field` parameter, deliberately **not** folded into a
//! hash-based commitment the way `delegate-persona/src/main.nr` folds `persona_account` into
//! `param_commitment`.
//!
//! That is a considered deviation, not an oversight. `delegate-persona` needed the hash-fold
//! trick because it rides inside ZKPassport's outer circuit, whose disclosure-subproof
//! interface is a *fixed* 8-field layout with no room for an extra public output slot — folding
//! was the only way to bind `persona_account` at all. This circuit has no such constraint: it
//! is a standalone SNARK with a public-input list of its own choosing. A SNARK's public inputs
//! are cryptographically bound into the verification equation regardless of whether the
//! circuit body's constraints reference them — this codebase already leans on exactly that
//! property elsewhere (`disclosure`/`migrate-disclosure`'s `service_scope`/`service_subscope`
//! are declared `pub` and silenced with `let _ = ...` without ever being used in a constraint,
//! and are still part of the outer proof's checked statement), and it is independently
//! confirmed empirically by `runtime/src/verifier.rs`'s own
//! `rejects_mutated_public_inputs_for_a_real_proof` test, which flips one bit of *every* public
//! input word of a real bb 5.0.0 proof — including ones the circuit under test never
//! constrains — and the pairing check rejects every single mutation. So: an observer who lifts
//! a valid `(proof, root, delegate_persona_id, max_backings_per_citizen, backing_nullifier)`
//! tuple from the mempool and resubmits it with a different `delegate_persona_id` gets a proof
//! that fails verification, full stop — no `param_commitment`-style fold is needed to achieve
//! that here, and adding one would be complexity bought for a security property this design
//! already has for free.
//!
//! `delegate_persona_id` is therefore deliberately excluded from `backing_nullifier`'s own
//! formula (see `derive_backing_nullifier`'s doc comment): a citizen's slot nullifier stays
//! stable while they retarget that slot to a different delegate over time, and the target
//! binding lives entirely in this proof's public-input list, checked once, at verification
//! time, for *this* specific proof.
//!
//! # `max_backings_per_citizen`: a checked public input, not a compile-time constant
//!
//! `pallet_elections::MaxBackingsPerCitizen` (`pallets/pallet-elections/src/lib.rs`) is a
//! `StorageValue`, changeable at any time via `set_election_params` under `ConstitutionalOrigin`
//! — a genuinely governance-mutable "constitutional parameter", not a fixed deployment-time
//! constant. Baking a specific bound into this circuit at compile time would mean every future
//! governance change to the cap requires shipping a new circuit (a new VK, a new on-chain
//! allowlist entry, a hard migration cutover for every citizen's cached tooling) just to move a
//! number — disproportionate cost for what governance is designed to be able to tune freely.
//!
//! So the bound is instead taken as a public input, checked in-circuit only for internal
//! consistency (`assert_lt(slot_index, max_backings_per_citizen)` below), with the caller
//! responsible for checking `max_backings_per_citizen` equals the *current* on-chain value
//! before accepting the proof — exactly the same split `runtime/src/anchor_verifier.rs`'s
//! module docs describe for `oprf_pk_hashes`/`current_date`: this module does the
//! cryptographic/structural work, a pallet extrinsic (now built as of commit `786b792` — see
//! "Status" below) does the storage-dependent checks, mirroring how `register_citizen` already
//! checks `AllowedMerkleRoots`/`OprfCommitteeKeys` itself rather than delegating that to
//! `T::ZkVerifier`.
//!
//! The tradeoff this buys: a proof proves "slot_index is within *some* claimed bound", not
//! "within the bound governance has actually set" — that second, load-bearing half is a
//! Rust-side check against live chain storage, not a circuit-level guarantee. A future consumer
//! that forgets to perform it would accept backings using slot indices beyond the real,
//! current cap. `slot_index` itself has no separate range check beyond `assert_lt` — since
//! `max_backings_per_citizen` is itself always `<= u32::MAX` by construction on the Rust side
//! (`pallet_elections::MaxBackingsPerCitizen` is a `u32`), bounding `slot_index` below it is
//! already a complete bound.
//!
//! # Status
//!
//! Compiles, produces a bb 5.0.0 verification key, and has a full real `nargo execute` ->
//! `bb prove` -> `bb verify` round-trip (see `Prover.toml`, built from a genuine
//! `pallets/pallet-identity`-side vector — see that file's header comment). **Update, commit
//! `786b792`**: a pallet extrinsic now genuinely consumes this circuit's output —
//! `pallet_elections::back_delegate`/`remove_backing` (`pallets/pallet-elections/src/lib.rs`,
//! via the `verify_backing_proof` helper) check proof validity (`T::BackingProofVerifier`), root
//! freshness (`T::BackingRootChecker`), the proof's public `delegate_persona_id` against the
//! call's target delegate, `max_backings_per_citizen` against the live `MaxBackingsPerCitizen`
//! storage value, and reuse of `backing_nullifier` against `UsedBackingNullifier` — all of the
//! storage-dependent checks this module's docs above describe as this circuit's own
//! responsibility to leave to a future consumer.

use identity_anchor::{
    backing_tree_node_hash, derive_backing_commitment, derive_backing_nullifier,
    BACKING_TREE_DEPTH,
};
use std::field::bn254::assert_lt;

/// Public inputs:  `root`, `delegate_persona_id`, `max_backings_per_citizen`
/// Public output:  `backing_nullifier`
///
/// # Private witnesses
/// - `backing_root_secret` — the citizen's cached per-registration secret (see module docs).
/// - `slot_index` — which of the citizen's `0..max_backings_per_citizen` backing slots this
///   proof is for.
/// - `leaf_index` — this citizen's permanent position in the backing-commitment tree
///   (`pallets/pallet-identity`'s `BackingLeafIndexOf`). Private so that proving membership
///   does not reveal *which* leaf — the point of a nullifier scheme.
/// - `merkle_siblings` — the `BACKING_TREE_DEPTH` sibling hashes on `leaf_index`'s
///   authentication path, level 0 (adjacent to the leaf) first.
fn main(
    root: pub Field,
    delegate_persona_id: pub Field,
    max_backings_per_citizen: pub Field,
    backing_root_secret: Field,
    slot_index: Field,
    leaf_index: Field,
    merkle_siblings: [Field; BACKING_TREE_DEPTH],
) -> pub Field {
    // The real enforcement of `MaxBackingsPerCitizen` — see module docs. A genuine full-field
    // comparison (not a truncating `as u32` cast, which would let a `slot_index` far outside
    // u32 range pass a narrowed check while still producing a distinct, unbounded nullifier
    // below).
    assert_lt(slot_index, max_backings_per_citizen);

    // Reconstruct the authentication path from the leaf up to the claimed root. `to_le_bits`
    // is a genuinely constrained decomposition (unlike `as u32`): it fails the proof outright
    // if `leaf_index >= 2^BACKING_TREE_DEPTH`, so `index_bits[level]` are real, checked path
    // directions, not attacker-chosen bits smuggled in as a separate witness.
    let leaf = derive_backing_commitment(backing_root_secret);
    let index_bits: [bool; BACKING_TREE_DEPTH] = leaf_index.to_le_bits();

    let mut current = leaf;
    for level in 0..BACKING_TREE_DEPTH {
        let sibling = merkle_siblings[level];
        // Matches `pallets/pallet-identity/src/lib.rs`'s `recompute_backing_tree_path` exactly:
        // an even index at this level is a left child (`hash(current, sibling)`), odd is a
        // right child (`hash(sibling, current)`) — `to_le_bits`'s bit `level` is precisely
        // `leaf_index`'s parity at that level of the halving `index /= 2` walk that function
        // performs.
        current = if index_bits[level] {
            backing_tree_node_hash(sibling, current)
        } else {
            backing_tree_node_hash(current, sibling)
        };
    }
    assert(
        current == root,
        "Backing commitment is not a member of the published tree at this root",
    );

    let backing_nullifier = derive_backing_nullifier(backing_root_secret, slot_index);

    // `delegate_persona_id` binds via the SNARK's own public-input mechanism, not via any
    // constraint in this circuit body — see module docs for why that is sufficient here.
    let _ = delegate_persona_id;

    backing_nullifier
}

mod tests {
    use super::main;
    use identity_anchor::{backing_tree_node_hash, derive_backing_commitment, BACKING_TREE_DEPTH};

    /// The exact fixture in `Prover.toml`, inlined so `nargo test` exercises the same
    /// constraint system `nargo execute --prover-name Prover` does, without needing a
    /// filesystem read. See `Prover.toml`'s header for where these numbers come from.
    struct Fixture {
        root: Field,
        delegate_persona_id: Field,
        max_backings_per_citizen: Field,
        backing_root_secret: Field,
        slot_index: Field,
        leaf_index: Field,
        merkle_siblings: [Field; BACKING_TREE_DEPTH],
    }

    fn fixture() -> Fixture {
        Fixture {
            root: 0x09efbc7edc789c162ded75ed12f2d9e84fb8822535552d9d2f80ce535762f46e,
            delegate_persona_id: 777,
            max_backings_per_citizen: 5,
            backing_root_secret: 424242,
            slot_index: 2,
            leaf_index: 0,
            merkle_siblings: [
                0x16a6d47edce7b0993b10816d6d4460cc0f9c01845de1d399e3b0f8aaadef0b6b,
                0x0ff1c447d1ef5a528694a4136c2fb5a73c39955431759aa4933b26c82804d869,
                0x127a2e0a0fabf20e79b74f0b1dc142ef34b1a2b96897949ca8eda9c44cc15cec,
                0x24f38748daca7d742ff1e89fd2e392bc1b6f1350f2c6b535510bec20072653ee,
                0x17cc1dad0ceb51eac2e61ae17ce872aa685f220975666a8219c0c48acd07a3d8,
                0x2338faeb8f671731e719db24bc3120cd8fdca925f14222339790fb30096e4485,
                0x2fd1363825730a9c256b4065cb46a7e077e3a7dd18d63a7fe43e9590a4b6491a,
                0x0a074ddb0ca408f791508933f10d1d366b66aa53eeb1f57ecefd940eaba47d68,
                0x2d393d8911c5db59aa9abfcc0e48a751bb1b0cbaad41dd2e60de454540dd65dc,
                0x02b94f4247a758aac8c3142a10174941a1fc077bf9cbaad6ce5e3677d0e3d0ff,
                0x1275a9f2fd5bebca3e76d36a0cea99034f873e8c91765de4b2bbe641a094b4d3,
                0x0f937d9589069a406c8603b2ef608cf6213db4fc77fb036260a1fa5e41bafe49,
                0x2888aff9c0711a4e9df287fb43f8db4dfe5d6cb6d2b44fce6c25fb657255e0be,
                0x068d823f95c20eff7403f57dd08391ecddfb573af764bda91e4a7439c31cea44,
                0x25c4e39a8e9fb74b45aa01654c45c8f35888c57a7073fe01d4db2dec3df3abf3,
                0x1195bbd616681e2decf0aa8b5b1095e3418f8716eb12b2b852b772730d486ab4,
                0x277e87d20bcad693093566d14a8f52ec11ba52e4a7f2bb4817dd24c2a382bb0f,
                0x2ff5ff52088def59592b99e9ca9cfc806a11eb6529dc7a1d8804aff43ab919d7,
                0x18f2680546dd4484f95f7e8a77881634cdf0053917e7142cce299f888ba35ebd,
                0x2fdfd0706606aa1bdebaabb1a9252c40abacd3ee03006803e08ec7a6cc5a8dbd,
                0x2c4f3fa50306c38d44f2c971a74767fddc6f34e094f06b01c19a25e6cbe80316,
                0x16b241d5acc77ee8ff7c79930ccc934764c73a76e79049d4a1ca01ed347bc6ae,
                0x2eae37a1dbdacf6b9ce433f16bd0103b7f5a52f923ba70e84a501de61a9da063,
                0x2fab8211545d236550cbbab4fc8e8a37b68c639fadd87439f54d471427b0f747,
                0x1a28702748167233ae3d78271b4ca3c924a4d20ceb3ed5e17ea44447b18e28ad,
                0x2ea9b8751a33a12b4ff276ef8765edfbbde123e7744c1dd0666b3576deedd911,
                0x2483d2673ca9c6860686a40d7273071a50e016d302d4b2b496983bb1595d54a7,
                0x028e020c54878eb003e669b88af37968fc6c264937fece4ab11272dbf799f531,
                0x0eb394a8caf49da0ec70349d38670a0624afa9240454be94242b63419f33ad0b,
                0x19efdb04021cfb721d0f072340c01a4ab6de724381f9a3275152a40b0b6713ce,
                0x15f5e50cac7670592a741d1151f2f0a3de253681590dc38b37a6f69e1a0ed54a,
                0x1d74c0426d92db677c2ed90ca6d5ada72afc805a44c6bfa0e84bd81a90ce04da,
            ],
        }
    }

    #[test]
    fn executes_end_to_end_over_the_real_fixture() {
        let f = fixture();
        let nullifier = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
        // Real cross-check vector — see Prover.toml's header.
        let expected_nullifier =
            0x22b020294a8f8b82c34471b0c189da23fdacbbeaa2fec1427c327efa2b4d56e4;
        assert_eq(nullifier, expected_nullifier);
    }

    #[test(should_fail_with = "Backing commitment is not a member of the published tree at this root")]
    fn rejects_a_wrong_root() {
        let mut f = fixture();
        f.root += 1;
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test(should_fail_with = "Backing commitment is not a member of the published tree at this root")]
    fn rejects_a_wrong_backing_root_secret() {
        let mut f = fixture();
        f.backing_root_secret += 1;
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test(should_fail_with = "Backing commitment is not a member of the published tree at this root")]
    fn rejects_a_single_mutated_sibling() {
        let mut f = fixture();
        f.merkle_siblings[7] += 1;
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test(should_fail_with = "Backing commitment is not a member of the published tree at this root")]
    fn rejects_a_wrong_leaf_index() {
        let mut f = fixture();
        // A different leaf index walks a different (here: mismatched) path through the same
        // sibling values, so it recomputes a different, wrong root.
        f.leaf_index = 1;
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test(should_fail_with = "Field failed to decompose into specified 32 limbs")]
    fn rejects_a_leaf_index_that_does_not_fit_in_the_tree_depth() {
        let mut f = fixture();
        f.leaf_index = 0x100000000; // 2^32, one bit too many for BACKING_TREE_DEPTH = 32
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test]
    fn accepts_slot_index_at_the_top_of_the_valid_range() {
        // slot_index = max_backings_per_citizen - 1 is the largest allowed slot.
        let mut f = fixture();
        f.slot_index = f.max_backings_per_citizen - 1;
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test(should_fail_with = "call to assert_max_bit_size")]
    fn rejects_slot_index_equal_to_the_cap() {
        // slot_index must be strictly less than max_backings_per_citizen.
        let mut f = fixture();
        f.slot_index = f.max_backings_per_citizen;
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test(should_fail_with = "call to assert_max_bit_size")]
    fn rejects_slot_index_far_beyond_the_cap() {
        // Guards specifically against a truncating-cast bypass: a naive `slot_index as u32 <
        // max as u32` check could be fooled by a slot_index far outside u32 range whose low
        // bits happen to land under the cap. `assert_lt` must reject this over the full field,
        // not just check a narrowed truncation.
        let mut f = fixture();
        f.slot_index = 0x1_0000_0003; // truncates to 3 as u32, which IS < 5 — must still fail
        let _ = main(
            f.root,
            f.delegate_persona_id,
            f.max_backings_per_citizen,
            f.backing_root_secret,
            f.slot_index,
            f.leaf_index,
            f.merkle_siblings,
        );
    }

    #[test]
    fn delegate_persona_id_does_not_affect_the_nullifier() {
        // By design (see module docs): the nullifier identifies a (secret, slot) pair, not a
        // (secret, slot, delegate) triple, so retargeting a slot to a different delegate keeps
        // the same nullifier. The *proof* for a different delegate_persona_id is a genuinely
        // different SNARK statement (see runtime/src/backing_nullifier_verifier.rs's tests for
        // the on-chain-facing half of that claim) — this test only pins the in-circuit value.
        let f = fixture();
        let nullifier_a = main(
            f.root, f.delegate_persona_id, f.max_backings_per_citizen, f.backing_root_secret,
            f.slot_index, f.leaf_index, f.merkle_siblings,
        );
        let nullifier_b = main(
            f.root, f.delegate_persona_id + 1, f.max_backings_per_citizen, f.backing_root_secret,
            f.slot_index, f.leaf_index, f.merkle_siblings,
        );
        assert_eq(nullifier_a, nullifier_b);
    }

    /// Sanity: the tree-walk half of `main`, inlined, must match `derive_backing_commitment` +
    /// `backing_tree_node_hash` composed by hand for `leaf_index = 0` — pins the left/right
    /// convention independently of `main`'s own internals.
    #[test]
    fn manual_tree_walk_matches_the_fixture_root() {
        let f = fixture();
        let mut current = derive_backing_commitment(f.backing_root_secret);
        for level in 0..BACKING_TREE_DEPTH {
            // leaf_index = 0 is a left child at every level.
            current = backing_tree_node_hash(current, f.merkle_siblings[level]);
        }
        assert_eq(current, f.root);
    }
}
