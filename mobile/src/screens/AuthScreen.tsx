/**
 * AuthScreen — desktop QR deep-link handler.
 *
 * Flow:
 *  1. User's phone camera (system) scans QR on desktop → launches app via deep link:
 *       democracychain://auth?challenge=<uuid>&callback=http://127.0.0.1:<port>/auth
 *  2. App.tsx handles the Linking event, navigates here with { deepLink }.
 *  3. This screen signs the challenge with the hardware-backed key.
 *  4. Posts { challenge, nullifierHash, signature, expiresAt } to callback URL.
 *  5. Desktop poll (`auth_poll_session`) picks up the session and verifies nullifier on-chain.
 *
 *  Fallback: if launched standalone (no deep link), shows a "Scan QR" placeholder.
 */

import React, { useEffect, useState } from "react";
import {
  StyleSheet,
  Text,
  TouchableOpacity,
  View,
  ActivityIndicator,
} from "react-native";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { getSigningKeypair } from "../chain/identity";
import { RootStackParamList } from "../App";

type Props = NativeStackScreenProps<RootStackParamList, "Auth">;

type AuthStatus = "idle" | "signing" | "posting" | "done" | "error";

export default function AuthScreen({ route, navigation }: Props) {
  const [status, setStatus] = useState<AuthStatus>("idle");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  // Auto-start auth if opened via deep link from the system camera
  useEffect(() => {
    if (route.params?.deepLink) {
      handleQrScanned(route.params.deepLink);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [route.params?.deepLink]);

  async function handleQrScanned(deepLink: string) {
    setStatus("signing");
    try {
      const url = new URL(deepLink);
      const challenge = url.searchParams.get("challenge");
      const callback = url.searchParams.get("callback");
      if (!challenge || !callback) {
        throw new Error("Invalid QR code: missing challenge or callback");
      }

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

      {(status === "signing" || status === "posting") && (
        <View style={styles.loadingBox}>
          <ActivityIndicator size="large" color="#6C63FF" />
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
          <TouchableOpacity style={styles.button} onPress={() => setStatus("idle")}>
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
    backgroundColor: "#0f1117",
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
  },
  title: {
    fontSize: 26,
    fontWeight: "700",
    color: "#ffffff",
    marginBottom: 12,
  },
  subtitle: {
    fontSize: 15,
    color: "#9ca3af",
    textAlign: "center",
    marginBottom: 40,
    lineHeight: 22,
  },
  button: {
    backgroundColor: "#6C63FF",
    paddingVertical: 16,
    paddingHorizontal: 40,
    borderRadius: 12,
  },
  buttonText: {
    color: "#ffffff",
    fontSize: 16,
    fontWeight: "600",
  },
  loadingBox: {
    alignItems: "center",
    gap: 16,
  },
  loadingText: {
    color: "#9ca3af",
    fontSize: 15,
  },
  successBox: {
    alignItems: "center",
    gap: 12,
  },
  successIcon: {
    fontSize: 56,
    color: "#22c55e",
  },
  successText: {
    fontSize: 18,
    fontWeight: "600",
    color: "#22c55e",
  },
  errorBox: {
    alignItems: "center",
    gap: 12,
  },
  errorTitle: {
    fontSize: 18,
    fontWeight: "600",
    color: "#ef4444",
  },
  errorMsg: {
    fontSize: 13,
    color: "#9ca3af",
    textAlign: "center",
  },
  hint: {
    fontSize: 13,
    color: "#6b7280",
    textAlign: "center",
    marginTop: 16,
    lineHeight: 20,
  },
  flowBox: {
    width: "100%",
    backgroundColor: "#161b27",
    borderRadius: 16,
    padding: 20,
    borderWidth: 1,
    borderColor: "#1f2937",
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
    backgroundColor: "#6C63FF",
    alignItems: "center",
    justifyContent: "center",
  },
  flowNumText: {
    color: "#ffffff",
    fontWeight: "700",
    fontSize: 14,
  },
  flowStepText: {
    flex: 1,
    fontSize: 14,
    color: "#d1d5db",
    lineHeight: 20,
  },
});
