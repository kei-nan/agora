# Cloud Security Hardening for `committee-node`

*Design specification, 2026-08-12. Written against the actual code in `committee-node/` (all six
source files, the `Dockerfile`, and `docker-compose.example.yml` read in full), not against a
generic cloud-security template.*

**Scope.** The founding-phase model is moving from citizen-hosted phones/laptops/Pis (changelog
`#082`) to **5 independent committees of ~8-15 named institutions each** — universities, NGOs,
election-monitoring bodies — each running this exact container on cloud infrastructure they choose
and pay for themselves. This document covers the security architecture that should hold *regardless
of which provider an operator picks*. **Which** providers to pick is a separate document; nothing
here depends on that choice.

**What's at stake per node.** One member's fragment of a 12-of-35 threshold secret, in one of 5
committees combined by hashed summation. The combined identity anchor stays unpredictable as long as
*one* of the 5 committees has no more than 11 of its 35 shares compromised (changelog `#073`). That
structure is the actual defense. Everything below is about making single-node compromise **rare,
detectable, and scoped to one committee** — not about making it impossible, which no cloud
configuration achieves.

**Caveat that colours the whole document**: nobody has ever run this container. `README.md` is
explicit that no `docker buildx build` was ever executed, and the `submit_oprf_response` extrinsic
has never reached a real chain. Everything here is reasoned from source, and the first real
deployment will surface things this document doesn't anticipate.

---

## What the current skeleton already gets right

Being fair before being critical — several of these are real decisions, not accidents:

- **Genuinely outbound-only, by construction.** There is no listener anywhere: no `TcpListener`, no
  `bind`, no HTTP server framework, no `EXPOSE` in the `Dockerfile`. The only network object in the
  whole crate is a `reqwest::Client` inside `RpcClient` (`committee-node/src/rpc.rs:29-33`) making
  POSTs to a single configured URL. This is the ideal starting point for a default-deny-inbound
  posture — most workloads have to be argued down to this; this one is already there.
- **Non-root runtime user with no shell.** `useradd --system ... --shell /usr/sbin/nologin
  committee`, `/keys` and `/wasm` chowned to it, `USER committee` before the entrypoint
  (`committee-node/Dockerfile:58-59,70`).
- **Multi-stage build.** The runtime stage is `debian:bookworm-slim` + `ca-certificates` only; no
  Rust toolchain, no compiler, no build tooling in the shipped image
  (`committee-node/Dockerfile:52-61`).
- **`cargo build --release --locked`** against a committed `Cargo.lock`
  (`committee-node/Dockerfile:49`) — dependency resolution can't drift silently between builds.
- **A real, standard encryption format rather than invented crypto.** `age`
  (age-encryption.org/v1) passphrase decryption in `committee-node/src/keystore.rs:67-92`, chosen
  deliberately over a bespoke scheme, and honestly documented as *not* tamper-resistant storage.
- **The config layer already knows env vars leak.** `committee-node/src/config.rs:20-25` documents
  why `KEY_PASSPHRASE_FILE` is preferred over `KEY_PASSPHRASE` — "process listings, container
  inspect, crash dumps" — and `resolve_passphrase()` (`config.rs:112-119`) prefers the file. That's
  the right instinct, already present.
- **Fail-closed on stub crypto.** `ALLOW_STUB_SUBMISSION` defaults false (`config.rs:93-95`) and
  `main.rs:153-160` refuses to submit a stub evaluation. A misconfigured deployment degrades to
  "does nothing" rather than "poisons the chain."
- **Chaum-Pedersen nonce handling is correct.** A fresh 32-byte nonce is drawn from `OsRng` on
  *every* evaluation (`committee-node/src/wasm_host.rs:185-188`), and the module docs
  (`wasm_host.rs:24-27`) state plainly that reuse would leak the secret key. This is the one place
  where a subtle mistake would be catastrophic, and it's right — including across process restarts,
  since the nonce is never derived from persistent state.
- **Logging discipline is already good.** Nothing in the codebase logs `blinded_query`,
  `evaluation`, `dlog_proof`, the OPRF secret, the chain seed, or the passphrase. The `info`-level
  events log `query_id`, `posted_at`, the submitter's SS58 address and a tx hash
  (`main.rs:145,172`) — all of which are already public on-chain.
- **`rustls-tls` with `default-features = false`** (`committee-node/Cargo.toml`) — no OpenSSL, no
  system-TLS-library CVE treadmill in the image.
- **`.gitignore` / `.dockerignore` both exclude `keys/*` and `*.age`** — key material can't be
  accidentally committed or baked into a layer.

That's a better baseline than most "minimal skeleton" code. The gaps below are real, but they are
mostly *deployment-pattern* gaps, not design mistakes.

---

## Network exposure

### The outbound-only claim, verified

Confirmed by reading every source file: the process's complete network behaviour is

| Direction | Destination | Purpose |
|---|---|---|
| Outbound HTTP(S) POST | `NODE_RPC_URL` (one host) | `state_getKeysPaged`, `state_queryStorageAt`, `state_getStorage`, `chain_getBlockHash`, `state_getRuntimeVersion`, `system_accountNextIndex`, `author_submitExtrinsic` (`rpc.rs:83-193`) |
| Outbound DNS | resolver | resolving that one host |
| Outbound (image pull only, not at runtime) | container registry | fetching the image |

Nothing else. No inbound anything. **So the correct firewall posture is: zero ingress rules at all,
and egress restricted to a one-host allowlist.**

### Inbound: default-deny, with no exceptions

1. **Delete the provider's default SSH ingress rule.** Every cloud's default security group /
   network security group / firewall rule set opens 22 on creation. Remove it. Verify from outside
   (`nmap -Pn <ip>` should show nothing open).
2. **Prefer no public IP at all**, with egress through NAT / Cloud NAT / a NAT gateway. If the
   provider requires a public IP, that's tolerable *only* with zero ingress rules — but no-public-IP
   is strictly better because it removes the node from internet-wide scanning entirely.
3. **Block the cloud metadata service (`169.254.169.254`) from the container** unless the wrapper
   entrypoint genuinely needs it to fetch a secret (see below), in which case fetch-then-drop. On
   AWS, enforce IMDSv2 and set the metadata hop limit to 1 so a container can't reach it through the
   host's NAT.

### Management access: don't expose SSH

**Recommendation: no `sshd` reachable from any network the operator doesn't already fully control.**
Ranked, most to least preferred:

