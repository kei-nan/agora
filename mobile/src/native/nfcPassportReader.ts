/**
 * Thin JS/TS bridge to the Android-only native `NfcPassportModule`
 * (`mobile/android/app/src/main/java/com/agora/nfc/NfcPassportModule.kt`),
 * which wraps JMRTD (`org.jmrtd:jmrtd`) + scuba-sc-android
 * (`net.sf.scuba:scuba-sc-android`) to perform a BAC-authenticated NFC read
 * of a biometric passport chip and return the RAW EF.DG1 / EF.DG15 / EF.SOD
 * data-group bytes — not parsed/summarized fields (name, DOB, etc.) — since
 * these bytes are direct witness inputs to the ZKPassport ZK circuit. See
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
  /**
   * EF.DG2's face image, already unwrapped from its ISO 19794-5 CBEFF header
   * by the native side (`NfcPassportModule.kt`) — not a ZK witness input like
   * the three fields above; consumed only by the face-match pipeline
   * (`../native/faceMatch.ts`). Empty if the passport had no parseable
   * FaceImageInfo.
   */
  dg2: Uint8Array;
  /** MIME type of `dg2`: `'image/jpeg'` or `'image/jp2'` per ICAO 9303 — `''` if `dg2` is empty. */
  dg2MimeType: string;
}

/** The native module's actual bridge shape: base64-encoded strings (React Native bridges can't cleanly pass raw binary). */
interface NfcPassportModuleNative {
  readPassport(
    documentNumber: string,
    dateOfBirth: string,
    dateOfExpiry: string,
  ): Promise<{ dg1: string; dg15: string; sod: string; dg2: string; dg2MimeType: string }>;
  /** Clears a pending `readPassport` call, rejecting it if one is in flight. No-ops if none is pending. */
  cancelPendingRead(): Promise<null>;
}

const NfcPassportModule = NativeModules.NfcPassportModule as NfcPassportModuleNative | undefined;

/** How long to wait for a passport tap before giving up. Generous — finding the chip's antenna position can take a few tries. */
export const DEFAULT_SCAN_TIMEOUT_MS = 60_000;

/**
 * Reads the raw DG1/DG15/SOD bytes off a biometric passport's NFC chip via
 * Basic Access Control (BAC), using the three MRZ fields as the BAC key
 * seed. The returned promise resolves once a passport chip is tapped to the
 * phone and the full read completes — there is no separate "wait for tag"
 * step exposed here (see `NfcPassportModule.readPassport`'s doc comment for
 * how tag delivery actually works on the native side).
 *
 * Rejects after `timeoutMs` (default {@link DEFAULT_SCAN_TIMEOUT_MS}) if no
 * tag is ever presented, calling `cancelPendingRead` on the native side so
 * the abandoned call doesn't permanently occupy the module's single pending
 * slot (`NfcPassportModule.kt`'s `readPassport` rejects any second call with
 * `NFC_BUSY` while one is pending) — without this, a passport that's never
 * tapped left every later scan attempt failing until the app was restarted.
 *
 * Android only — rejects immediately on other platforms or if the native
 * module isn't linked.
 */
export async function readPassport(
  mrz: PassportMrz,
  timeoutMs: number = DEFAULT_SCAN_TIMEOUT_MS,
): Promise<RawPassportData> {
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
  const nativeModule = NfcPassportModule;

  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    nativeModule.cancelPendingRead().catch(() => undefined);
  }, timeoutMs);

  try {
    const { dg1, dg15, sod, dg2, dg2MimeType } = await nativeModule.readPassport(
      mrz.documentNumber,
      mrz.dateOfBirth,
      mrz.dateOfExpiry,
    );

    return {
      dg1: new Uint8Array(Buffer.from(dg1, 'base64')),
      dg15: new Uint8Array(Buffer.from(dg15, 'base64')),
      sod: new Uint8Array(Buffer.from(sod, 'base64')),
      dg2: new Uint8Array(Buffer.from(dg2, 'base64')),
      dg2MimeType,
    };
  } catch (e) {
    if (timedOut) {
      throw new Error(`readPassport: no passport was tapped within ${timeoutMs}ms — timed out`);
    }
    throw e;
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Cancels a passport scan that's currently waiting for a tag — e.g. the user
 * navigated away from the scanning screen. Safe to call even if no scan is
 * in progress. Exposed separately from `readPassport`'s own timeout so a
 * screen's unmount/cleanup can call this without waiting for the timeout.
 */
export async function cancelPendingScan(): Promise<void> {
  if (Platform.OS !== 'android' || !NfcPassportModule) return;
  await NfcPassportModule.cancelPendingRead().catch(() => undefined);
}
