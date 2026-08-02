import { Buffer } from 'buffer';
import { buildCircuitInputs } from '../sodParser';
import {
  TEST_PASSPORT_DG1_BASE64,
  TEST_PASSPORT_DG15_BASE64,
  TEST_PASSPORT_SOD_BASE64,
} from '../__fixtures__/testPassport';

describe('generated test passport fixture', () => {
  it('parses through buildCircuitInputs like a real passport SOD would', () => {
    const dg1 = new Uint8Array(Buffer.from(TEST_PASSPORT_DG1_BASE64, 'base64'));
    const dg15 = new Uint8Array(Buffer.from(TEST_PASSPORT_DG15_BASE64, 'base64'));
    const sod = new Uint8Array(Buffer.from(TEST_PASSPORT_SOD_BASE64, 'base64'));

    const parsed = buildCircuitInputs(dg1, dg15, sod);

    expect(parsed.variant.signature.kind).toBe('rsa');
    expect(parsed.variant.signedAttrsHash).toBe('sha256');
    expect(parsed.variant.dataGroupHash).toBe('sha256');
    expect(parsed.idData.dg1Size).toBe(dg1.length);
    expect(parsed.integrity.dg1HashOffset).toBeGreaterThanOrEqual(0);
    expect(parsed.integrity.dg15HashOffset).toBeNull();
    expect(parsed.activeAuthentication).toBeNull();
  });
});
