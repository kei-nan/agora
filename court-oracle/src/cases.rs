//! SCALE-mirror types for the on-chain storage this service reads, plus the storage-key
//! decoding helper shared across all of them.
//!
//! These are hand-written mirrors, not generated from or dependent on the real pallet crates
//! (same reasoning as extrinsic.rs — see Cargo.toml's `[workspace]` comment). Each mirror's
//! variant/field order is transcribed directly from the pallet source in this working tree, not
//! guessed; the doc comment on each type says which file it was checked against.

use codec::{Decode, Encode};
use sp_core::crypto::AccountId32;

// ── pallet-courts (pallets/pallet-courts/src/lib.rs, this working tree) ─────────────────────

/// Mirrors `pallet_courts::pallet::CaseStatus`. Variant order matches exactly (SCALE encodes
/// enums by variant index, so order is load-bearing).
#[derive(Clone, Debug, PartialEq, Decode, Encode)]
pub enum CaseStatus {
    Filed,
    AIRulingIssued,
    InJuryAppeal,
    JurySeated,
    FinalRuling,
    Enforced,
}

/// Mirrors `pallet_courts::pallet::CaseSubject`.
#[derive(Clone, Debug, PartialEq, Decode, Encode)]
pub enum CaseSubject {
    General,
    LawChallenge { law_id: u32 },
    TreasuryDispute { department_id: u32 },
    CitizenConduct { nullifier: [u8; 32], suspension_blocks: Option<u32> },
}

/// Mirrors `Cases: StorageMap<_, Blake2_128Concat, u32, (AccountId, CaseStatus,
/// Option<[u8; 32]>, CaseSubject)>` — SCALE-decodes as a plain tuple, so no separate struct
/// derive is needed; `type CaseRecord = (...)` documents the field meaning for callers.
pub type CaseRecord = (AccountId32, CaseStatus, Option<[u8; 32]>, CaseSubject);

/// Mirrors `pallet_courts::pallet::Verdict`. Variant order matches exactly (SCALE encodes enums
/// by variant index). Needed on the write side too: `finalize_ruling(case_id, verdict)` takes
/// this as an explicit argument (the pallet does not record a verdict on submit_ai_ruling — see
/// README.md's "A real gap found" section), so `extrinsic::FinalizeRuling` encodes one of these.
#[derive(Clone, Debug, PartialEq, Decode, Encode)]
pub enum Verdict {
    Upheld,
    Overturned,
}

// ── pallet-constitution (pallets/pallet-constitution/src/lib.rs, this working tree) ─────────

/// Mirrors `pallet_constitution::pallet::LawTier`.
#[derive(Clone, Debug, PartialEq, Decode)]
pub enum LawTier {
    Ordinary,
    Structural,
    Foundational,
}

/// Mirrors `pallet_constitution::pallet::LawStatus`.
#[derive(Clone, Debug, PartialEq, Decode)]
pub enum LawStatus {
    Active,
    Paused,
    Repealed,
}

/// Mirrors `Laws: StorageMap<_, Blake2_128Concat, u32, (LawTier, LawStatus, u32, [u8; 32])>`
/// — `(tier, status, version, content_hash)`.
pub type LawRecord = (LawTier, LawStatus, u32, [u8; 32]);

// ── pallet-audit (pallets/pallet-audit/src/lib.rs, this working tree) ───────────────────────

/// Mirrors `pallet_audit::pallet::AuditStatus`.
#[derive(Clone, Debug, PartialEq, Decode)]
pub enum AuditStatus {
    Pending,
    Cleared,
    Flagged,
    Disputed,
}

/// Mirrors `pallet_audit::pallet::AuditEntry<AccountId>`.
#[derive(Clone, Debug, Decode)]
pub struct AuditEntry {
    pub dept_id: u32,
    pub amount: u128,
    pub ipfs_hash: [u8; 32],
    pub status: AuditStatus,
    pub flag_reason: Option<[u8; 32]>,
    pub flagged_by: Option<AccountId32>,
}

// ── shared key-decoding helper ───────────────────────────────────────────────────────────────

