/**
 * Themed replacement for `Alert.alert`. The native OS alert doesn't respect
 * the app's dark theme (stock white/light dialog on both platforms) and its
 * button-array API can't distinguish "informational" from "confirm a
 * destructive action" styling. This renders the same in-app dark card for
 * both cases, reusing the modal look RegisterScreen's registration-result
 * dialog already established.
 */
import React, { createContext, ReactNode, useCallback, useContext, useState } from 'react';
import { Modal, StyleSheet, Text, TouchableOpacity, View } from 'react-native';
import { colors } from '../theme';

type ModalState =
  | { kind: 'info'; title: string; message: string; devDetails?: string }
  | {
      kind: 'confirm';
      title: string;
      message: string;
      confirmLabel: string;
      destructive?: boolean;
      onConfirm: () => void;
    };

interface ConfirmOptions {
  title: string;
  message: string;
  confirmLabel?: string;
  destructive?: boolean;
  onConfirm: () => void;
}

interface AppModalContextValue {
  /** Friendly info dialog. Pass `devDetails` to show raw detail in dev builds only. */
  showInfo: (title: string, message: string, devDetails?: string) => void;
  /**
   * Friendly error dialog for a caught exception. `message` overrides the
   * generic default; the raw `error` is only ever shown in `__DEV__`.
   */
  showError: (title: string, error: unknown, message?: string) => void;
  /** Cancel/confirm dialog, e.g. before a destructive or high-stakes action. */
  showConfirm: (options: ConfirmOptions) => void;
}

const AppModalContext = createContext<AppModalContextValue | null>(null);

function errorDetails(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function AppModalProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<ModalState | null>(null);
  const close = useCallback(() => setState(null), []);

  const showInfo = useCallback((title: string, message: string, devDetails?: string) => {
    setState({ kind: 'info', title, message, devDetails });
  }, []);

  const showError = useCallback((title: string, error: unknown, message?: string) => {
    setState({
      kind: 'info',
      title,
      message: message ?? 'Something went wrong talking to the chain. Please try again.',
      devDetails: __DEV__ ? errorDetails(error) : undefined,
    });
  }, []);

  const showConfirm = useCallback((options: ConfirmOptions) => {
    setState({
      kind: 'confirm',
      title: options.title,
      message: options.message,
      confirmLabel: options.confirmLabel ?? 'Confirm',
      destructive: options.destructive,
      onConfirm: options.onConfirm,
    });
  }, []);

  return (
    <AppModalContext.Provider value={{ showInfo, showError, showConfirm }}>
      {children}
      <Modal visible={state !== null} transparent animationType="fade" onRequestClose={close}>
        <View style={s.backdrop}>
          <View style={s.card}>
            <Text style={s.title}>{state?.title}</Text>
            <Text style={s.message}>{state?.message}</Text>

            {state?.kind === 'info' && state.devDetails && (
              <>
                <View style={s.divider} />
                <Text style={s.devLabel}>DEV DETAILS</Text>
                <Text style={s.devText}>{state.devDetails}</Text>
              </>
            )}

            {state?.kind === 'confirm' ? (
              <View style={s.btnRow}>
                <TouchableOpacity
                  style={[s.btn, s.btnSecondary]}
                  onPress={close}
                  accessibilityRole="button"
                  accessibilityLabel="Cancel"
                >
                  <Text style={s.btnSecondaryText}>Cancel</Text>
                </TouchableOpacity>
                <TouchableOpacity
                  style={[s.btn, s.btnFlex, state.destructive ? s.btnDanger : s.btnPrimary]}
                  onPress={() => {
                    const { onConfirm } = state;
                    close();
                    onConfirm();
                  }}
                  accessibilityRole="button"
                  accessibilityLabel={state.confirmLabel}
                >
                  <Text style={s.btnText}>{state.confirmLabel}</Text>
                </TouchableOpacity>
              </View>
            ) : (
              <TouchableOpacity
                style={[s.btn, s.btnPrimary, s.btnFull]}
                onPress={close}
                accessibilityRole="button"
                accessibilityLabel="OK"
              >
                <Text style={s.btnText}>OK</Text>
              </TouchableOpacity>
            )}
          </View>
        </View>
      </Modal>
    </AppModalContext.Provider>
  );
}

export function useAppModal(): AppModalContextValue {
  const ctx = useContext(AppModalContext);
  if (!ctx) throw new Error('useAppModal() must be called within an AppModalProvider');
  return ctx;
}

const s = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.65)',
    justifyContent: 'center',
    alignItems: 'center',
    padding: 24,
  },
  card: {
    width: '100%',
    maxWidth: 400,
    backgroundColor: colors.card,
    borderRadius: 16,
    padding: 24,
    borderWidth: 1,
    borderColor: colors.border,
  },
  title: { fontSize: 18, fontWeight: '700', color: colors.textPrimary, marginBottom: 10 },
  message: { fontSize: 14, color: colors.textBody, lineHeight: 20 },
  divider: { height: 1, backgroundColor: colors.border, marginVertical: 16 },
  devLabel: { fontSize: 10, fontWeight: '700', color: colors.textMuted, letterSpacing: 0.8, marginBottom: 6 },
  devText: { fontSize: 12, color: colors.textMuted, lineHeight: 17 },
  btnRow: { flexDirection: 'row', gap: 10, marginTop: 20 },
  btn: { paddingVertical: 12, borderRadius: 10, alignItems: 'center' },
  btnFull: { marginTop: 20 },
  btnFlex: { flex: 1 },
  btnPrimary: { backgroundColor: colors.accent },
  btnDanger: { backgroundColor: colors.dangerSolid },
  btnSecondary: { flex: 1, backgroundColor: colors.border },
  btnText: { color: colors.textPrimary, fontWeight: '700', fontSize: 15 },
  btnSecondaryText: { color: colors.textSecondary, fontWeight: '600', fontSize: 15 },
});