1. **No `sshd` at all.** Use the provider's out-of-band console (serial console / browser SSH /
   "connect" button) for the rare break-glass case. This removes an entire always-listening daemon
   from a machine whose only job is to make outbound HTTP calls.
2. **Provider-brokered, identity-aware access**: AWS SSM Session Manager, GCP IAP TCP forwarding,
   Azure Bastion. Port 22 is never on a routable address; access is authenticated by cloud IAM and
   logged in the cloud audit trail — which is *also* what makes the access detectable later.
3. **If a real `sshd` is unavoidable** (some institutions' own compliance regimes require their
   standard agent-based management): bind it to a private interface only, `PasswordAuthentication
   no`, `PermitRootLogin no`, key-only or certificate-based auth, source-IP allowlist to the
   institution's own management range, and no agent forwarding.

**Be honest about what this buys.** Removing SSH does not remove the way in — it *moves* it to cloud
IAM. Anyone with console credentials for the project can attach a serial console, snapshot the disk,
or read VM memory through the control plane. So "no SSH" is only meaningful when paired with
hardware-key MFA and least-privilege IAM on the cloud account (checklist items 5 and 25). This is a
trade, not a win: it swaps a service the operator has to patch and monitor for a service the provider
patches and monitors, and it makes access events land in an audit log the operator can alarm on.

### Egress: allowlist to the chain endpoint

A default-allow egress policy means a compromised node can post the share to any collection point on
the internet. Restricting egress to the RPC endpoint's host plus DNS raises that cost substantially.

Honest limitation: it does **not** eliminate exfiltration. An attacker with code execution can still
encode data into extrinsic bytes and push it through the one allowed channel. Egress filtering makes
theft slower, noisier and more likely to trip an alert — it is not a containment boundary.

### The RPC endpoint is an unauthenticated trust boundary — this is the largest network finding

`poll_once` (`main.rs:111-143`) takes whatever `state_getKeysPaged` / `state_queryStorageAt` return,
decodes it as a `PendingQuery`, and evaluates it. There is **no verification that the query is
actually in chain state**: no `state_getReadProof`, no light-client header check, nothing. The node
trusts the JSON it gets back.

On-chain, `submit_oprf_query` requires the caller to be a registered citizen
(`pallets/pallet-identity/src/lib.rs:1036`, `ensure!(Self::is_citizen(&who), ...)`). **A malicious or
MITM'd RPC endpoint bypasses that gate entirely** and turns the node into a chosen-input OPRF oracle:
the attacker feeds fabricated `PendingOprfQueries` entries containing blinded points of their
choosing, the node dutifully evaluates them under its share, and the attacker reads the response —
either from the `author_submitExtrinsic` call they're also intercepting, or off-chain from the node's
own submission attempt.

That is precisely the offline-guessing capability the OPRF exists to prevent, applied to one
committee's share. It does not break the system alone (all 5 committees are needed to reconstruct an
anchor, per `#073`'s hashed-summation argument), but it converts "compromise this operator's machine"
into the much easier "sit between this operator and their RPC provider."

Mitigations, in order of strength:

1. **The operator runs their own `agora-node` full node**, and `NODE_RPC_URL` points at loopback or a
   private VPC address. The trust boundary collapses to a machine the operator controls. This is the
   right answer for an institutional operator with IT staff; it costs one more VM.
2. **HTTPS to a named, contractually-known endpoint**, never plain HTTP. Note the default is
   `http://127.0.0.1:9944` (`config.rs:55`) — correct for dev, wrong for anything else. Consider
   certificate pinning at the network layer.
3. **Second-source cross-check** (checklist item 22): a small out-of-band script comparing the
   queries the node sees against a second, independent RPC provider. Cheap, and it catches exactly
   this attack.
4. **Upstream fix** (Agora's job, not the operator's): verify storage proofs via `state_getReadProof`,
   or embed a light client. The desktop app's intended smoldot direction (per `/CLAUDE.md`) applies
   here for the same reason.

One related quirk worth knowing: `RpcClient::get_storage` swallows RPC errors and returns `Ok(None)`
(`rpc.rs:110-121`), so an endpoint returning errors is indistinguishable from "no data." That's a
silent-failure channel, and a monitoring signal (see below).

### One instance per key — a hard operational constraint

`extrinsic.rs` uses `Era::Immortal` (`extrinsic.rs:66,104`) with a nonce fetched fresh per submission
via `system_accountNextIndex` (`extrinsic.rs:91`). Two instances holding the same
`chain_account_seed` will fetch the same nonce and produce colliding extrinsics — and an `Immortal`
extrinsic stays validly signed and replayable indefinitely, a tradeoff `extrinsic.rs:49-51` already
documents. **No autoscaling group with size > 1, no rolling deploy with instance overlap, no warm
standby that also holds the key.** A standby that holds the share also doubles the memory-exposure
surface, so this constraint is doing double duty.

---

## Secrets and key-share custody

### What `age`-on-disk actually buys, and what breaks it in cloud

`keystore.rs:67-92` is a correct use of a good format. The file alone, without the passphrase, is
useless. That's real.

**The problem is not the format — it's where the deployment examples put the passphrase.** Both the
`README.md` `docker run` example (lines 130-137) and `docker-compose.example.yml` (lines 20-30) mount
the age file *and* the passphrase file from the same host filesystem:

```
-v $PWD/keys:/keys:ro
-v $PWD/local-dev/passphrase.txt:/run/secrets/passphrase:ro
```

On a member's own Raspberry Pi, that split at least meant an attacker needed the physical device. On
a cloud VM, **"snapshot the volume" is a single API call**, routinely available to anyone with cloud
console credentials, to the provider's staff, and to the provider's backup systems. A snapshot yields
ciphertext *and* passphrase together. In that configuration, `age` is decorative.

**Rule: the passphrase must never be written to the VM's persistent disk.** Two workable patterns:

