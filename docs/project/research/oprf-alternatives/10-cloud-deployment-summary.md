# Cloud Deployment — Summary and What Was Actually Built

*2026-08-12. Synthesizes [08-cloud-hosting-providers.md](08-cloud-hosting-providers.md) and
[09-cloud-security-hardening.md](09-cloud-security-hardening.md), and records what was built
directly in `committee-node/` as a result — this entry is partly research, partly a changelog.*

## The question and the framing correction

Asked: find a cloud service that takes blockchain-currency payment, make sure it's secure, and
make it deployable so committee members can use it. Before answering literally: doing this
centrally — Agora picking one provider and paying for every operator's hosting — would quietly
undo the entire point of the institutional-operator recommendation from
[00-index.md](00-index.md). That design pays for jurisdictional and organizational independence
specifically so no single party can be pressured, subpoenaed, or have its account suspended and
take the whole system down. A shared cloud account or payment relationship collapses a 12-of-35
threshold into one admin credential. So the actual deliverable is **a menu each independent
operator chooses and pays for themselves, plus portable automation and a hardening spec that
holds regardless of which provider they pick** — not a vendor selection.

## What the research found

**Hosting** ([08](08-cloud-hosting-providers.md)): several assumptions didn't survive checking —
Hetzner, OVHcloud, IONOS, and Contabo no longer take crypto directly (only via resellers, which
reintroduces a third party). What does: **Cherry Servers** (Lithuania, genuine bare-metal
isolation, USDC) and **Vultr** (USA, dedicated-vCPU/bare-metal tiers, USDC via BitPay) as the two
strongest commercial picks; **1984 Hosting** and **FlokiNET** (Iceland-domiciled, Monero-accepting,
minimal/no KYC) for jurisdictional diversity; **Phala Cloud** as the one confidential-computing
option that also takes crypto. **Akash Network**, the thematically obvious decentralized-compute
choice, came back weaker than expected — ~58 average active providers in Q1 2026 (a record low),
no compute-layer encryption shipped yet, no SLA — recommended as a one-or-two-members-per-committee
pilot, not a default. The single best-ranked option in the whole document is the least exotic:
**an institution running the node on infrastructure it already owns**, funded by the treasury
stipend from [07](07-treasury-funded-infrastructure.md) rather than a bill Agora pays.

**Security** ([09](09-cloud-security-hardening.md)): `committee-node`'s existing design is already
well-suited to hardening (genuinely outbound-only, non-root, no invented crypto) — most of the
gap is deployment pattern, not architecture. The most important finding: **both of the existing
deployment examples put the encrypted key file and its passphrase on the same disk**, which is
fine on a member's own Raspberry Pi (an attacker needs the physical device) but breaks completely
in the cloud, where "snapshot the volume" is one API call. Confidential VMs (SEV-SNP/TDX) are
worth enabling but not trusting as a security boundary — TEE.Fail (Oct 2025) extracted keys from
all three major vendors' confidential-computing offerings, including forging TDX attestations on a
live production system. And the most consequential recommendation isn't a per-node control at all:
**diversity across the 5 committees in provider, region, jurisdiction, and technology** buys more
than any single hardening measure, because it's what stops one CVE or one provider's outage from
taking out more than one committee.

## What was found and fixed, not just documented

The security research surfaced a real, blocking correctness bug while grounding itself in the
actual code, and it's fixed now, not just recorded:

**`submit_oprf_response`'s extrinsic encoding was missing an argument.** The real pallet call
(`pallets/pallet-identity/src/lib.rs`) takes five arguments — `query_id`, `committee_slot`,
`evaluation`, `committee_pubkey`, `dlog_proof` — but `committee-node/src/extrinsic.rs` only
encoded four, silently dropping `committee_pubkey`. Every extrinsic this component has ever built
would have been rejected by a real chain. The value was already being computed and thrown away:
`wasm_host.rs`'s `EvaluationResult.pk` was marked `#[allow(dead_code)]` with a comment claiming it
wasn't needed on-chain — which was true of an earlier interface guess, not the real one. Fixed
across `extrinsic.rs` (added the field, encoded it in the correct position), `main.rs` (wired
`evaluation.pk` through to the call), and `wasm_host.rs` (corrected the now-wrong comment, removed
the dead-code allowance). Verified: `cargo build --release` and `cargo test --release` both pass,
5/5 tests green.

