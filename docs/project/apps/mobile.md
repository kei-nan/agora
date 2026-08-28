# Mobile app

All TypeScript/JS logic is done. `android/` is a real, committed native project (added in
`0f15f52`, "feat(mobile): add Android NFC passport reader native module") — not a scaffold
missing native code. `ios/` genuinely does not exist yet (confirmed directly, no `mobile/ios/`
anywhere in the repo).

### Current status (see `docs/project/changelog/080.md` for the full verification)

- `cd mobile && npm install` then `npm test`: **21 suites / 300 tests, all passing** (re-verified
  2026-08-25 via `npx jest`) — grown well past the 4 suites / 77 tests changelog #080 originally
  verified (`sodParser`, `zkProving`, `proofEncoding`, `certificateTree` plus, since then,
  governance-screen fixes, chain-call argument tests, Keystore-backed signing coverage, and — most
  recently — recover-account, delegate-persona/backing-privacy, and IPFS content-fetching
  coverage).
- `mobile/android/` has real Gradle config (Gradle 8.6 via `gradle-wrapper.properties`,
  `compileSdkVersion`/`buildToolsVersion` 34, `ndkVersion 26.1.10909125` in `build.gradle`), a
  checked-in `app/debug.keystore`, generated app resources, and a hand-written native module:
  `app/src/main/java/com/agora/nfc/NfcPassportModule.kt` + `NfcPassportPackage.kt`.
- **No JDK or Android SDK is installed in this WSL2 environment** — `java`/`javac` not found,
  `ANDROID_HOME`/`ANDROID_SDK_ROOT` unset, no `adb`/`gradle`/`sdkmanager` on `PATH`, no SDK
  under `~/Android`, `~/Android/Sdk`, or the usual system locations. `cd mobile/android &&
  ./gradlew --version` fails immediately with `JAVA_HOME is not set and no 'java' command
  could be found`. This is the actual remaining blocker to a real `assembleDebug` — not a
  missing native project.

### To make it buildable (once JDK/SDK are installed — not done unattended; needs disk space
and license acceptance, so confirm with the user first)

Install, then build:
```bash
# JDK 17 (RN 0.74's documented requirement / what Gradle 8.6 + AGP expect)
# Android SDK command-line tools, then via sdkmanager:
sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0" "ndk;26.1.10909125"

cd mobile/android
./gradlew assembleDebug
```

Register the deep link scheme (not yet done in the checked-in `android/`):
- **Android**: add `<data android:scheme="democracychain" android:host="auth" />` to `AndroidManifest.xml` intent filter
- **iOS**: once an `ios/` project exists, add `democracychain` as a URL scheme in `Info.plist`

### Files

Chain reads:
- `src/chain/api.ts` — WsProvider + ApiPromise singleton
- `src/chain/identity.ts` — `registerCitizen`/`reverifyCitizen`/`migrateOprfScheme` (all take an
  outer ZKPassport `zk_proof`/variable-length `public_inputs` plus the OPRF identity-anchor
  material — `anchor`/`oprf_pk_hashes`, or `new_anchor`/`old_oprf_pk_hashes`/`new_oprf_pk_hashes`
  for migration — matching `pallet-identity`'s real call-index-0/6/7 signatures, not the old
  5-signal Rarimo `registerIdentity` shape), `isCitizen`, `getSigningKeypair`
- `src/chain/governance.ts` — `fetchProposals`, `fetchLaws`, `fetchPetitions`, `voteOnReferendum`, `signPetition`, `getDelegation`, `delegateVote`, `revokeDelegation`
- `src/chain/voting.ts` — MACI proposal submission, budget allocation
- `src/chain/constitution.ts` — petition submission and amendment
- `src/chain/courts.ts` — case filing, appeal, jury vote

Screens:
- `src/screens/HomeScreen.tsx` — citizen status, chain stats, quick nav
- `src/screens/ProposalsScreen.tsx` — referendum list with For/Against vote buttons
- `src/screens/LawsScreen.tsx` — active laws with tier + status chips
- `src/screens/PetitionScreen.tsx` — petition list with progress bar + sign button
- `src/screens/DelegateScreen.tsx` — per-topic delegation: set delegate, revoke, current status
- `src/screens/AuthScreen.tsx` — desktop QR deep-link handler (auto-activates on `democracychain://auth?...`)
- `src/screens/RegisterScreen.tsx` — does a real JMRTD BAC NFC read (via
  `src/native/nfcPassportReader`) and real SOD/DG1/DG15 parsing (`buildCircuitInputs` in
  `chain/sodParser.ts`); "stub" understates it — it only stops short at proof generation,
  throwing `NotImplementedError` once it reaches that step

`src/App.tsx`:
- Bottom tab navigator (Home / Proposals / Laws / Petitions / Delegate)
- Stack routes for Register + Auth (modal)
- `Linking` listener for `democracychain://auth?...` deep links → auto-navigates to AuthScreen

### Rarimo passport integration — HISTORICAL, SUPERSEDED (kept for record only)

**This section describes a plan that was dropped 2026-07-30** (CLAUDE.md / HANDOFF log #65) in
favor of ZKPassport (Noir/UltraHonk circuits). It no longer reflects the current architecture —
see the "Files" section above for the real ZKPassport/OPRF-anchor call shapes actually wired
into `identity.ts`. Kept here only as a record of real, verified research that predates the
migration:

`@rarimo/react-native-passport-reader` (previously referenced here) **does not exist** — verified
via npm/GitHub search, nothing under that name from Rarimo or anyone else. That reference was
wrong; don't install it or search for it again. The plan researched at the time was on-device
Groth16 proving via `@iden3/react-native-rapidsnark` + `@iden3/react-native-circom-witnesscalc`
against `passport-zk-circuits` directly, **not** `@rarimo/rarime-rn-sdk` (a real but wrong-fit
package — Expo-coupled, generates Noir proofs, submits straight to Rarimo's own EVM identity
contracts; doesn't feed our pallet at all). None of this circom/Groth16 tooling is what's
actually used now — ZKPassport's own Noir/UltraHonk SDK replaces it.