- **Secret manager → tmpfs → exec.** A wrapper entrypoint fetches the passphrase from AWS Secrets
  Manager / GCP Secret Manager / Azure Key Vault / HashiCorp Vault, writes it to a `tmpfs` mount
  (`--mount type=tmpfs,destination=/run/secrets`, or compose's `tmpfs:` key), and `exec`s the binary.
  **This requires zero changes to `committee-node`** — `config.rs` already reads
  `KEY_PASSPHRASE_FILE` from a path. Minimum friction, and it produces three things the current
  design has none of: an auditable `Decrypt`/`GetSecretValue` event to alarm on, a remote kill switch
  (revoke the grant and the node cannot restart with the key), and a passphrase that survives no
  reboot and no disk snapshot.
- **Human-supplied at each start.** Strongest against the provider, worst for availability — a 3am
  reboot means the node is down until someone types it. With a 6-day query SLA
  (`runtime/src/configs/mod.rs:297`, `OprfQuerySlaBlocks = ConstU32<{ 6 * DAYS }>`) and *n*-of-*n*
  across 5 committees, availability genuinely matters. Recommend: secret manager for normal
  operation, with the manual path documented as the fallback.

Honest tradeoff on the secret-manager route: the cloud IAM role that can read the passphrase makes the
provider *able* to decrypt the share. That is not an improvement in the "provider is the adversary"
model — but it is not a regression either, since the provider could already snapshot the disk. It is a
clear improvement against disk-snapshot theft, backup exposure, and forgotten dev copies, and it adds
audit logging where there was none.

### The share is plaintext in process memory, 24/7

This is the part no deployment configuration fixes. Concretely, from the code:

- `keystore.rs:44-48` — `Secrets` holds the two secrets as hex `String`s. The decrypted `Vec<u8>`
  (`keystore.rs:84-88`) and both `String`s are dropped without zeroization; there is no `zeroize`
  dependency in `Cargo.toml`.
- `config.rs:112-119` returns the passphrase as a plain `String`, and `keystore.rs:81` makes a second
  copy (`SecretString::from(passphrase.to_string())`). Both linger in freed heap.
- `main.rs:51-52` binds `seed: [u8; 32]` and `oprf_secret_key: Vec<u8>` for the entire process
  lifetime.
- Worst of all: **the OPRF share is copied into wasmtime's guest linear memory on every evaluation**
  (`wasm_host.rs:181-191` — a 160-byte host buffer, then `memory.write` into the guest). `oprf_dealloc`
  is called (`wasm_host.rs:201`) but frees without any guarantee of zeroing. Over time the share is
  smeared across a large, long-lived wasmtime allocation.

Consequence, stated plainly: **anything that can read this process's memory gets the share.** A core
dump, `/proc/<pid>/mem` via root, a container escape, a debugger, an OOM dump shipped to a logging
service, a VM hibernation image, or a provider-side live migration that writes guest RAM to provider
storage. This is why the incident-response section below insists that any root-level compromise be
treated as *full* share disclosure — there is no partial-compromise story for a key that lives in RAM.

Cheap operator-side mitigations: disable core dumps (`--ulimit core=0` on the container, plus host
`kernel.core_pattern` / `fs.suid_dumpable=0`), disable hibernation and suspend-to-disk, and set
`onHostMaintenance=TERMINATE` (or the provider equivalent) so the VM is destroyed rather than
live-migrated with its memory. Upstream fixes — `zeroize`, `mlock`, `PR_SET_DUMPABLE=0` — are
listed in Tier 3.

### Cloud KMS / HSM: a partial answer, and it's important to say which part

**What it does:** keeps the *wrapping* key out of the VM, so unwrapping the share requires an
authenticated, audit-logged API call. That log line — "this key was unwrapped at time T by principal
P" — is evidence the current design produces none of, and it is arguably the single most valuable
thing KMS adds here.

**What it does not do:** it cannot hold the OPRF share and evaluate on it. The OPRF operation is
BabyJubJub scalar multiplication inside a wasm module (`wasm_host.rs:14-19`). No cloud HSM exposes
BabyJubJub; KMS products do RSA/ECDSA on NIST curves, AES, and (on some providers) Ed25519.
**"The key never leaves the HSM" is simply not achievable for the OPRF share** — the plaintext must
reach VM memory for the evaluation to happen at all. Any vendor pitch to the contrary is describing a
different problem.

Practical shape: `age` has no native KMS recipient, so don't try to bolt one on. Either keep the age
file and put its *passphrase* in the secret manager (above), or switch the file to `sops` with a KMS
key — a standard, widely-deployed tool that already implements this envelope pattern, requiring a
wrapper script rather than new crypto.

One genuinely non-obvious partial win: the *chain account seed* is a different case from the OPRF
share. The chain accepts `MultiSignature::Ed25519` (`extrinsic.rs:63`), and `AccountId32` is
scheme-agnostic, so an operator could in principle sign `submit_oprf_response` with a KMS-held
Ed25519 key and never hold that seed in VM memory at all. That removes one of the two secrets from
the exposure surface. It costs an upstream code change (sign via KMS API instead of
`sr25519::Pair::from_seed`), Ed25519 signing support must be confirmed for the specific provider, and
the `CommitteeMembers` registration path should be verified to accept an ed25519-derived
`AccountId32`. Worth doing eventually; it does **not** help the OPRF share, which is the crown jewel.

### Confidential computing: enable it, don't believe it

**The narrow case it addresses is exactly the gap above.** The decrypted share sits in VM RAM
continuously; on a normal cloud VM the hypervisor can read that RAM, and so can anyone who can take a
memory snapshot through the control plane. A confidential VM (AMD SEV-SNP, Intel TDX) encrypts guest
memory with a key the host doesn't hold. That is precisely the right shape of countermeasure.

**The project's prior skepticism about TEEs was correct and still applies to the trust-anchor claim.**
The 2026 state of the art is not reassuring:

- **TEE.Fail** (Georgia Tech + Purdue, disclosed Oct 2025) is a sub-$1,000 DDR5 memory-bus interposer
  built from resold parts. It extracted cryptographic keys from Intel SGX, Intel TDX *and* AMD
  SEV-SNP — the SEV-SNP attack working **even with Ciphertext Hiding enabled**. Worse for the trust
  story: the researchers forged TDX attestations convincingly enough to impersonate a genuine enclave
  on a live production system (Ethereum's BuilderNet) and access confidential transaction data.
- Both Intel and AMD classify physical attacks as **out of scope** for their threat models — Intel
  said so explicitly in its TEE.Fail response. And in a cloud deployment, *the provider is the one
  party with physical access.*
- Related work compounds this: Heckler (malicious-interrupt attacks on confidential VMs), and a 2026
  paper recovering **WebAssembly code through SEV-SNP's exposed address space** — notable, given this
  workload is literally a wasm module.

**Judgment.** Attestation is not a trust anchor you can build on: it is forgeable by an adversary with
physical access to a DDR5 machine, which is the adversary a confidential VM is nominally protecting
you from. So a CVM does **not** protect the share from a determined cloud provider, or from a state
that can compel one.

But it does raise the bar meaningfully against the failure modes that are *more likely* for a
university or NGO operator: other tenants, a remotely-compromised hypervisor, casual insider access,
memory snapshots taken through the normal control plane, and accidental exposure via live migration,
hibernation or a support dump. And it costs almost nothing — a checkbox at VM creation, ~2-10%
performance overhead, and a small surcharge (GCP N2D SEV-SNP: about $0.00275 per vCPU-hour plus
$0.00037 per GiB-hour). No code change.

**So: enable it, treat it as one layer, and build no part of the security argument on attestation.**
Two operational notes: Confidential VMs generally force `onHostMaintenance=TERMINATE` (no live
migration), which is the behaviour we want anyway; and GCP has flagged that from August 2026 SEV-SNP
Confidential VMs may see longer boot times and performance changes from a guest-kernel migration,
expected resolved around November 2026 — plan the first deployment around that.

**AWS Nitro Enclaves** is a stronger, differently-shaped option: an isolated enclave with no
persistent storage and no interactive access, and KMS condition keys that let a CMK be unwrapped
*only* by an enclave matching specific PCR measurements. It is genuinely best-in-class **if the
operator is on AWS** — but it requires restructuring the application (the enclave has no network; the
RPC calls would need a vsock proxy), and it is AWS-only, which cuts against "hold regardless of
provider." Recommendation: **permitted and encouraged for AWS operators, never required, and the
design must not assume it.**

### The decisive control is diversity, not any single hardening measure

With 5 independent committees, the worst realistic outcome is **all 5 operators making the same
infrastructure choices** — one provider CVE, one base-image CVE, one region outage, and all five fall
together. Committee-selection criteria should therefore require diversity across the 5 committees in
provider, region, jurisdiction, host OS, and confidential-computing technology. This is the same
logic that made `#073` insist on independent founding groups per committee rather than one shared
group, and it buys more than any control on any single node.

---

## Supply-chain / build integrity

### Where the project actually is today

Checked, not assumed:

- `.github/workflows/release.yml` builds the **repo-root** `Dockerfile` and pushes
  `ghcr.io/<repo>:<tag>` via `docker/build-push-action@v6` with `push: true` — **no signing, no
  provenance attestation, no SBOM**.
- It does not build `committee-node` at all.
- Its `release-binaries` job still uploads `target/release/solochain-template-node` and
  `solochain_template_runtime.compact.compressed.wasm` — untouched residue from the
  polkadot-sdk-solochain-template. That workflow is demonstrably not exercised.
- Action pinning is inconsistent: `jlumbroso/free-disk-space` is pinned by commit SHA, while every
  `docker/*` action floats on a major tag.
- `committee-node/Dockerfile` uses floating base tags — `rust:slim-bookworm` (line 35) and
  `debian:bookworm-slim` (line 52) — and runs `apt-get update && apt-get install` at build time
  (lines 42, 56). The image is therefore not reproducible today, and "the same Dockerfile" produces
  different images on different days.

So the honest current answer to "how does an operator verify what they're running?" is: **they
can't.**

### Proposal, ranked by value-per-unit-of-maintenance

Deliberately sized for a small open-source project — every item below is a few lines of CI config,
not a program.

1. **Cosign keyless signing (Fulcio + Rekor) in GitHub Actions.** This is the highest-value item and
   the one that most directly avoids `#082`'s concern. Keyless signing uses the workflow's OIDC
   identity and a short-lived Fulcio certificate, with the signing event recorded in the public Rekor
   transparency log. **There is no long-lived signing key to protect, rotate, or steal** — the
   supply-chain target that `#082` worried about is never created. Operator verification is one
   command, binding to the exact workflow identity:
   ```
   cosign verify \
     --certificate-identity-regexp '^https://github\.com/kei-nan/agora/\.github/workflows/release\.yml@' \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com \
     ghcr.io/kei-nan/agora/committee-node@sha256:<digest>
   ```
   **Verify by digest, and run by digest.** Tags are mutable; a digest is not.
2. **`actions/attest-build-provenance`** — GitHub's built-in SLSA-v1 provenance. One extra step in the
   workflow; the operator verifies with `gh attestation verify oci://... --repo kei-nan/agora`. Lower
   friction than the standalone SLSA generator workflows and adequate for this scale.
3. **SBOM and provenance as OCI attestations**: `docker buildx build --sbom=true --provenance=true`
   emits SPDX plus provenance alongside the image in the same push. Nearly free, and it *reduces*
   adoption friction — university and NGO procurement processes increasingly require an SBOM before
   IT will sign off on running third-party software.
4. **Digest-pin everything.** `FROM rust:1.96-slim-bookworm@sha256:...` and
   `FROM debian:bookworm-slim@sha256:...` in `committee-node/Dockerfile`; pin every GitHub Action by
   commit SHA (matching what `free-disk-space` already does); keep `--locked`. Cheap, immediate
   determinism win.
5. **Partial reproducibility — pursue it for the binary, not the image.** Be realistic: a 2026 study
   found roughly 2.7% of Dockerfiles produce bitwise-identical images without infrastructure changes,
   rising to about 21% with them. BuildKit does support `SOURCE_DATE_EPOCH` with
   `rewrite-timestamp=true`, and Cargo passes `SOURCE_DATE_EPOCH` through to build scripts. The
   achievable target is a **reproducible binary**: pinned toolchain via `rust-toolchain.toml`,
   `--locked`, `CARGO_INCREMENTAL=0`, `-C debuginfo=0`, `--remap-path-prefix`, and a published
   `sha256` of `target/release/committee-node`. Then anyone with a Rust toolchain can independently
   answer "did Agora ship what the source says," even if full image layers differ.
6. **An independent rebuilder, and a two-person release rule.** This is the piece that actually
   addresses `#082`'s question rather than deferring it: have at least one party outside the release
   author rebuild the tagged source and countersign the published digest. Even *one* independent
   rebuilder converts "trust Agora's CI" into "trust Agora's CI **and** an independent rebuilder."
   One volunteer, one command, run per release — entirely realistic at this project's size.

**What not to do:** don't invent a bespoke signing scheme; don't create a long-lived signing key held
on a maintainer's laptop (that *is* the single point of compromise `#082` names — keyless avoids
creating one at all); don't require operators to build from source, which guarantees drift and
skipped verification in practice.

**Honest limit.** Cosign keyless doesn't eliminate authority, it relocates it: whoever can trigger the
release workflow on the repo can author a signed image. The remaining controls are organisational —
hardware-key MFA for all maintainers, protected `main` with required reviews, tag protection, and a
GitHub Environment with required reviewers gating the release job. State that plainly rather than
implying signing solves it.

---

## Update and patch governance

### Direct answer to `#082`'s open question

`#082` left unresolved: *who is allowed to author a device update without becoming a new centralized
single point of compromise?* For the cloud/institutional model, the answer has two halves:

**Half one — remove the standing key.** Keyless signing (above) means there is no firmware-signing key
to steal. The supply-chain target `#082` describes is never brought into existence.

**Half two, which matters more — remove the *reach*.** Signing authority still exists in the form of
"whoever can run the release workflow." The defense that actually caps blast radius is that **nobody
can make an operator apply an update.**

**Specification: pull-based, operator-scheduled, signed, digest-pinned releases. No push channel, no
auto-update agent, no remote management plane, no telemetry callback to Agora infrastructure.**

The reasoning is a straight blast-radius comparison. With a push channel, an attacker who compromises
the release pipeline reaches **all 5 committees simultaneously** — a total break of a structure
explicitly designed so that one honest committee suffices. With pull-based updates on staggered
operator schedules, the same attacker reaches committees **one at a time, over days, in public, with a
Rekor transparency-log record of every signing event**. That difference is the entire security
argument for the model.

### Retire the balenaCloud path for the institutional model

`committee-node/README.md:155-179` documents balenaCloud as the intended fleet-management and OTA
update mechanism. That was a defensible choice for citizen-owned devices whose owners cannot be
expected to run `docker pull` — but for institutions with IT staff, the justification evaporates,
and what remains is a fleet-management plane through which **one party can change all 5 committees'
code at once**. That is exactly the property the 5-committee independence structure exists to
prevent. Recommend explicitly retiring the balena path when the institutional model is adopted, and
recording why.

Related operator-side warning: Watchtower-style auto-pull containers are a common ops habit that
silently re-creates a push channel through the registry. Prohibit it in the operator agreement.

### What Agora should publish per release

- An **immutable digest** (`sha256:...`), never a moving tag.
- A **cosign signature** with its Rekor transparency-log entry.
- A **provenance attestation** (SLSA v1 via `actions/attest-build-provenance`).
- An **SBOM** (SPDX).
- A **human-readable changelog** stating what changed and whether it is security-relevant.
- A **severity label**, which is the operationally important part:
  - `routine` — apply at the operator's convenience.
  - `security` — apply within a stated window.
  - `breaking` — the chain's call encoding or storage layout changed; nodes that don't update will
    produce invalid responses from block *N*. This case is real and near-term: the runtime's
    `submit_oprf_response` signature can change, and `config.rs`'s `CALL_INDEX` default has already
    had to move once (from a guessed 13 to the real 16 — see `config.rs:37-41`).

### Staggering as policy

Make it explicit rather than incidental: **no more than 2 of the 5 committees should adopt a new image
within the same 72-hour window**, except for a release the Emergency Council has flagged as urgent.
This is a governance rule, costs nothing, and directly buys the property that one bad image cannot
take all 5 committees at once.

### The forced-update lever, and its honest limit

Some updates genuinely need all 5 to move — a runtime upgrade changing the extrinsic encoding, or a
real vulnerability in the crypto core. Route those through the governance Agora already has (an
Emergency-Council-flagged advisory) rather than inventing a new mechanism. But be honest: governance
can **ask**; it cannot **make** an operator update, and that is the intended property, not a
deficiency. The enforcement is economic and reputational — a stale node stops producing valid
`submit_oprf_response` calls, its committee starts missing the 6-day SLA, and that failure is visible
to everyone on-chain.

### Operator-side update procedure

Verify signature and provenance → pin the new digest in the deploy config → apply in a maintenance
window → confirm the startup log shows the real wasm module loaded and the roster check passing →
keep the previous digest available for immediate rollback.

---

## Monitoring and incident response

### What to monitor — all of it derivable from what the code already emits

The existing log events map almost one-to-one onto the signals worth alerting on:

| Signal | Source | Why it matters |
|---|---|---|
| Process up / restart count | container runtime | baseline liveness |
| Time since last successful poll cycle | absence of `main.rs:114`/`:145` events | the core health metric |
| Poll failures | `main.rs:70` `"poll cycle failed, will retry next interval"` | RPC unreachable or misbehaving |
| Queries seen per interval | `main.rs:145` `"found pending OPRF query"` | workload volume |
| Responses submitted | `main.rs:172` `"submitted submit_oprf_response"` | actual productivity |
| Submission failures | `main.rs:175`, `main.rs:177` | signing envelope or endpoint problems |
| **Queries seen but unanswered, and their age vs. the 6-day SLA** | derived | the metric that actually predicts an SLA miss |
| Wasm evaluation error classes | `wasm_host.rs:54-63` `describe_error_code` | malformed inputs, or a wrong configured key (`ERR_ZERO_SECRET_KEY`) |
| **STUB mode active** | `wasm_host.rs:106-112` warning | page immediately — the wasm module is missing |
| **Not on the `CommitteeMembers` roster** | `main.rs:89-92` warning | this node's responses will be rejected |

**Never log or export**: `blinded_query`, `evaluation`, `dlog_proof`, the OPRF secret key, the chain
seed, or the passphrase. The code doesn't today; the operational rule is to keep it that way. Pin
`RUST_LOG=info` (as `Dockerfile:75` already does) rather than letting anyone set `trace` on a hunch.

On the `submitter` account logged at `main.rs:145`: it is already public on-chain, so logging it is
not a new information leak. It does create *copies in places with different access control*, so bound
log retention (30-90 days) and do not ship logs to a third-party SaaS by default.

**Implementation caution that undoes section 1 if ignored:** adding a Prometheus `/metrics` endpoint
would give this component its first inbound listener. Don't. Either bind metrics to loopback or a unix
socket with a local agent pushing outbound, or — simpler — emit structured JSON logs to the provider's
log agent, which is an outbound path that already exists.

### Signals that indicate *compromise*, not merely downtime

- **`submit_oprf_response` extrinsics on-chain from this committee account that the operator's own
  logs do not show it submitting.** This is the single highest-value detector, and crucially it is
  available to *anyone* watching the chain, not just the operator — so it works even when the
  operator is the problem. Recommend a public, Agora- or Audit-Office-operated watcher comparing
  per-slot response counts and patterns against pending queries.
- **A jump in queries seen with no matching on-chain `OprfQuerySubmitted` events** — the fabricated-
  query attack from the network section. Detectable by second-sourcing the chain state.
- Unexpected nonce advancement on the committee account.
- Outbound connection attempts blocked by the egress allowlist.
- Container restart with no corresponding operator action.
- **Any snapshot or image-export of the data volume in the cloud audit log** — this is the primary
  theft path; alarm on it.
- **`Decrypt` / `GetSecretValue` calls from an unexpected principal or at an unexpected time.** This
  is the concrete payoff of moving the passphrase into a secret manager: key access becomes an
  auditable, alarmable event rather than an invisible file read.

### How a cloud incident feeds the existing rotation governance

The governance is already decided (`#073`) and needs no redesign: emergency rotation requires an
**AND** of `pallet-emergency-council`'s 2/3 supermajority and `pallet-courts`' Oracle endorsement,
both referencing the **same incident hash** and the **same `committee_id`** (`Option<u8>`, `None` for
system-wide) — replacing the runtime's current placeholder `EmergencyRotationOrigin =
EnsureRoot<AccountId>` (still present at `runtime/src/configs/mod.rs:283` and `:323`). The operator
procedure maps onto it directly:

1. **Contain locally, immediately, unilaterally.** The operator does not need governance approval to
   stop their own node. Stop the container; **revoke the secret-manager/KMS grant** so the passphrase
   cannot be re-fetched — this is the fastest true kill switch and it works even if the VM is no
   longer under the operator's control; image the VM for forensics *before* destroying it, to an
   encrypted, access-logged location.
2. **Declare publicly and fast.** Publish an incident report; **its hash is the incident hash both
   governance arms must reference.** Contents: what was compromised, when the exposure window opened
   and closed, whether the OPRF share must be presumed disclosed, the affected image digest, and
   which `committee_id`.
3. **Open both governance arms in parallel** — file the court case for the Oracle-endorsement arm and
   notify the Emergency Council for the 2/3 arm, both citing the same incident hash and
   `committee_id`.
4. **Keep the scope per-committee.** One operator's cloud compromise rotates *one* committee, not all
   five. That is exactly what `Option<u8>` scoping is for, and why `#073` chose it: the other four
   keep serving and registration continues.
5. **Presumption rule, which the operator agreement should state up front:** because the share sits
   in plaintext in VM memory continuously, **any root-level compromise, any unexplained snapshot, or
   any provider-side incident affecting the host must be treated as full share disclosure.** There is
   no partial-compromise story for a key in RAM. Stating this in advance removes the temptation to
   under-declare.

**Why honest disclosure is cheap here, and why that matters.** Hashed-summation combination means the
combined anchor stays unpredictable as long as *one* committee is honest (`#073`). One compromised
committee is not a break. So the correct posture is "declare loudly, rotate deliberately" rather than
"conceal and hope" — and because the structural cost of disclosure is low, honest disclosure is
actually likely to happen. That is a designed-in property worth naming explicitly to operators.

Two further notes: a compromise detected by a *third party* (the chain watcher above) should trigger
the identical path — the governance arms do not require the operator's cooperation, which is correct.
And a compromise of the **image or release pipeline** rather than one node is the system-wide case:
`committee_id = None`.

---

## Minimum-viable hardening checklist

Ranked by effort vs. impact. Written for a competent sysadmin at a university or NGO — no blockchain
knowledge assumed. Tiers 0-2 are the operator's job; Tier 3 is Agora's, listed so operators know what
is still missing.

### Tier 0 — blocking. Do all of these before the node touches real key material. (~1-2 hours)

1. **Dedicated VM, in a dedicated cloud project/account, for this workload only.** No co-tenancy with
   other institutional workloads. *Why: it minimises the set of people and processes that can read
   this machine's disk or memory.*
2. **Zero inbound firewall rules.** Delete the provider's default SSH rule. Verify externally with
   `nmap -Pn <ip>` — nothing should be open. *The node never accepts a connection; nothing legitimate
   breaks.*
3. **No public IP** if the provider supports egress via NAT. If one is required, still zero ingress
   rules.
4. **No exposed `sshd`.** Use the provider's out-of-band console or identity-aware access (SSM Session
   Manager / IAP TCP forwarding / Azure Bastion). If `sshd` is unavoidable: private-interface bind,
   key-only, `PasswordAuthentication no`, `PermitRootLogin no`, source allowlist.
