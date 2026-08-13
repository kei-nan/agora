# "Just Use World ID" — Considered and Rejected

*Addendum, 2026-08-11, written directly (not via subagent) in response to a follow-up question after
the main 5-document research round. World ID appeared only as one comparison row in
[04-alternative-crypto-primitives.md](04-alternative-crypto-primitives.md); this note evaluates it
directly as a candidate to depend on, with current (2026) facts checked by web search rather than
relying on the earlier pass.*

## The three ways "use World ID" could be read, and why each fails

**1. Replace Agora's identity stack with World ID outright.** World ID has two verification paths as
of 2026:

- **Orb** (iris-scan hardware) — gives genuine biometric uniqueness independent of any document, but
  requires physical presence at an Orb, which doesn't fit a passport-NFC architecture and isn't
  universally deployed.
- **World ID Credentials** (new since the earlier research pass) — an NFC-passport/national-ID path
  that doesn't require an Orb, live for passports from Argentina, Chile, Colombia, Costa Rica, Japan,
  Malaysia, Mexico, Panama, South Korea, Taiwan, the UK, and the US ([World ID Credentials
  announcement](https://world.org/blog/announcements/new-world-id-passport-credential-launches-access-wld-tokens)).

The non-Orb path has the exact weakness already flagged in doc 04's comparison table: it uses the
document number as the uniqueness signal, and World's own documentation concedes someone can report a
document lost, obtain a new number, and re-verify as a new person. It does not solve
renewal-stability any better than doing nothing — it is architecturally close to what Agora already
builds with ZKPassport, minus an answer to the one genuinely hard part.

**2. Borrow just their OPRF/nullifier network, keep Agora's own passport verification.** Already
effectively ruled out by doc 04's own findings: World ID 4.0's distributed OPRF network is
Shamir-shared **per relying party** and scoped to World's own app ecosystem — it is not a general
service a third-party chain can query. Consuming it would mean becoming dependent on World
Foundation's governance for who counts as a valid registrant, which is the same shape as the TACEO
live-network rejection already on record (see
[02-existing-threshold-networks.md](02-existing-threshold-networks.md)), just with a different vendor.

**3. Accept it as one optional registration path for citizens who already have a World ID.** Doesn't
reduce Agora's own build burden (the passport path is still needed for everyone else), and any citizen
taking this path inherits the renewal-stability weakness from (1).

## The decisive new finding: World ID's operator has an active pattern of government bans

Checked by web search, not assumed. World ID is run by **Tools for Humanity**, a private company
co-founded by Sam Altman and Alex Blania. In the twelve months before this note:

- **October 2025** — the Philippines' National Privacy Commission ordered Tools for Humanity to
  **halt operations immediately** over consent and exploitation-of-vulnerable-populations concerns.
- **November 2025** — Thai authorities **shut down** World's biometric data collection and demanded
  data deletion.
- **Kenya banned** World's operations over privacy and financial concerns.
- **South Korea fined** the company roughly **$830,000** for privacy-law violations.

([Rest of World coverage](https://restofworld.org/2026/sam-altman-worldcoin-zoom-tinder-partnerships/);
[Biometric Update](https://www.biometricupdate.com/202606/world-shifts-from-crypto-identity-experiment-to-enterprise-proof-of-humanity))
The company has also pivoted its business model toward enterprise proof-of-humanity fee revenue and
reportedly conducted layoffs amid weak revenue in 2026
([TechCrunch](https://techcrunch.com/2026/06/08/as-openai-files-for-ipo-sam-altmans-eye-scanning-company-is-doing-layoffs-report-says/)).

Agora's entire premise is **real government adoption** of a sovereign identity and voting layer.
Anchoring any part of that to a private company that multiple sovereign governments have actively
banned or shut down for the exact category of privacy/consent concern Agora's own architecture exists
to avoid is a much larger version of the vendor-lock-in problem already rejected twice in this
project's history (Rarimo, then TACEO's live network) — this time at the level of the trust
foundation itself, not one substitutable crypto primitive.

## Verdict

**No, on all three readings.** The non-Orb path doesn't solve the problem Agora actually has
(renewal-stability under a low-entropy identifier); the OPRF-network path is scoped to World's own
ecosystem and reintroduces the vendor-dependency objection at a larger scale; and even setting the
cryptography aside, the operator's live regulatory track record makes it a non-starter for a platform
whose stated goal is real government adoption. This doesn't change the recommendation already on
record in [00-index.md](00-index.md): the closest real shortcut remains self-hosting
`TaceoLabs/oprf-service`'s permissively-licensed code under Agora's own governance, not depending on
anyone else's live network — World ID included.
