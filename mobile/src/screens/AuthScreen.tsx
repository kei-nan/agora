/**
 * AuthScreen — desktop QR deep-link handler.
 *
 * Flow:
 *  1. User's phone camera (system) scans QR on desktop → launches app via deep link:
 *       democracychain://auth?challenge=<uuid>&callback=http://127.0.0.1:<port>/auth
 *  2. App.tsx handles the Linking event, navigates here with { deepLink }.
 *  3. This screen shows the caller what it's about to sign and waits for an explicit tap.
 *  4. On confirmation, signs the challenge with the hardware-backed key and posts
 *     { challenge, nullifierHash, signature } to the callback URL.
 *  5. Desktop poll (`auth_poll_session`) picks up the session and verifies nullifier on-chain.
 *
 *  Fallback: if launched standalone (no deep link), shows a "Scan QR" placeholder.
 *
 * # Why this doesn't auto-sign on deep-link open
 *
 * The `democracychain://` scheme is registered `android:exported="true"` with
 * `BROWSABLE` (AndroidManifest.xml), so *any* link a user taps — a webpage, a
 * chat message, another app — can open this screen with attacker-chosen
 * `challenge`/`callback` values, not just a real desktop QR code. Two checks
 * gate signing as a result:
 *
 *  - `callback` must resolve to loopback (127.0.0.1/localhost), matching what
 *    the flow actually is (a local desktop app on the same machine the QR was
 *    displayed on). Anything else is refused before the keypair is ever
 *    touched — otherwise this endpoint is a generic "sign whatever bytes this
 *    link contains and mail the signature to whatever server this link
 *    names" oracle for the user's real chain key.
 *  - The raw challenge is shown and requires an explicit tap to sign, rather
 *    than firing automatically from the `useEffect` on deep-link open, so a
 *    user has a chance to notice something's wrong before their key signs it.
 */

import React, { useEffect, useState } from "react";
import {
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
  ActivityIndicator,
  Platform,
} from "react-native";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { getSigningKeypair } from "../chain/identity";
import { RootStackParamList } from "../App";
import { colors } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "Auth">;

type AuthStatus = "idle" | "awaitingConfirmation" | "signing" | "posting" | "done" | "error";

interface ParsedAuthRequest {
  challenge: string;
  callback: string;
}

/**
 * Only `http(s)://127.0.0.1[:port]` and `http(s)://localhost[:port]` are
 * accepted — this flow is a desktop app on the same machine the QR code was
 * displayed on, never a remote server. Rejecting everything else here is
 * what stops an arbitrary link from turning this screen into a signing
 * oracle that exfiltrates to an attacker's server (see module doc comment).
 */
