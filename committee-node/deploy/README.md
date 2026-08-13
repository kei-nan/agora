# Cloud Deployment Runbook — For Institutional Committee Operators

**Status note (2026-08-13): `committee-node`'s orchestration loop now implements the genuine
two-round threshold protocol** (`../README.md`'s "Option B" section) — `submit_oprf_round1`/
`submit_oprf_round2`, not the retired single-response `submit_oprf_response`. Deployment
mechanics below (hosting, hardening, secret handling) are unaffected and apply exactly as
written. **Two new required environment variables** the compose/config examples below predate:
`MEMBER_INDEX` (this node's 1-based `CommitteeMembers[slot]` roster position) and
`GROUP_PUBKEY_HEX` (this committee's group public key) — see `../README.md`'s "Option B"
section and `config.rs` for what these are and why getting them wrong fails silently. As
always: nothing here has been run against a real chain or a real multi-party exchange (no
chain is running in this environment, and no real committee exists yet) — see `../README.md`'s
own "still open" caveats before treating this as production-ready.

This is the operational counterpart to two research documents everyone running this should read
first:

- [`docs/project/research/oprf-alternatives/08-cloud-hosting-providers.md`](../../docs/project/research/oprf-alternatives/08-cloud-hosting-providers.md)
  — **which provider to use.** A menu, not a recommendation of one — pick something different
  from what your committee's other operators picked where you can. Read its "hard requirements
  any substitute must meet" section even if you're not using one of the listed providers.
- [`docs/project/research/oprf-alternatives/09-cloud-security-hardening.md`](../../docs/project/research/oprf-alternatives/09-cloud-security-hardening.md)
  — **why every step below exists.** This runbook operationalizes that document's Tier 0/1
  checklist; it doesn't replace reading the reasoning, especially the "Secrets and key-share
  custody" and "Update and patch governance" sections.

**What this covers**: steady-state operation of an already-provisioned committee member (polling
for queries, evaluating them, submitting responses). **What this does not cover**: the founding
DKG ceremony that generates your committee's key shares in the first place — that's a separate,
harder problem (09's own open questions flag this explicitly) and isn't solved here.

**Honesty note, matching the rest of this component**: every script here was authored and
reasoned through against the actual `committee-node` source and the research documents above, but
none of it has been run against a real cloud VM or a real chain. Treat this as a specification to
execute and correct, not a description of a tested system.

---

## Step 0 — Choose a provider and a role

Pick from `08-cloud-hosting-providers.md`'s shortlist, or something meeting its hard
requirements. In order of preference per that document: your own institution's existing
infrastructure first; otherwise a crypto-payable provider with real workload isolation (dedicated
vCPU, bare metal, or a confidential VM). Confirm before provisioning:

- [ ] Dedicated VM/instance, for this workload only — no co-tenancy with other institutional
      workloads (09 checklist item 1).
- [ ] A confidential-computing option (AMD SEV-SNP / Intel TDX), if your provider offers one —
      enable it at creation time; it can't be added after (09 checklist item 20).
- [ ] Debian or Ubuntu, since `harden-host.sh` below targets `apt`/`ufw`/`systemd`. Adapt it if
      your provider's base image differs.

## Step 1 — Set up out-of-band access *before* you lock down the network

`harden-host.sh` (next step) removes inbound access entirely, including SSH. Before running it,
confirm you have a way back into the machine that doesn't depend on an open port:

- Your provider's browser-based console / serial console ("Connect" button in most dashboards), or
- Identity-aware access that doesn't route through a routable port (AWS SSM Session Manager, GCP
  IAP TCP forwarding, Azure Bastion).

If neither is available and you genuinely need `sshd`, see 09's "Management access" section for
the fallback configuration (private-interface bind, key-only, source-IP allowlist) — but exhaust
the alternatives first.

## Step 2 — Harden the host

```bash
scp deploy/harden-host.sh you@your-vm:
ssh you@your-vm
sudo ./harden-host.sh
```

Then, **from a different machine**, verify nothing is reachable:

```bash
nmap -Pn <your-vm-ip>
```

This should report no open ports. If it does, stop and fix the firewall before going further —
do not proceed to mounting key material on a host with an open inbound port.

Separately, in your cloud provider's own console (not covered by the script):

- [ ] Delete the default SSH ingress rule from the VM's security group / firewall policy.
- [ ] Enable hardware-key MFA and least-privilege IAM on the account that can reach this VM (09
      checklist item 5 — with no SSH, cloud IAM is the primary way in).

## Step 3 — Decide how you'll supply the passphrase

Two options (09's "Secrets and key-share custody" section):

**A. Secret manager (recommended).** Requires your secret-manager CLI (`aws`, `gcloud`, `az`,
`vault`, ...) to be reachable from inside the container. Two ways to get it there:

- Extend the published image with a small additional layer:
  ```dockerfile
  FROM ghcr.io/CHANGE-ME/committee-node@sha256:CHANGE-ME
  USER root
  RUN apt-get update && apt-get install -y --no-install-recommends awscli && rm -rf /var/lib/apt/lists/*
  USER committee
  ```
  (swap `awscli` for whichever CLI your `SECRET_FETCH_CMD` needs), or
- Bind-mount a statically-linked CLI binary into the container instead of installing one.

Store the passphrase in your provider's secret manager now, under whatever name your
`SECRET_FETCH_CMD` will reference (see `deploy/fetch-secret-and-run.sh`'s header comment for
examples per provider).

**B. Manual.** Skip `SECRET_FETCH_CMD` and the `entrypoint:` override in the compose file, and set
`KEY_PASSPHRASE_FILE` to point at a file you place on a **tmpfs** mount by hand after each start.
Strongest against a compromised cloud account, worst for availability — a restart means the node
is down until someone is available to re-enter it. 09 recommends A for normal operation with B
documented as the fallback, given the 6-day query SLA and *n*-of-*n* combination across 5
committees.

Either way: **the passphrase must never land on a persistent volume.** `fetch-secret-and-run.sh`
refuses to run if its destination isn't tmpfs, specifically to catch this misconfiguration.

## Step 4 — Get your key material and the wasm module onto the host

```bash
mkdir -p keys wasm
# Copy your committee-secrets.age file into keys/ — how you get it there safely (out-of-band,
# not over a network path this document controls) is between you and your committee's founding
# ceremony process; not covered here.
# Copy the compiled oprf-crypto-core.wasm into wasm/ — see ../wasm/README.md.
```

## Step 5 — Configure and deploy

```bash
cd committee-node/deploy
cp .env.example .env
# edit .env: NODE_RPC_URL, COMMITTEE_SLOT, SECRET_FETCH_CMD
docker compose -f docker-compose.prod.yml up -d
docker compose -f docker-compose.prod.yml logs -f
```

## Step 6 — Verify startup, every time

Check the logs for, in order:

1. `"loaded real OPRF crypto-core Wasm module"` — if you instead see a STUB-mode warning, the
   wasm module wasn't found at the mounted path. **Stop and fix this before anything else**; a
   stub node that somehow submitted would poison the chain (it won't by default —
   `ALLOW_STUB_SUBMISSION` defaults false — but don't rely on that alone).
2. `"confirmed: this account is on the CommitteeMembers roster"` — if absent, this node's
   responses will be rejected on-chain; check `COMMITTEE_SLOT` and that your account is actually
   registered.
3. Normal poll-cycle activity at your configured `POLL_INTERVAL_SECS`.

```bash
findmnt /run/secrets   # must show tmpfs — confirms the passphrase never touched persistent disk
```

## Step 7 — Monitoring

Ship the container's JSON logs to your provider's log service (outbound only — do not add a
`/metrics` listener, see 09's "Implementation caution" note) and alert on:

- No successful poll cycle within 2× `POLL_INTERVAL_SECS`.
- The STUB-mode warning appearing at all.
- The "NOT on the CommitteeMembers roster" warning appearing at all.
- Repeated `author_submitExtrinsic failed` errors.
- Any snapshot or image-export of this VM's disk in your cloud provider's audit log — this is
  the primary theft path (09's "Signals that indicate compromise" section).
- Any secret-manager access to the passphrase from a principal or time you didn't expect.

## Step 8 — Know the incident procedure before you need it

Full detail in 09's "How a cloud incident feeds the existing rotation governance" section. The
short version, which should be a laminated card, not a wiki page nobody reads under pressure:

1. **You can act alone, immediately, without waiting for governance approval**: stop the
   container; revoke the secret-manager grant (this is your fastest real kill switch — it works
   even if you've lost control of the VM); image the disk for forensics before destroying
   anything.
2. **Publish an incident report.** Its hash is what both governance arms (Emergency Council +
   Courts Oracle) reference.
3. **Any root-level compromise, unexplained snapshot, or provider-side incident must be treated
   as full share disclosure.** The share lives in plaintext process memory continuously — there
   is no partial-compromise story for a key in RAM. State this to yourself in advance; don't
   negotiate it during an actual incident.
4. This rotates *your* committee only, not all five — that's the whole point of the per-committee
   scoping. The other four keep operating.

## Update policy — read before your first update

**Pull-based only.** You apply updates on your own schedule, never automatically. Nothing in this
deployment auto-pulls a new image — if you add a Watchtower-style auto-updater, you are
personally recreating the single-point-of-compromise problem the 5-committee structure exists to
prevent. Don't.

When a new release is published: verify its cosign signature and provenance attestation, pin the
new digest in `docker-compose.prod.yml`, apply in a maintenance window, re-run Step 6's
verification, and keep the previous digest noted for immediate rollback. Releases are labeled
`routine` / `security` / `breaking` — see 09's "What Agora should publish per release" section.
Committees are asked (not forced) to stagger adoption so no more than 2 of 5 update within the
same 72 hours, except for anything the Emergency Council flags as urgent.

---

## What's still not solved here (tracked in 09's checklist, not this runbook)

- The release pipeline does not yet publish a signed image for `committee-node` at all — the
  `image:` line above has a placeholder digest for a reason. This is Agora's job (09 Tier 3, items
  32), not something an operator can work around.
- The RPC-endpoint trust boundary (09's largest network finding): running your own full node
  instead of pointing at a third-party RPC closes it. Strongly recommended, not yet made
  mandatory by anything in this repo.
- Graceful `SIGTERM` handling, `zeroize`-on-drop for the secret material in memory, and persisting
  `already_responded` across restarts are real, identified gaps in `committee-node` itself (09
  Tier 3, items 27, 30, 33) — upstream code changes, not deployment configuration, and not
  addressed by anything in this directory.