5. **Hardware-key MFA and least-privilege IAM on every account that can reach this cloud project.**
   *With no SSH, cloud IAM is the way in — it is now the primary attack surface, not a secondary one.*
6. **Run the image by digest, never by tag**, and verify its cosign signature and provenance before
   the first run and before every update.
7. **The passphrase never touches persistent disk.** Mount `/run/secrets` as `tmpfs`
   (`--mount type=tmpfs,destination=/run/secrets`) and have the entrypoint fetch the passphrase into
   it at start. Confirm with `findmnt /run/secrets` that it is `tmpfs`. *A disk snapshot must not
   yield both the age file and its passphrase — the current examples put them on the same disk.*
8. **Startup verification, every time.** `ALLOW_STUB_SUBMISSION` unset; `COMMITTEE_SLOT` correct; logs
   show `"loaded real OPRF crypto-core Wasm module"` (`wasm_host.rs:139`) and `"confirmed: this
   account is on the CommitteeMembers roster"` (`main.rs:87`). **If you see STUB mode, stop and fix
   it.**
9. **Exactly one running instance per key.** No autoscaling group with size > 1, no rolling deploy
   with overlap, no key-holding warm standby. *Two instances collide on the extrinsic nonce
   (`extrinsic.rs:91,104`), and a standby doubles the memory-exposure surface.*

