/**
 * Tests `oprfCombine.ts`'s honest stub. This module deliberately does NOT implement the
 * real BabyJubJub/Poseidon2-t3-t16 threshold-combination math — see its module doc comment
 * for the full accounting of why. Accordingly, this test file only confirms the stub
 * behaves as documented (throws a clear, on-topic error, and the module imports cleanly
 * with its exported types intact) — it does not, and must not, assert anything about the
 * correctness of math that isn't implemented.
 */
import {
  CombinedCommitteeProof,
  OprfCombinationUnimplementedError,
  OprfRound1Commitment,
  OprfRound2Response,
  combineCommitteeSlotResponses,
} from './oprfCombine';

function fakeRound1(index: number): OprfRound1Commitment {
  return {
    member: `5Member${index}`,
    index,
    rI: new Uint8Array(64).fill(index),
    dG: new Uint8Array(64).fill(index),
    dQ: new Uint8Array(64).fill(index),
    eG: new Uint8Array(64).fill(index),
    eQ: new Uint8Array(64).fill(index),
  };
}

function fakeRound2(index: number): OprfRound2Response {
  return {
    member: `5Member${index}`,
    index,
    zI: new Uint8Array(32).fill(index),
  };
}

describe('combineCommitteeSlotResponses (honest stub)', () => {
  it('throws OprfCombinationUnimplementedError with a clear, on-topic message', async () => {
    const round1 = [fakeRound1(1), fakeRound1(2), fakeRound1(3)];
    const round2 = [fakeRound2(1), fakeRound2(2), fakeRound2(3)];
    const groupPublicKey = new Uint8Array(32).fill(9);

    await expect(combineCommitteeSlotResponses(round1, round2, groupPublicKey)).rejects.toBeInstanceOf(
      OprfCombinationUnimplementedError,
    );
    await expect(combineCommitteeSlotResponses(round1, round2, groupPublicKey)).rejects.toThrow(
      /not implemented/i,
    );
  });

  it('throws regardless of input shape — this is a stub, not a validator', async () => {
    await expect(combineCommitteeSlotResponses([], [], new Uint8Array(0))).rejects.toBeInstanceOf(
      OprfCombinationUnimplementedError,
    );
  });
});

describe('module shape', () => {
  it('exports the documented types and function without crashing the type system', () => {
    // Compile-time check only: if these types didn't exist/match, this file wouldn't
    // type-check. No runtime assertion is meaningful here beyond "it imported".
    const proof: CombinedCommitteeProof | undefined = undefined;
    expect(proof).toBeUndefined();
    expect(typeof combineCommitteeSlotResponses).toBe('function');
    expect(typeof OprfCombinationUnimplementedError).toBe('function');
  });
});
