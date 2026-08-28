/**
 * i18n setup — Agora mobile app.
 *
 * PATTERN (read this before adding a new locale key or migrating a screen):
 *  - One JSON namespace per screen, named after the screen (e.g. `laws` for
 *    LawsScreen.tsx), living at `src/i18n/locales/<lng>/<namespace>.json`.
 *    Register any new namespace in the `resources` map below.
 *  - English-only for now — no other locale directories exist yet. The
 *    detector/init below is fully wired so adding a second language later
 *    is just: add `src/i18n/locales/<lng>/*.json` files, list them in
 *    `resources`, and push `<lng>` onto `SUPPORTED_LNGS`.
 *  - Within a screen component: `const { t } = useTranslation('<namespace>');`
 *    then `t('key')` / `t('key', { placeholder })`. Keep the English copy in
 *    the JSON byte-for-byte identical to what the component used to
 *    hardcode — this pass is about wiring extraction, not rewording.
 *  - Only user-facing rendered copy is extracted. Data-layer values that
 *    happen to be English words (e.g. a law's on-chain `status` of
 *    "Active"/"Paused"/"Repealed") are left untranslated for now — they're
 *    chain data flowing through a typed union, not a JSX string literal in
 *    the component, and localizing them means mapping enum values, a
 *    separate follow-up.
 *  - Plurals: use i18next's built-in `_one` / `_other` key suffixes and
 *    call `t('key', { count })` — don't hand-roll ternaries for "s"
 *    suffixes in new code (see desktop's i18n/index.ts for a worked plural
 *    example; none of the three screens migrated here happened to need
 *    one).
 *
 * Language detector: no all-in-one "i18next + React Native" detector
 * package is current/maintained as of this writing — the historically
 * obvious name, `i18next-react-native-language-detector`, still pins a
 * peer dependency on i18next ^3.0.0 against this app's i18next ^26, so it
 * was skipped. `react-native-localize` (actively maintained, autolinked
 * like this app's other native modules — see react-native-fs,
 * react-native-get-random-values) is the current standard building block;
 * it's wired below through i18next's own documented custom
 * languageDetector-module interface rather than a wrapper package. Since
 * SUPPORTED_LNGS is just `['en']` today this has no visible effect yet,
 * but it means adding a second language later is config-only.
 *
 * STILL TO MIGRATE (mobile/src/screens/): CasesScreen.tsx,
 * DelegateDetailScreen.tsx, DelegateScreen.tsx, RegisterDelegateScreen.tsx,
 * ProposalsScreen.tsx. RegisterScreen.tsx, RecoverAccountScreen.tsx,
 * AuthScreen.tsx, VoteScreen.tsx, RegistrationStatusScreen.tsx, and
 * HomeScreen.tsx were excluded from this pass because other agents were
 * actively editing them concurrently — migrate those next using the same
 * pattern once free to touch them.
 */
import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import * as RNLocalize from 'react-native-localize';
import type { LanguageDetectorModule } from 'i18next';

import enLaws from './locales/en/laws.json';
import enPetitions from './locales/en/petitions.json';
import enFileCase from './locales/en/fileCase.json';

export const defaultNS = 'laws';

const SUPPORTED_LNGS = ['en'];

const languageDetector: LanguageDetectorModule = {
  type: 'languageDetector',
  init: () => {},
  detect: () => RNLocalize.findBestLanguageTag(SUPPORTED_LNGS)?.languageTag ?? 'en',
  cacheUserLanguage: () => {},
};

i18n
  .use(languageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: {
        laws: enLaws,
        petitions: enPetitions,
        fileCase: enFileCase,
      },
    },
    fallbackLng: 'en',
    supportedLngs: SUPPORTED_LNGS,
    defaultNS,
    interpolation: { escapeValue: false }, // React already escapes.
  });

export default i18n;
