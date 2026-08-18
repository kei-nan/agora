/**
 * Thin wrapper around the native `FaceCameraView` component
 * (`FaceCameraViewManager.kt`'s `SimpleViewManager<PreviewView>` — a live
 * front-camera preview for the face-match/liveness capture flow, see
 * `../screens/RegisterScreen.tsx`). `requireNativeComponent` is the
 * classic-bridge way to surface a native view; matches this app's
 * `newArchEnabled=false` setting the same way `./faceMatch.ts`'s native
 * modules do (see `FaceCameraViewManager.kt`'s doc comment).
 *
 * **Android only** — `requireNativeComponent('FaceCameraView')` throws at
 * render time on iOS/an environment with no such native view registered;
 * callers must gate rendering this on `isFaceMatchAvailable()`
 * (`./faceMatch.ts`) the same way every other native-module call in this
 * app is gated.
 *
 * Not runtime-tested: no Android SDK, emulator, or physical device is
 * available in this development environment.
 */
import React from 'react';
import { requireNativeComponent, ViewProps } from 'react-native';

const NativeFaceCameraView = requireNativeComponent<ViewProps>('FaceCameraView');

export default function FaceCameraView(props: ViewProps) {
  return <NativeFaceCameraView {...props} />;
}
