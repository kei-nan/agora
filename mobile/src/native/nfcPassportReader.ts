/**
 * Thin JS/TS bridge to the Android-only native `NfcPassportModule`
 * (`mobile/android/app/src/main/java/com/agora/nfc/NfcPassportModule.kt`),
 * which wraps JMRTD (`org.jmrtd:jmrtd`) + scuba-sc-android
 * (`net.sf.scuba:scuba-sc-android`) to perform a BAC-authenticated NFC read
 * of a biometric passport chip and return the RAW EF.DG1 / EF.DG15 / EF.SOD
 * data-group bytes — not parsed/summarized fields (name, DOB, etc.) — since
 * these bytes are direct witness inputs to the Rarimo ZK circuit. See
 * `../chain/zkProving.ts` / `../chain/proofEncoding.ts` for the on-device
 * proving half this feeds into (this module is strictly upstream of those —
 * it does not touch them, and they don't touch this).
 *
 * **Android only.** iOS needs an equivalent Swift native module wrapping
 * AndyQ/NFCPassportReader (MIT, confirmed real/current — see HANDOFF.md
 * item 8 / log #57) — not built here, since no `ios/` native project exists
 * yet in this repo; standing one up is a separate, larger undertaking.
 *
 * Not runtime-tested: no Android SDK, emulator, or physical device is
 * available in this development environment. Every native API call this
 * bridges to was cross-checked against JMRTD/scuba's actual source (see
 * HANDOFF.md log #57 and the module doc comment in `NfcPassportModule.kt`),
 * but only `tsc --noEmit` has been verified for this file.
 */
import { NativeModules, Platform } from 'react-native';
import { Buffer } from 'buffer';

/** The three MRZ (machine-readable zone) fields needed to derive the BAC key, per ICAO Doc 9303. Dates must be `YYMMDD`. */
export interface PassportMrz {
  documentNumber: string;
  dateOfBirth: string;
  dateOfExpiry: string;
}

/** Raw data-group bytes read from the passport chip — inputs to the ZK circuit witness, not parsed/human-readable fields. */
export interface RawPassportData {
  /** EF.DG1 — the MRZ, encoded per ICAO Doc 9303 Part 10. */
  dg1: Uint8Array;
  /** EF.DG15 — the Active Authentication public key. */
  dg15: Uint8Array;
  /** EF.SOD — the Document Security Object (signed hashes of all data groups + the issuing country's certificate chain). */
  sod: Uint8Array;
}

/** The native module's actual bridge shape: base64-encoded strings (React Native bridges can't cleanly pass raw binary). */
interface NfcPassportModuleNative {
  readPassport(
    documentNumber: string,
    dateOfBirth: string,
    dateOfExpiry: string,
  ): Promise<{ dg1: string; dg15: string; sod: string }>;
}

const NfcPassportModule = NativeModules.NfcPassportModule as NfcPassportModuleNative | undefined;

/**
 * Reads the raw DG1/DG15/SOD bytes off a biometric passport's NFC chip via
 * Basic Access Control (BAC), using the three MRZ fields as the BAC key
 * seed. The returned promise resolves once a passport chip is tapped to the
 * phone and the full read completes — there is no separate "wait for tag"
 * step exposed here (see `NfcPassportModule.readPassport`'s doc comment for
 * how tag delivery actually works on the native side).
 *
 * Android only — rejects immediately on other platforms or if the native
 * module isn't linked.
 */
export async function readPassport(mrz: PassportMrz): Promise<RawPassportData> {
  if (Platform.OS !== 'android') {
    throw new Error(
      `readPassport: NFC passport reading is only implemented for Android in this codebase (got Platform.OS=${Platform.OS}). ` +
        'iOS needs a native Swift module wrapping AndyQ/NFCPassportReader — not yet built, see HANDOFF.md item 8.',
    );
  }
  if (!NfcPassportModule) {
    throw new Error(
      'readPassport: NativeModules.NfcPassportModule is not available. Is the native module linked? ' +
        '(MainApplication.kt should register NfcPassportPackage.)',
    );
  }

  const { dg1, dg15, sod } = await NfcPassportModule.readPassport(
    mrz.documentNumber,
    mrz.dateOfBirth,
    mrz.dateOfExpiry,
  );

  return {
    dg1: new Uint8Array(Buffer.from(dg1, 'base64')),
    dg15: new Uint8Array(Buffer.from(dg15, 'base64')),
    sod: new Uint8Array(Buffer.from(sod, 'base64')),
  };
}