### Tier 1 — high impact, low effort. (~half a day)

10. **Egress allowlist:** outbound permitted only to the chain RPC host and DNS. Deny everything else,
    including `169.254.169.254` from inside the container. *Raises the cost of exfiltrating the share
    — it does not make it impossible.*
11. **`NODE_RPC_URL` over HTTPS, to an endpoint you control.** **Strongly preferred: run your own
    `agora-node` full node** and point at loopback or a private VPC address. *A rogue or MITM'd RPC
    endpoint can feed this node fabricated queries and use it as a chosen-input OPRF oracle — see the
    network section.*
12. **Container runtime flags:** `--read-only` root filesystem, `--cap-drop=ALL`,
    `--security-opt no-new-privileges`, `--ulimit core=0`, a memory limit, `--pids-limit`, `/keys` and
    `/wasm` mounted `:ro`, `tmpfs` for `/tmp` and `/run/secrets`, `--restart unless-stopped`.
13. **Disable core dumps host-wide** (`fs.suid_dumpable=0`, a `kernel.core_pattern` that discards, or
    systemd `LimitCORE=0`) **and disable hibernation / suspend-to-disk.** *The OPRF share is plaintext
    in RAM (`wasm_host.rs:181-191`); a dump or hibernation image is a copy of it on disk.*
