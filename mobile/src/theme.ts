/**
 * Shared color tokens for the mobile app's screens.
 *
 * Every one of these hex values was previously duplicated as a raw string
 * literal across most of `src/screens/*.tsx` (e.g. `#0f1117` appeared in all
 * 9 screens, `#6C63FF` in all of them) — a change to the brand color or the
 * dark-theme palette meant hunting down every occurrence individually, with
 * no guarantee two screens hadn't already drifted to slightly different
 * values for what was meant to be the same color. This file is the single
 * source of truth those literals now point back to.
 *
 * Deliberately just a flat color palette, not a full theming system (no
 * light-mode variant, no ThemeProvider/context) — this app has exactly one
 * dark theme today, and adding that machinery ahead of an actual need would
 * be speculative.
 */
export const colors = {
  /** Screen background. */
  bg: '#0f1117',
  /** Card/panel background, one step up from `bg`. */
  card: '#161b27',
  /** Default hairline border on cards, inputs, dividers. */
  border: '#1f2937',
  /** Brand accent — primary buttons, active states, links. */
  accent: '#6C63FF',

  /** Primary (near-white) text. */
  textPrimary: '#ffffff',
  /** Secondary text — labels, less prominent copy. */
  textSecondary: '#9ca3af',
  /** Muted text — captions, sub-labels. */
  textMuted: '#6b7280',
  /** Dim text — placeholders, least prominent copy. */
  textDim: '#4b5563',
  /** Faintest text/border tone, used sparingly (e.g. dashed borders). */
  textFaint: '#374151',

  /** Positive/success status (e.g. "Active", vote passed). */
  success: '#22c55e',
  /** Negative/danger status (e.g. errors, "Overturned"). */
  danger: '#ef4444',
  /** Warning/pending status (e.g. "Pending", term-limit warnings). */
  warning: '#f59e0b',

  /** Body copy on cards/modals — one step brighter than `textSecondary`. */
  textBody: '#d1d5db',
  /** Solid success background — buttons/indicators for a completed positive action ("step done", "vote for"). */
  successSolid: '#166534',
  /** Subtle success-tinted background for status chips (e.g. "Active" law/proposal, threshold reached). */
  successBg: '#052e16',
  /** Solid danger background — buttons for a destructive/negative action (revoke, vote against, remove backing). */
  dangerSolid: '#7f1d1d',
  /** Warning banner background. */
  warningBg: '#451a03',
  /** Warning banner border; also used for "this will change an existing choice" caution actions. */
  warningBorder: '#92400e',
  /** Warning banner text. */
  warningTextStrong: '#fcd34d',
} as const;
