import {
  _resetBackingStateForTests,
  clearBacking,
  getAllBackingSlots,
  getBackingSlotFor,
  isBackingDelegateLocally,
  nextFreeBackingSlot,
  recordBacking,
} from './backingState';

const DELEGATE_A = '5DelegateAddressA';
const DELEGATE_B = '5DelegateAddressB';

beforeEach(() => {
  _resetBackingStateForTests();
});

describe('recordBacking / getBackingSlotFor / isBackingDelegateLocally', () => {
  it('has nothing recorded initially', () => {
    expect(getBackingSlotFor(DELEGATE_A)).toBeNull();
    expect(isBackingDelegateLocally(DELEGATE_A)).toBe(false);
  });

  it('records and reports a backing', () => {
    recordBacking(DELEGATE_A, 3);
    expect(getBackingSlotFor(DELEGATE_A)).toBe(3);
    expect(isBackingDelegateLocally(DELEGATE_A)).toBe(true);
    // Unrelated delegate is unaffected.
    expect(isBackingDelegateLocally(DELEGATE_B)).toBe(false);
  });

  it('overwrites a prior slot for the same delegate', () => {
    recordBacking(DELEGATE_A, 0);
    recordBacking(DELEGATE_A, 1);
    expect(getBackingSlotFor(DELEGATE_A)).toBe(1);
  });
});

describe('clearBacking', () => {
  it('removes a recorded backing', () => {
    recordBacking(DELEGATE_A, 2);
    clearBacking(DELEGATE_A);
    expect(getBackingSlotFor(DELEGATE_A)).toBeNull();
    expect(isBackingDelegateLocally(DELEGATE_A)).toBe(false);
  });

  it('is a no-op for a delegate never backed', () => {
    expect(() => clearBacking(DELEGATE_A)).not.toThrow();
  });
});

describe('getAllBackingSlots', () => {
  it('lists every recorded (delegate, slotIndex) pair', () => {
    recordBacking(DELEGATE_A, 0);
    recordBacking(DELEGATE_B, 1);
    const all = getAllBackingSlots();
    expect(all).toHaveLength(2);
    expect(all).toEqual(expect.arrayContaining([[DELEGATE_A, 0], [DELEGATE_B, 1]]));
  });
});

describe('nextFreeBackingSlot', () => {
  it('returns 0 when nothing is backed yet', () => {
    expect(nextFreeBackingSlot(50)).toBe(0);
  });

  it('returns the lowest unused slot', () => {
    recordBacking(DELEGATE_A, 0);
    recordBacking(DELEGATE_B, 2);
    expect(nextFreeBackingSlot(50)).toBe(1);
  });

  it('throws once every slot up to the cap is in use', () => {
    recordBacking(DELEGATE_A, 0);
    recordBacking(DELEGATE_B, 1);
    expect(() => nextFreeBackingSlot(2)).toThrow(/all 2 backing slots/);
  });
});