14. **Encrypt the volume holding the age file** (provider-managed keys, CMEK if available), **and
    alarm on any snapshot or image-export of it** in the cloud audit log. *That snapshot is the
    primary theft path.*
15. **Automatic OS security updates on the host** (`unattended-upgrades` or the provider equivalent) —
    **but never auto-update the application image.** *These are different things: auto-patching the OS
    is good practice; auto-pulling the app image re-creates the push channel the update-governance
    section exists to eliminate.*
16. **Ship logs outbound to the provider's log service** (no listener, no agent port). Alert on:
    process down for more than two poll intervals; time-since-last-successful-poll exceeding
    threshold; the STUB-mode warning; repeated `author_submitExtrinsic failed`; any occurrence of
    "NOT on the CommitteeMembers roster".
17. **Bounded log retention (30-90 days), no third-party SaaS log shipping by default.**
18. **Time sync via the provider's NTP service.** *TLS certificate validation depends on a
    roughly-correct clock.*
19. **A one-page written incident runbook, tested once.** Who to call; how to stop the container; how
    to revoke the secret-manager grant; where the incident report is published (its hash becomes the
    on-chain incident hash); who files the court case and notifies the Emergency Council.

### Tier 2 — worthwhile, more effort. (days)

20. **Enable a Confidential VM** (AMD SEV-SNP or Intel TDX) if the provider offers it. A VM-creation
    checkbox, ~2-10% overhead, small surcharge, no code change. Set `onHostMaintenance=TERMINATE`.
    *Defense-in-depth against hypervisor, other-tenant and control-plane memory exposure — **not** a
    trust anchor: see TEE.Fail. Do not weaken any other control because this is on.*
