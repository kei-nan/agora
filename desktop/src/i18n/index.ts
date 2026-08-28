/**
 * i18n setup — Agora desktop app.
 *
 * PATTERN (read this before adding a new locale key or migrating a page):
 *  - One JSON namespace per page, named after the page (e.g. `laws` for
 *    LawsPage.tsx), living at `src/i18n/locales/<lng>/<namespace>.json`.
 *    Register any new namespace in the `resources` map below.
 *  - English-only for now — no other locale directories exist yet. The
 *    detector/init below is fully wired so adding a second language later
 *    is just: add `src/i18n/locales/<lng>/*.json` files, list them in
 *    `resources`, and push `<lng>` onto `supportedLngs`.
 *  - Within a page component: `const { t } = useTranslation('<namespace>');`
 *    then `t('key')` / `t('key', { placeholder })`. Keep the English copy
 *    in the JSON byte-for-byte identical to what the component used to
 *    hardcode — this pass is about wiring extraction, not rewording.
 *  - Only user-facing rendered copy is extracted. Strings that are purely
 *    internal — e.g. the free-text context blob each page hands to the AI
 *    agent panel via `setActiveItem(id, "Law: ...\nTier: ...")` — are
 *    deliberately left as plain template literals in the component; an LLM
 *    prompt isn't UI copy.
 *  - Plurals: use i18next's built-in `_one` / `_other` key suffixes (see
 *    `elections.json`'s `citizenCount_one`/`citizenCount_other`, or
 *    `legislature.json`'s `memberCount_one`/`memberCount_other`) and call
 *    `t('citizenCount', { count })` — don't hand-roll ternaries for "s"
 *    suffixes in new code.
 *
 * STILL TO MIGRATE (desktop/src/pages/): AntiCorruptionPage.tsx,
 * CourtsPage.tsx. ProposalsPage.tsx and AuthPage.tsx were excluded from
 * this pass because other agents were actively editing them concurrently —
 * migrate those next using the same pattern once free to touch them.
 */
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import enLaws from "./locales/en/laws.json";
import enTreasury from "./locales/en/treasury.json";
import enElections from "./locales/en/elections.json";
import enLegislature from "./locales/en/legislature.json";

export const defaultNS = "laws";

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: {
        laws: enLaws,
        treasury: enTreasury,
        elections: enElections,
        legislature: enLegislature,
      },
    },
    fallbackLng: "en",
    supportedLngs: ["en"],
    defaultNS,
    interpolation: { escapeValue: false }, // React already escapes.
  });

export default i18n;