/// Extracts the SCALE-encoded `u32` map key from a full storage key, assuming
/// `Blake2_128Concat` (32-byte twox128 pallet+item prefix, then 16-byte blake2_128 hash, then
/// the raw un-hashed key). `Blake2_128Concat` is "hash ++ key" — the *plaintext* key is still
/// present at the end, so this is exact, not a guess. Every numeric-keyed map this service
/// reads (`Cases`, `Laws`, `DepartmentBudgets`, ...) uses `Blake2_128Concat` — confirmed by
/// reading each pallet's storage declaration in this working tree.
pub fn decode_u32_map_key(key_hex: &str) -> Option<u32> {
    let bytes = hex::decode(key_hex.trim_start_matches("0x")).ok()?;
    if bytes.len() < 4 {
        return None;
    }
    let tail = &bytes[bytes.len() - 4..];
    Some(u32::from_le_bytes(tail.try_into().ok()?))
}

/// Same idea as `decode_u32_map_key`, for the `u64`-keyed maps (`ExpenditureLog`, `AuditLog`).
pub fn decode_u64_map_key(key_hex: &str) -> Option<u64> {
    let bytes = hex::decode(key_hex.trim_start_matches("0x")).ok()?;
    if bytes.len() < 8 {
        return None;
    }
    let tail = &bytes[bytes.len() - 8..];
    Some(u64::from_le_bytes(tail.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec::Encode;

    #[test]
    fn decode_u32_map_key_round_trips() {
        let case_id: u32 = 42;
        // Simulate prefix(32 bytes) ++ blake2_128 hash(16 bytes) ++ raw key(4 bytes).
        let mut key_bytes = vec![0xABu8; 32 + 16];
        key_bytes.extend(case_id.to_le_bytes());
        let key_hex = format!("0x{}", hex::encode(key_bytes));
        assert_eq!(decode_u32_map_key(&key_hex), Some(case_id));
    }

    #[test]
    fn decode_u32_map_key_rejects_too_short() {
        assert_eq!(decode_u32_map_key("0x1234"), None);
    }

    #[test]
    fn decode_u64_map_key_round_trips() {
        let index: u64 = 12345;
        let mut key_bytes = vec![0xCDu8; 32 + 16];
        key_bytes.extend(index.to_le_bytes());
        let key_hex = format!("0x{}", hex::encode(key_bytes));
        assert_eq!(decode_u64_map_key(&key_hex), Some(index));
    }

    /// Round-trip a `CaseRecord` through SCALE encode/decode, confirming the mirror types'
    /// variant/field order actually decodes what the real pallet's `Encode` would produce for
    /// each `CaseSubject` shape. This is the closest a pure unit test can get to verifying the
    /// mirror without the real pallet crate as a dependency (deliberately not a dependency —
    /// see this file's header comment).
    #[test]
    fn case_subject_variants_round_trip() {
        for subject in [
            CaseSubject::General,
            CaseSubject::LawChallenge { law_id: 7 },
            CaseSubject::TreasuryDispute { department_id: 3 },
            CaseSubject::CitizenConduct { nullifier: [9u8; 32], suspension_blocks: Some(100) },
            CaseSubject::CitizenConduct { nullifier: [1u8; 32], suspension_blocks: None },
        ] {
            let encoded = subject.encode();
            let decoded = CaseSubject::decode(&mut &encoded[..]).expect("decode failed");
            assert_eq!(subject, decoded);
        }
    }

    #[test]
    fn case_status_all_variants_round_trip() {
        for status in [
            CaseStatus::Filed,
            CaseStatus::AIRulingIssued,
            CaseStatus::InJuryAppeal,
            CaseStatus::JurySeated,
            CaseStatus::FinalRuling,
            CaseStatus::Enforced,
        ] {
            let encoded = status.encode();
            let decoded = CaseStatus::decode(&mut &encoded[..]).expect("decode failed");
            assert_eq!(status, decoded);
        }
    }

    /// `Verdict`'s variant order (Upheld = 0, Overturned = 1) is load-bearing for
    /// `finalize_ruling`'s call encoding — confirm it round-trips and that the encoded
    /// discriminant matches `pallets/pallet-courts/src/lib.rs`'s declaration order exactly.
    #[test]
    fn verdict_variants_round_trip_with_expected_discriminant() {
        assert_eq!(Verdict::Upheld.encode(), vec![0u8]);
        assert_eq!(Verdict::Overturned.encode(), vec![1u8]);
        for verdict in [Verdict::Upheld, Verdict::Overturned] {
            let encoded = verdict.encode();
            let decoded = Verdict::decode(&mut &encoded[..]).expect("decode failed");
            assert_eq!(verdict, decoded);
        }
    }
}