21. **Move the passphrase into the cloud secret manager / KMS** via a wrapper entrypoint that fetches
    into `tmpfs` and `exec`s the binary — **zero changes to `committee-node`**. *Gains an auditable,
    alarmable key-access event and a real remote kill switch (revoke the grant).*
22. **Second-source the chain state.** A small out-of-band script comparing the queries the node acts
    on against an independent RPC provider. *Detects a rogue or MITM'd primary endpoint — the single
    most dangerous network attack against this component.*
23. **A public chain-side watcher** (Agora or the Audit Office operates it, not each operator):
    alert when a slot's responses don't match its queries, or when an account submits responses its
    operator's logs don't show. *This detector works even when the operator is the problem.*
24. **Diversity as a committee-selection criterion.** The 5 committees must not converge on the same
    provider, region, jurisdiction, base OS, or CVM technology. *This buys more than any single
    control on any single node.*
25. **Restrict and review who at the institution can reach the cloud project.** Quarterly access
    review; offboarding checklist that includes revoking this project's access.
26. **Rehearse the rotation path end-to-end at least once before production.** *A rehearsed rotation
    is worth considerably more than an unrehearsed plan, and it will find gaps in `#073`'s
    two-arm governance flow that reading it will not.*

### Tier 3 — requires upstream changes in Agora, not operator configuration

27. **`zeroize` on the decrypted buffer, `Secrets`' hex strings, the passphrase, `seed`,
    `oprf_secret_key`, and the 160-byte wasm input buffer** (`keystore.rs:44-48,81-92`,
    `main.rs:51-52`, `wasm_host.rs:181-191`). No `zeroize` dependency exists today.
28. **Set `RLIMIT_CORE=0` and `PR_SET_DUMPABLE=0` in-process at startup**, plus `mlock` where
    feasible, so memory protection doesn't depend on operator configuration.
29. **Verify storage proofs (`state_getReadProof`) or embed a light client**, so a rogue RPC endpoint
    cannot feed fabricated `PendingOprfQueries` entries (`main.rs:111-143`, `rpc.rs:83-150`).
30. **Persist `already_responded`** (`main.rs:64`) across restarts — it is an in-memory `HashSet`
    today, so a restart re-evaluates and re-submits work already done, and it grows without bound in
    a long-running process.
31. **Digest-pin base images, add `rust-toolchain.toml`, add reproducible-build flags, publish the
    binary's `sha256`** (`Dockerfile:35,52`).
32. **Add cosign keyless signing, `actions/attest-build-provenance`, and
    `--sbom=true --provenance=true` to the release workflow — and make it actually build
    `committee-node`.** `.github/workflows/release.yml` today builds only the repo-root Dockerfile,
    signs nothing, and still references `solochain-template-node` binaries.
33. **Handle `SIGTERM` gracefully.** Tokio's `signal` feature is already a dependency
    (`Cargo.toml`) but is never used, so a stop can interrupt a half-built extrinsic.
34. *(Optional, partial)* **Sign the extrinsic with a KMS-held Ed25519 key** instead of an in-memory
    sr25519 seed (`extrinsic.rs:63` shows the chain accepts `MultiSignature::Ed25519`). Removes one of
    the two secrets from VM memory. Does not help the OPRF share.
35. **Fix the `submit_oprf_response` argument mismatch — blocking for any real deployment.** The real
    call takes five arguments including `committee_pubkey: [u8; 64]` between `evaluation` and
    `dlog_proof` (`pallets/pallet-identity/src/lib.rs:1068-1075`), but `extrinsic.rs:70-75,93-100`
    encodes only four and omits it. Every extrinsic this node builds today is malformed. Ironically
    the needed value is already computed and discarded: `EvaluationResult.pk` is marked
    `#[allow(dead_code)]` with a comment saying it is "not submitted on-chain"
    (`wasm_host.rs:69-73`) — that comment is now wrong.

---

## Open questions

Genuinely unresolved; each would need real operational testing or a decision that isn't this
document's to make.

- **Nobody has ever run this container.** Everything above is designed against source code. `README.md`
  states no `docker buildx build` was ever executed, and no `submit_oprf_response` has reached a real
  chain. The first real deployment will find things this document doesn't anticipate.
- **Does requiring each operator to run their own full node break the cost/effort model?** It is the
  clean fix for the rogue-RPC problem, but it doubles the operational surface per operator. Whether a
  light client (smoldot, as the desktop app already intends) is sufficient here is untested.