## What was built

New files in `committee-node/deploy/`:

- **`harden-host.sh`** — the mechanical half of 09's Tier 0/1 host checklist: default-deny-inbound
  firewall, metadata-service blocking, core-dump/hibernation disabling, OS-only auto-updates.
- **`fetch-secret-and-run.sh`** — a generic secret-manager wrapper entrypoint that fetches the
  passphrase into a tmpfs mount at container start and refuses to run if the destination isn't
  actually tmpfs. Directly fixes 09's biggest finding, with **zero changes to `committee-node`
  itself** (it already reads `KEY_PASSPHRASE_FILE` from a path).
- **`docker-compose.prod.yml`** + **`.env.example`** — the hardened container runtime
  configuration: read-only root filesystem, all capabilities dropped, `no-new-privileges`, tmpfs
  for secrets and scratch space, resource limits, bounded log retention, digest-pinned image
  (placeholder until the release pipeline actually publishes and signs one).
- **`README.md`** — an operator-facing runbook, written for a competent sysadmin with no
  blockchain background, walking through provider selection → host hardening → secret setup →
  deployment → startup verification → monitoring → incident response → update policy, each step
  citing exactly which part of 08/09 it implements and why.

`committee-node/README.md` now points to this path and explicitly marks the balenaCloud section
as the citizen-hosted-device path specifically, not deleted (that model isn't decided against),
but no longer the default recommendation for institutional operators.

## What's still open — genuinely, not as a formality

- **The release pipeline doesn't publish a signed image for `committee-node` at all.** It doesn't
  even build it — `.github/workflows/release.yml` only builds the repo-root Dockerfile and still
  references `solochain-template-node` binary names, unexercised template residue. Adding cosign
  keyless signing, SLSA provenance, and an SBOM (09's supply-chain proposal, ranked by
  value-per-maintenance-hour) is real, scoped work — deliberately not done in this pass, since it
  changes the project's shared CI/CD pipeline rather than one component's deployment config, and
  that's a decision worth a deliberate go-ahead rather than a side effect of this research.
- **Nothing here has touched a real chain or a real cloud VM.** Every script and config file was
  reasoned through against the actual source and the research documents, matching this
  component's existing honesty convention (see its Dockerfile's own "NOTE ON VERIFICATION") — not
  empirically tested end-to-end. The first real deployment will find gaps this doesn't anticipate.
- **The RPC-endpoint trust boundary** (09's largest network finding: an unverified RPC response
  can turn a node into a chosen-input OPRF oracle for its own share) is documented and mitigated
  by recommending operators run their own full node, but nothing forces that — it's advisory.
- **Graceful shutdown, `zeroize`-on-drop, and persisting `already_responded` across restarts**
  (09's Tier 3 items 27, 30, 33) are real, identified gaps in `committee-node` itself, not
  deployment configuration — left for a dedicated follow-up rather than folded into this pass.
- **No real committee, no real DKG ceremony, no real institutional operators exist yet.** This
  entire thread — from [00-index.md](00-index.md) through this document — is infrastructure and
  research built ahead of the institution that will eventually use it, the same pattern
  `committee-node/` itself already followed before this session.

## Verdict

The literal ask ("find a cloud service") was the wrong shape of question, and answering it
literally would have quietly undermined the design it was meant to serve — so the deliverable is a
menu plus portable, documented automation instead of a vendor choice. Within that framing, the
research produced a genuinely useful, current (2026) provider landscape, a security architecture
grounded in the actual code rather than generic advice, one real bug fix that would have blocked
every real deployment, and working deployment tooling an institutional operator could pick up
today. What it did not produce, and could not have: an actual live deployment, since no real
committee or institutional operator exists yet to deploy for.