function isLoopbackCallback(callback: string): boolean {
  let url: URL;
  try {
    url = new URL(callback);
  } catch {
    return false;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return false;
  // WHATWG URL serializes an IPv6 literal host with brackets ("[::1]"), not
  // bare "::1" — the unbracketed form here previously never matched anything
  // real, silently rejecting a legitimate IPv6-loopback callback (fails
  // safe, not exploitable, but still wrong).
  return url.hostname === "127.0.0.1" || url.hostname === "localhost" || url.hostname === "[::1]";
}

export default function AuthScreen({ route, navigation }: Props) {
  const [status, setStatus] = useState<AuthStatus>("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [pendingRequest, setPendingRequest] = useState<ParsedAuthRequest | null>(null);

  // Parse (but do not act on) a deep link as soon as one arrives — signing
  // happens only after the user taps Confirm below.
  useEffect(() => {
    if (route.params?.deepLink) {
      parseDeepLink(route.params.deepLink);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [route.params?.deepLink]);

  function parseDeepLink(deepLink: string) {
    try {
      const url = new URL(deepLink);
      const challenge = url.searchParams.get("challenge");
      const callback = url.searchParams.get("callback");
      if (!challenge || !callback) {
        throw new Error("Invalid QR code: missing challenge or callback");
      }
      if (!isLoopbackCallback(callback)) {
        throw new Error(
          `Refusing to sign: this link asks to send your signature to "${callback}", ` +
            "which isn't the desktop app running on this machine. This link may not be genuine.",
        );
      }
      setPendingRequest({ challenge, callback });
      setStatus("awaitingConfirmation");
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setErrorMsg(msg);
      setStatus("error");
    }
  }

  async function confirmAndSign() {
    if (!pendingRequest) return;
    const { challenge, callback } = pendingRequest;
    setStatus("signing");
    try {
      // Sign the challenge with the hardware-backed signing key.
      const { keypair, nullifierHash } = await getSigningKeypair();
      const sig = keypair.sign(Buffer.from(challenge, "utf8"));
      const signature = Buffer.from(sig).toString("hex");

      setStatus("posting");
      const response = await fetch(callback, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ challenge, nullifierHash, signature }),
      });

      if (!response.ok) {
        const body = await response.text();
        throw new Error(`Callback server returned ${response.status}: ${body}`);
      }

      setStatus("done");
      setTimeout(() => navigation.goBack(), 1500);
    } catch (e: unknown) {
      const msg = e instanceof Error ? e.message : String(e);
      setErrorMsg(msg);
      setStatus("error");
    }
  }

  return (
    <View style={styles.container}>
      <Text style={styles.title}>Desktop Sign-In</Text>
      <Text style={styles.subtitle}>
        Use your phone's camera to scan the QR code shown on the Agora desktop app. Your signing
        key never leaves the phone.
      </Text>

      {status === "idle" && (
        <View style={styles.flowBox}>
          <FlowStep num="1" text="Open your phone's camera app" />
          <FlowStep num="2" text="Point it at the QR code on the Agora desktop" />
          <FlowStep num="3" text="Tap the notification — this screen will handle the rest" />
          <Text style={styles.hint}>
            The QR code expires in 5 minutes. Refresh it on the desktop if it times out.
          </Text>
        </View>
      )}

      {status === "awaitingConfirmation" && pendingRequest && (
        <View style={styles.flowBox}>
          <Text style={styles.confirmExplainer}>
            Confirm this matches the code shown on your desktop screen to finish signing in.
          </Text>
          <Text style={styles.confirmLabel}>Code shown on desktop:</Text>
          <Text style={styles.confirmChallenge} selectable>
            {pendingRequest.challenge}
          </Text>
          <Text style={styles.confirmLabel}>Signing in to the desktop app at:</Text>
          <Text style={styles.confirmChallenge} selectable>
            {pendingRequest.callback}
          </Text>
          <TouchableOpacity
            style={styles.button}
            onPress={confirmAndSign}
            accessibilityRole="button"
            accessibilityLabel="Confirm and sign"
          >
            <Text style={styles.buttonText}>Confirm & Sign</Text>
          </TouchableOpacity>
          <TouchableOpacity
            style={styles.secondaryButton}
            onPress={() => {
              setPendingRequest(null);
              setStatus("idle");
            }}
            accessibilityRole="button"
            accessibilityLabel="Cancel"
          >
            <Text style={styles.secondaryButtonText}>Cancel</Text>
          </TouchableOpacity>
        </View>
      )}

      {(status === "signing" || status === "posting") && (
        <View style={styles.loadingBox}>
          <ActivityIndicator size="large" color={colors.accent} />
          <Text style={styles.loadingText}>
            {status === "signing"
              ? "Signing challenge…"
              : "Sending to desktop…"}
          </Text>
        </View>
      )}

      {status === "done" && (
        <View style={styles.successBox}>
          <Text style={styles.successIcon}>✓</Text>
          <Text style={styles.successText}>Desktop authenticated!</Text>
        </View>
      )}

      {status === "error" && (
        <View style={styles.errorBox}>
          <Text style={styles.errorTitle}>Authentication failed</Text>
          <Text style={styles.errorMsg}>{errorMsg}</Text>
          <TouchableOpacity
            style={styles.button}
            onPress={() => {
              setPendingRequest(null);
              setStatus("idle");
            }}
            accessibilityRole="button"
            accessibilityLabel="Try again"
          >
            <Text style={styles.buttonText}>Try again</Text>
          </TouchableOpacity>
        </View>
      )}
    </View>
  );
}

function FlowStep({ num, text }: { num: string; text: string }) {
  return (
    <View style={styles.flowStep}>
      <View style={styles.flowNum}>
        <Text style={styles.flowNumText}>{num}</Text>
      </View>
      <Text style={styles.flowStepText}>{text}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: colors.bg,
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
  },
  title: {
    fontSize: 26,
    fontWeight: "700",
    color: colors.textPrimary,
    marginBottom: 12,
  },
  subtitle: {
    fontSize: 15,
    color: colors.textSecondary,
    textAlign: "center",
    marginBottom: 40,
    lineHeight: 22,
  },
  button: {
    backgroundColor: colors.accent,
    paddingVertical: 16,
    paddingHorizontal: 40,
    borderRadius: 12,
  },
  buttonText: {
    color: colors.textPrimary,
    fontSize: 16,
    fontWeight: "600",
  },
  secondaryButton: {
    marginTop: 12,
    paddingVertical: 12,
    alignItems: "center",
  },
  secondaryButtonText: {
    color: colors.textSecondary,
    fontSize: 14,
    fontWeight: "600",
  },
  confirmExplainer: {
    color: colors.textBody,
    fontSize: 14,
    lineHeight: 20,
    marginBottom: 4,
  },
  confirmLabel: {
    color: colors.textSecondary,
    fontSize: 13,
    marginTop: 4,
  },
  confirmChallenge: {
    color: colors.textPrimary,
    fontSize: 14,
    fontWeight: "600",
    fontFamily: Platform.OS === "android" ? "monospace" : "Menlo",
    backgroundColor: colors.bg,
    borderRadius: 8,
    padding: 10,
    marginBottom: 8,
  },
  loadingBox: {
    alignItems: "center",
    gap: 16,
  },
  loadingText: {
    color: colors.textSecondary,
    fontSize: 15,
  },
  successBox: {
    alignItems: "center",
    gap: 12,
  },
  successIcon: {
    fontSize: 56,
    color: colors.success,
  },
  successText: {
    fontSize: 18,
    fontWeight: "600",
    color: colors.success,
  },
  errorBox: {
    alignItems: "center",
    gap: 12,
  },
  errorTitle: {
    fontSize: 18,
    fontWeight: "600",
    color: colors.danger,
  },
  errorMsg: {
    fontSize: 13,
    color: colors.textSecondary,
    textAlign: "center",
  },
  hint: {
    fontSize: 13,
    color: colors.textMuted,
    textAlign: "center",
    marginTop: 16,
    lineHeight: 20,
  },
  flowBox: {
    width: "100%",
    backgroundColor: colors.card,
    borderRadius: 16,
    padding: 20,
    borderWidth: 1,
    borderColor: colors.border,
    gap: 16,
  },
  flowStep: {
    flexDirection: "row",
    alignItems: "center",
    gap: 14,
  },
  flowNum: {
    width: 32,
    height: 32,
    borderRadius: 16,
    backgroundColor: colors.accent,
    alignItems: "center",
    justifyContent: "center",
  },
  flowNumText: {
    color: colors.textPrimary,
    fontWeight: "700",
    fontSize: 14,
  },
  flowStepText: {
    flex: 1,
    fontSize: 14,
    color: colors.textBody,
    lineHeight: 20,
  },
});