- **Is single-instance-per-key compatible with the availability the SLA needs?** The 6-day
  `OprfQuerySlaBlocks` window (`runtime/src/configs/mod.rs:297`) combined with *n*-of-*n* across 5
  committees suggests it is — but a key-holding hot standby would double memory exposure, so the
  tradeoff is real and unmodelled. Note also that `#082` itself flags the SLA window as a placeholder,
  not a measured figure.
- **Is cloud KMS Ed25519 signing available on all three major providers?** Not confirmed here; verify
  per provider before relying on item 34.
- **Does `CommitteeMembers` registration accept an ed25519-derived `AccountId32`?** It should —
  `AccountId32` is scheme-agnostic — but this was not verified against the real registration path.
- **Can wasmtime's guest linear memory be reliably `mlock`ed or zeroed?** Its allocation strategy
  (whether pages are `mmap`ed lazily, and whether they can be swapped) needs real testing before item
  27 can claim to cover the share's largest in-memory footprint.
- **Can the `committee-node` binary actually be made bit-for-bit reproducible?** Needs an attempt. The
  full image almost certainly cannot be without significant work.
- **Will 5 independent institutions accept a staggered-update policy contractually, and who arbitrates
  "this is urgent enough to skip staggering"?** The Emergency Council is the obvious answer but it is
  not currently scoped to update advisories.
- **What is a committee's SLA obligation while it is mid-rotation?** `#073` defines the rotation
  trigger and scope but not the service expectation during the gap.
- **`#082`'s DKG-ceremony question is untouched here.** Everything above concerns steady-state query
  answering. Founding-ceremony key generation on cloud infrastructure is a different and harder
  problem — the share has to be *created* somewhere, and creating it inside a cloud VM inherits every
  memory-exposure concern in this document at the worst possible moment.

---

## Verdict

**The skeleton's architecture is already well-suited to cloud hardening.** A single-purpose,
non-root, no-listener, outbound-only process is the easy case, and most of the work in Tiers 0-2 is
operator configuration rather than code. The authors' documented instincts — prefer the passphrase
file over the env var, fail closed on stub crypto, fresh nonce every call, never log secrets — are
consistently right.

**The one genuinely inadequate piece is key custody — but not for the reason it looks like.** `age`
is not weak. The problem is that **every deployment example puts the passphrase on the same disk as
the ciphertext**, and in cloud, "snapshot the volume" is one API call available to the provider, the
backup system, and anyone with console credentials. Fixing that — passphrase from a secret manager
into `tmpfs`, never persisted — closes the largest single gap for a few hours of work and requires no
code change.

**The share is plaintext in RAM 24/7, and no realistic cloud measure changes that.** Confidential VMs
narrow the exposure meaningfully and cost nearly nothing, so enable them. TEE.Fail proves they do not
close it, and that their attestation is forgeable by exactly the adversary with physical access —
i.e. the provider. **Turn them on; don't believe them.** Never let "we run in a TEE" justify weakening
anything else.

**The real defense remains the 12-of-35 × 5-committee structure**, which was designed from the start
to survive an operator being fully compromised. Cloud hardening's job is to make single-node
compromise rare, detectable, and scoped to one committee — and the single highest-value thing this
document recommends is not a control on any node at all, but **diversity across the 5 committees**
in provider, region, jurisdiction and technology, so that no single CVE or single provider decision
takes more than one of them.

**On `#082`'s open question, the answer is: you don't remove the signing authority, you remove its
reach.** Keyless signing eliminates the standing key; pull-based, digest-pinned, staggered updates
eliminate the ability of whoever holds that authority to touch all 5 committees at once. That is what
makes the release pipeline a survivable target rather than a fatal one — and it is why the balenaCloud
fleet-push path, correct for citizen-owned devices, should be retired for the institutional model.

**And nothing here has been tested against a running container.** Treat this as a specification to
execute and then correct, not as a description of a system that works.

---

### Sources

- [TEE.Fail attack breaks confidential computing on Intel, AMD, NVIDIA CPUs — BleepingComputer](https://www.bleepingcomputer.com/news/security/teefail-attack-breaks-confidential-computing-on-intel-amd-nvidia-cpus/)
- [New TEE.Fail Side-Channel Attack Extracts Secrets from Intel and AMD DDR5 Secure Enclaves — The Hacker News](https://thehackernews.com/2025/10/new-teefail-side-channel-attack.html)
- [Intel security announcement 2025-10-28-001 (TEE.fail)](https://www.intel.com/content/www/us/en/security-center/announcement/intel-security-announcement-2025-10-28-001.html)
- [Confidential VMs Explained: An Empirical Analysis of AMD SEV-SNP and Intel TDX (ACM SIGMETRICS)](https://dl.acm.org/doi/10.1145/3700418)
- [Heckler: Breaking Confidential VMs with Malicious Interrupts](https://arxiv.org/pdf/2404.03387)
- [Lost in the Pages: WebAssembly Code Recovery through SEV-SNP's Exposed Address Space](https://arxiv.org/pdf/2512.14376)
- [Google Cloud Confidential VM pricing](https://cloud.google.com/confidential-computing/confidential-vm/pricing)
- [Google Cloud Confidential VM release notes](https://docs.cloud.google.com/confidential-computing/confidential-vm/docs/release-notes)
- [About Azure confidential VMs — Microsoft Learn](https://learn.microsoft.com/en-us/azure/confidential-computing/confidential-vm-overview)
- [AWS KMS cryptographic attestation for Nitro Enclaves](https://docs.aws.amazon.com/kms/latest/developerguide/services-nitro-enclaves.html)
- [Building secure, verifiable blockchain key management on AWS Nitro Enclaves at Turnkey](https://aws.amazon.com/blogs/web3/building-secure-verifiable-blockchain-key-management-on-aws-nitro-enclaves-at-turnkey/)
- [sigstore/cosign](https://github.com/sigstore/cosign)
- [SLSA: General availability of SLSA 3 Container Generator for GitHub Actions](https://slsa.dev/blog/2023/02/slsa-github-workflows-container-ga)
- [Reproducible builds with GitHub Actions — Docker Docs](https://docs.docker.com/build/ci/github-actions/reproducible-builds/)
- [Reproducible Builds — February 2026 report](https://reproducible-builds.org/reports/2026-02/)
- [Bit-for-bit reproducible builds with Dockerfile — Akihiro Suda](https://medium.com/nttlabs/bit-for-bit-reproducible-builds-with-dockerfile-7cc2b9faed9f)
