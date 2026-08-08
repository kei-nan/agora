/**
 * Root component for the committee-duty app.
 *
 * Deliberately a single screen (`CommitteeDutyScreen`) — this app does one job (check
 * for pending OPRF duties, fulfill them) per changelog 082's separation rationale, so
 * it doesn't need the tab/stack navigation `mobile/src/App.tsx` has for the many
 * citizen-facing surfaces. No native `android/`/`ios/` project has been scaffolded for
 * this app yet (out of scope for this task — see the task's final report), so this
 * component isn't runnable on a device today; it exists so the orchestration layer
 * under `chain/` and `storage/` has a real caller to be exercised through eventually,
 * the same relationship `mobile/src/App.tsx` has to its own screens.
 */
import React from 'react';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import CommitteeDutyScreen from './screens/CommitteeDutyScreen';

export default function App() {
  return (
    <SafeAreaProvider>
      <CommitteeDutyScreen />
    </SafeAreaProvider>
  );
}
