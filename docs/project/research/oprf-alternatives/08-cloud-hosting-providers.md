# Cloud Hosting Providers for Committee Nodes — A Menu, Not a Vendor

*Research note, 2026-08-12. Companion to [07-treasury-funded-infrastructure.md](07-treasury-funded-infrastructure.md)
(who pays) and the institutional-operator recommendation in [00-index.md](00-index.md) (who runs it).
This note answers: **where does the container actually run, and who buys the server?***

**Scope.** The workload is `committee-node/` — a small Rust binary shipped as a Docker image
(linux/amd64 + linux/arm64) that polls a chain RPC endpoint for pending OPRF queries, decrypts an
`age`-encrypted secret-share file, calls a `wasmtime`-loaded crypto core, and submits signed
extrinsics. It is **outbound-only** (no listening ports), nearly idle, and needs on the order of
1–2 vCPU / 2 GB RAM / 20 GB disk. Cost is not the interesting variable here — **trust topology is.**

**Everything below is a menu handed to each institutional operator.** Agora does not pick one and
provision 40–75 accounts on it. Section 1 explains why that distinction is the whole point.

---

## Why this must be a menu, not a single provider

The committee design already paid a large price for distributed trust: 5 independent committees of
~8–15 named institutions each, deliberately spread across jurisdictions, with a threshold (think
12-of-35) so that compromising the system requires compromising twelve independent parties. If Agora
then puts all of those operators on one cloud account it controls, **the threshold silently becomes
1-of-1** and every rand of that design cost is refunded to the attacker.

Concretely, a shared Agora-controlled hosting account re-centralises on seven separate axes:

1. **Key material.** Console/API access to the account is remote code execution on every node. An
   attacker who phishes one Agora cloud admin gets all 35 shares at once — including the passphrase
   injection mechanism. The 12-of-35 threshold is bypassed, not attacked.
2. **Legal process.** One provider means one company, in one country, receiving one court order,
   subpoena, or national-security letter that reaches every share simultaneously. Jurisdictional
   diversity of *institutions* is decorative if all their servers sit in one vendor's estate under one
   legal system.
3. **Coercion and suspension.** Cloud accounts get suspended for billing disputes, sanctions
   screening, and acceptable-use complaints, generally without due process and sometimes without
   notice. A single AUP action would take a national identity system offline.
4. **The payment relationship is itself control.** If Agora pays the bill, Agora can stop paying —
   an off-chain kill switch over a committee whose entire value proposition is that Agora *cannot*
   control it. It also makes operator "independence" legally fictional: an entity whose infrastructure
   you provision and fund looks, to a court, a regulator, or a journalist, like your contractor.
5. **Correlated failure.** Same provider means the same outage, the same hypervisor CVE, the same
   BGP incident, the same misconfigured default. The reason to prefer institutions over citizen phones
   in the first place was an always-on availability bar; correlated failure across the whole committee
   gives that back.
6. **Update authority.** Changelog #082 already flags an unsolved question — who may author an image
   update without becoming a new supply-chain single point of compromise. A shared account makes this
   strictly worse: account admin *is* unilateral update authority, with no separate signing key to
   even argue about.
7. **Legitimacy.** For a system a real government is meant to adopt, "who hosts the identity
   committee" is a political question with a political answer. "Each named institution, on
   infrastructure it owns and pays for, declared in the public register" survives scrutiny.
   "All of them, on the project's cloud account" does not.

### The quieter failure mode: default convergence

A menu is necessary but not sufficient. If 30 of 35 members independently pick the cheapest well-known
option, you have re-centralised by accident. The fix is a **concentration cap that is checkable**, and
the sharp version of it falls straight out of the threshold:

> **No single hosting provider, and no single legal jurisdiction, may account for a number of members
> of any one committee equal to or greater than that committee's reconstruction threshold.**

With 35 members and a threshold of 12, the hard ceiling is 11 per provider — but 11 is far too close
to the line to be prudent. A sane operational target is **≤ 5 members per committee on any one
provider, and ≤ 8 in any one country**, with each member's provider and jurisdiction declared in the
on-chain committee register so the concentration is publicly auditable rather than discovered after an
incident. (Whether that cap is enforced on-chain at registration or only socially is an open question
below.)

---

## Traditional crypto-paying VPS/dedicated providers

Verified against provider documentation where possible in August 2026. Crypto-payment support in this
market changes without announcement — **every operator should re-check at purchase time.**

Note on what "accepts crypto" means here: nearly all of these route through a payment processor
(BitPay, CoinGate, Coinbase Commerce) rather than holding coins themselves. That's fine — it's a
direct merchant integration, not a reseller you'd have to separately trust with your account — but it
does mean the processor's own compliance rules, not just the host's, govern large payments.

### Cherry Servers — Lithuania

Lithuanian IaaS provider, founded 2002. Accepts crypto directly via **CoinGate** (BTC, ETH, SOL, LTC,
**USDC**, TRON, Polygon and others) alongside cards, SEPA, PayPal. The important differentiator: it
sells **genuine single-tenant bare metal** (from roughly $66/mo, hourly billing available) as well as
cloud VDS from around $6/mo — so an operator can get real hardware isolation without leaving the
crypto-payable set. Data centres in Lithuania, Netherlands, Germany, Sweden, USA and Singapore, which
also lets *different* operators pick different countries within one familiar vendor. EU/GDPR
jurisdiction. Reputation is solid and unremarkable — a boring 20-year-old hosting company, which for
this purpose is a compliment.

### Vultr — United States

Large, mature provider (The Constant Company), 30+ global regions. Vultr's own billing documentation
confirms crypto payment **via BitPay**: BTC, BCH, ETH, DOGE, LTC, **USDC**, PAX, BUSD, GUSD. Offers a
proper isolation ladder: shared-vCPU Cloud Compute (avoid), **Optimized Cloud Compute with dedicated
vCPUs** (no noisy neighbours), and **single-tenant Bare Metal**. Crypto here is a prepaid account
top-up, not per-invoice anonymity — BitPay is a US-regulated processor and payments above thresholds
are subject to its own identity rules. US domicile is a real jurisdictional fact to weigh, not a
disqualifier: it is a *different* jurisdiction from the European options, which is the point. It does
mean US sanctions posture applies, so operators in sanctioned or sanctions-adjacent countries should
assume this option is closed to them.

### Hostinger — Lithuania

Among the few genuinely mass-market hosts accepting crypto **directly at checkout**, via CoinGate —
marketed as 70+ assets including BTC, ETH, LTC, DOGE. Hostinger International Ltd is based in Kaunas,
Lithuania (EU). KVM VPS with root access. The caveat is product fit, not payment: the value tiers are
budget shared-host territory, and Hostinger's centre of gravity is website hosting, not
infrastructure-as-a-service for long-running daemons. Usable, but an operator choosing it should buy a
tier with guaranteed (not burstable) CPU and should not expect infrastructure-grade support.

### 1984 Hosting — Iceland

`1984 ehf`, Reykjavík. Accepts **Bitcoin and Monero** and says so on the front page. ISO 27001
certified, geothermal/hydro powered, and explicitly founded on a civil-liberties premise. Entry VPS is
1 vCPU / 1 GB / 25 GB NVMe at about $9.66/mo — thin but genuinely sufficient for this workload. Iceland
is a meaningfully distinct legal jurisdiction with strong press- and speech-protection traditions,
which is exactly the kind of diversity the committee design wants. Downsides: small company, small
plans, no bare metal in the entry range, and the usual question of whether a micro-provider is a safe
bet on a ten-year government horizon.

### FlokiNET — Iceland (with Romania, Netherlands, Finland)

Established 2012, self-described safe harbour for press and whistleblower projects. Accepts BTC,
**Monero**, LTC, DASH, ETH, plus bank transfer, PayPal and even cash by post. **Signup requires only a
working email address — no KYC.** Offers VPS, dedicated servers, GPU and colocation across four
countries today (Norway, Canada, Singapore advertised as coming), which is unusual: one vendor
relationship, four jurisdictions, so two operators using FlokiNET in different countries are less
correlated than two using the same vendor in one. Downsides: small operator; and the
takedown-resistant-hosting brand is a political adjacency a government-facing identity committee may or
may not want. That's a policy call, not a technical one.

### OrangeWebsite — Iceland

Iceland-domiciled since 2009, accepts BTC, LTC, ETH, BCH, **XMR**, advertises 99.9% network uptime and
anonymous registration, 100% renewable power. Technically a reasonable peer of the two above. However,
public review sentiment is more mixed than 1984's, including recurring complaints about historical
tolerance of abusive content on the platform. For a workload whose entire product is public trust,
neighbourhood reputation is a legitimate selection criterion. Listed for completeness; not shortlisted.

### Njalla — Nevis / Sweden

Privacy service founded by Peter Sunde; legally structured as 1337 LLC in Nevis with operations linked
to Sweden. Accepts BTC, LTC, **XMR**, BCH, DASH, ETH into a prepaid balance, and its terms state KYC
will never be requested — signup needs only an email or XMPP address. As of 2026 its **VPS product is
available only in Sweden**, which caps how much diversity it can contribute. Best understood as a
one-or-two-members-per-committee option for an operator that specifically wants the Nevis/Sweden split
and hard no-KYC guarantees, not as a mainstream choice.

### Checked and *not* recommended for crypto-paying operators

- **Hetzner (Germany)** — excellent price/performance and reputation, but **does not accept
  cryptocurrency**, a position apparently unchanged since around 2021. The only crypto routes are
  resellers and crypto-funded virtual cards, both of which insert a third party between the operator
  and the server — precisely the laundered-through-a-reseller pattern to avoid. **Perfectly fine for a
  fiat-paying operator**, and a genuinely good choice there.
- **OVHcloud (France)** and **IONOS (Germany)** — same story: no direct crypto, workarounds only via
  virtual-card services. Both are large, sovereign-EU-friendly, and entirely appropriate for
  fiat-paying operators (OVHcloud in particular is a common choice for European public-sector work).
- **Contabo (Germany)** — **does not accept crypto** (EUR/USD/GBP only), and carries a persistent
  reputation for oversold, congested hardware. Fails both the payment test and the isolation test.

---

## Decentralized / Web3-native compute

### Akash Network

The thematically obvious candidate: a Cosmos-based marketplace where tenants post a Stack Definition
Language (SDL) manifest, providers bid, and the lowest bid wins a lease. It runs **arbitrary Docker
containers** (Kubernetes-orchestrated), supports persistent storage, and tenants can **pay in AKT or
USDC** without first acquiring the native token. Prices run roughly 70–85% below hyperscalers.

The 2026 reality check is less flattering, and it matters here:

- **Provider count is at a record low, not a record high.** Messari's *State of Akash Q1 2026* puts
  average active providers at **58**, down 8.4% from 63 in Q4 2025 and 69 a year earlier — the lowest
  in the network's recent history, with capacity contracting across all four resource categories.
  Other 2026 write-ups quote 73, and some quote "200–300"; treat the higher figures as counting
  registered rather than active providers. Either way, "decentralized" here means *tens* of operators,
  not thousands.
- **The provider sees everything.** There is no encryption at the compute layer today: whoever wins
  your lease has host-level visibility into the container that holds your decrypted share. For a
  key-custody workload that is the central objection. Confidential computing (Kata Containers) and
  trusted-execution hardware verification are on the **2026 roadmap** with May/July 2026 target dates —
  **I could not verify that either has actually shipped.**
- **Lease economics create an availability risk.** Deployments run against an escrow balance; when it
  drains, the lease closes. A mostly-idle, always-critical service is exactly the kind of thing that
  quietly dies from an unfunded escrow.
- **No SLA, variable provider quality**, and the network's own centralising moves (Starcluster /
  Starbonds protocol-owned compute in enterprise datacentres with "vetted Nodekeepers"; AkashML as a
  managed, centrally-APIed service) cut against the sovereignty argument.
- A ChainLight-reported vulnerability allowing unauthorised deployment access has been **patched**;
  worth knowing it existed, not disqualifying.
- **Bid-selection is a convergence trap.** If every operator takes the cheapest bid, many land on the
  same handful of large providers — recentralisation wearing a decentralisation badge. Any operator
  using Akash for this should pin a specific, named provider, not accept the market default.

**Assessment:** worth a real pilot and worth having 1–2 members per committee on, for genuine
architectural diversity and thematic coherence. Not defensible as the default for a majority of
operators in 2026.

### Fluence

Decentralized cloud offering VMs across independent providers, provisioned in compute units of 2 vCPU
/ 4 GB / 25 GB at roughly $10.78/mo, with a single API, public fixed pricing and no egress fees; GPU
compute launched in late 2025 with Spheron as a supply partner. Sizing and price are a good fit for
this workload. I could not verify its legal domicile, provider count, or operational track record to
the same depth as Akash — **treat as unvetted**, worth an evaluation rather than a recommendation.

### Flux (RunOnFlux)

The largest of these by raw footprint: **15,000+ FluxNodes across 67 countries**, 111,500+ cores,
running Dockerized apps under FluxOS. Genuine geographic spread. But the nodes are consumer/hobbyist
operated with no confidentiality guarantees and no per-node accountability of the kind an institutional
committee register wants — the operator would be handing a secret share to an anonymous node runner.
Good technology, wrong trust model for this specific payload.

### Golem

Docker-centric but batch/HPC oriented: images must be converted into Golem VM images, and the
scheduling model targets finite jobs (rendering, simulation) rather than a permanently-resident
polling daemon. Poor fit; not recommended.

---

## Confidential computing / isolation options

This workload holds a threshold key share. `committee-node`'s `keys/README.md` is honest that `age`
encryption at rest is "a modest speed bump," not tamper-resistant storage, and changelog #082 leaves
hardware-backed key custody explicitly unsolved for member-hosted devices — stock Raspberry Pi has no
secure boot or hardware root of trust. **The move to institutional hosting quietly opens a better
answer to that open problem than the Pi ever had**, and that's a finding worth surfacing beyond this
document.

- **AMD SEV-SNP and Intel TDX** are both shipping in production on Azure, AWS and Google Cloud. They
  encrypt guest memory and register state with keys the hypervisor never holds, detect tampering at
  access time, and support remote attestation of exactly what code is running — i.e. they remove the
  *host operator* from the trusted computing base. SEV-SNP is the more mature option for CPU-only
  workloads with lower overhead; TDX is more often chosen for government/high-assurance use. (One
  operational note: Google has flagged that from **August 2026**, SEV-SNP Confidential VMs may see
  longer boot times and performance changes during a guest-kernel migration, expected resolved by
  November 2026.)
- **The catch: none of the hyperscalers accept crypto.** Confidential computing and crypto payment are
  largely disjoint sets today. A fiat-paying institutional operator can have hardware isolation on
  Azure/GCP/AWS immediately; a crypto-paying one mostly cannot — with one exception:
- **Phala Cloud** is the exception, and the most interesting single finding in this research. It
  deploys **arbitrary Docker / docker-compose workloads into confidential VMs** on Intel TDX, Intel
  SGX, AMD SEV (and TEE-enabled NVIDIA H100/H200 for GPU work), with **secrets encrypted client-side
  and decrypted only inside the CVM at boot**, plus emitted runtime measurements for independent
  verification — an almost exact match for how `committee-node` handles its `age`-encrypted share and
  passphrase. Payment accepts **crypto via Coinbase Commerce** (ERC20 on Base, settling to USDC),
  Stripe cards, or wire for business accounts. Caveats: it is the youngest option here, account
  verification involves a $1 card authorisation (so it is not a no-KYC path), and "decentralized
  confidential computing" today still leans on Phala-operated capacity — so it should be a
  one-or-two-members-per-committee slot, not a committee-wide standard.
- **Akash's** confidential-computing roadmap is noted above and unverified.
- **Baseline expectation if confidential computing isn't available:** dedicated vCPU or single-tenant
  bare metal, full-disk encryption, and passphrase injection that does not leave the passphrase sitting
  in a provider dashboard as a plaintext environment variable.

---

## Comparison table

| Provider | Crypto accepted | KYC | Jurisdiction | Isolation available | Reputation notes |
|---|---|---|---|---|---|
| **Cherry Servers** | BTC, ETH, SOL, LTC, **USDC**, TRX, MATIC (CoinGate) | Standard billing; no special KYC found | Lithuania (EU); DCs LT/NL/DE/SE/US/SG | **Single-tenant bare metal**, cloud VDS | Established 2002, unremarkable in the good way |
| **Vultr** | BTC, BCH, ETH, DOGE, LTC, **USDC**, PAX, BUSD, GUSD (BitPay) | Processor-level rules apply on larger payments | USA | **Dedicated vCPU + bare metal** | Large, mature, 30+ regions; US sanctions posture applies |
| **Hostinger** | 70+ assets incl. BTC/ETH/LTC/DOGE (CoinGate) | Standard billing | Lithuania (EU) | KVM VPS; guaranteed-CPU tiers only | Mass-market, budget; IaaS is not its core business |
| **1984 Hosting** | **BTC, XMR** only | Minimal | **Iceland** | Small VPS (shared vCPU at entry tier) | ISO 27001, civil-liberties founding premise, geothermal; small |
| **FlokiNET** | BTC, **XMR**, LTC, DASH, ETH | **None — email only** | **Iceland + RO/NL/FI** | VPS, dedicated, colocation | Small; free-press/whistleblower brand adjacency |
| **OrangeWebsite** | BTC, LTC, ETH, BCH, **XMR** | Minimal | **Iceland** | VPS, dedicated | Mixed public sentiment incl. past content-policy complaints |
| **Njalla** | BTC, LTC, **XMR**, BCH, DASH, ETH (prepaid) | **Explicitly never** | **Nevis** (ops: Sweden) | VPS, **Sweden only** | Strong privacy guarantees; single location, small |
| **Hetzner** | **None** (resellers only) | Standard, can be strict | Germany (EU) | Dedicated + bare metal | Excellent value/reputation — **fiat operators only** |
| **OVHcloud / IONOS** | **None** (virtual cards only) | Standard | France / Germany | Full range incl. bare metal | Large EU incumbents — **fiat operators only** |
| **Contabo** | **None** | Standard | Germany | Oversubscribed VPS | Persistent overselling/congestion complaints — avoid |
| **Akash Network** | **AKT, USDC natively** | None | Protocol (Cosmos); providers global, US-founded core | Container on provider's host; **provider sees all**; CVM on roadmap only | ~58 avg active providers Q1 2026 (record low), no SLA, patched ChainLight CVE |
| **Fluence** | Native token / crypto | Unverified | **Unverified** | VM (2 vCPU/4 GB units) | Good sizing/price fit; track record unvetted |
| **Flux** | FLUX | None | 15k+ nodes, 67 countries | Docker on consumer-run nodes; no confidentiality | Large footprint, anonymous node runners — wrong trust model |
| **Phala Cloud** | Crypto via Coinbase Commerce → **USDC**; also cards | $1 card auth (light) | Vendor-operated TEE capacity, global | **Intel TDX / SGX / AMD SEV confidential VMs** | Youngest option; best isolation story available with crypto payment |
| **Azure / GCP / AWS** | **None** | Full corporate onboarding | US (with regional DCs) | **SEV-SNP / TDX / Nitro Enclaves** | Fiat operators wanting maximum isolation; heaviest centralisation optics |
| **Institution's own DC** | n/a | n/a | Operator's own | Whatever it already runs | **Maximal independence** — see below |

---

## Recommended shortlist and hard requirements for any substitute

### The shortlist to hand an operator

Present these as *examples that clear the bar*, with an explicit instruction to pick something
**different from what their peers picked** where possible.

1. **The institution's own infrastructure.** Named universities, election-monitoring bodies and NGOs
   frequently already run a datacentre, a virtualisation cluster, or capacity on a national research
   network. This is the **strongest** option in the menu: zero third-party trust, jurisdiction
   determined by the institution itself, no payment relationship to compromise, and a treasury stipend
   that simply offsets internal cost. It should be listed first, not as an afterthought.
2. **Cherry Servers (Lithuania)** — the best "boring, isolated, crypto-payable" pick: real bare metal,
   USDC accepted, EU jurisdiction, multiple DC countries, 20+ year operating history.
3. **Vultr (USA)** — the best non-EU commercial pick, with dedicated-vCPU and single-tenant bare metal
   tiers, USDC via BitPay, and a large operational track record.
4. **1984 Hosting (Iceland)** or **FlokiNET (Iceland/Romania/Netherlands/Finland)** — the
   jurisdictionally distinct, Monero-accepting, minimal-KYC picks. 1984 for ISO-27001 formality;
   FlokiNET for per-country choice and no-KYC signup. Ideal for one or two members per committee.
5. **Phala Cloud** — the confidential-computing slot. Take this if hardware isolation from the host
   matters more than vendor maturity, and cap it at one or two members per committee.
6. **Akash Network** — the decentralized-compute slot, as a deliberate pilot with a *pinned, named
   provider* rather than a market-default bid. One or two members per committee. Revisit if
   confidential computing actually ships.
7. **Fiat-paying operators:** Hetzner, OVHcloud, IONOS, or a hyperscaler confidential VM are all
   entirely acceptable and add diversity. Paying in fiat is not a downgrade — the crypto option exists
   only as a convenience for operators who would rather not off-ramp their stipend.

On the stipend-currency nicety: **no provider accepts AGR**, and none realistically will. If the goal
is "pay the hosting bill in the currency the stipend arrives in," the practical answer is to settle
stipends in **USDC**, which is accepted by Cherry Servers (CoinGate), Vultr (BitPay), Akash (natively)
and Phala (via Coinbase Commerce). That is a nice-to-have, not a design constraint.

### Hard requirements any substitute must meet

An operator may choose anything not on this list, provided it satisfies all of the following:

1. **Runs an arbitrary OCI/Docker container** (linux/amd64 or linux/arm64), long-running, with
   restart-on-failure — not a PaaS restricted to supported runtimes, not a function/batch platform.
2. **Outbound-only networking is sufficient.** `committee-node` opens no listening ports. Default-deny
   inbound is acceptable and preferred; a provider that requires exposing a public port is a worse fit,
   not a better one.
3. **Workload isolation: dedicated vCPU, single-tenant bare metal, or a confidential VM.** Burstable
   or oversubscribed shared-vCPU plans are excluded — not for performance, but because co-tenancy is a
   side-channel surface for a process that decrypts key material into memory.
4. **The account is owned, controlled and paid for solely by the operating institution.** No shared
   credentials with other operators, no Agora-held root/console access, no Agora-held payment
   instrument, no Agora-managed organisation or sub-account. This is non-negotiable.
5. **Persistent encrypted storage** for the `age`-encrypted share, plus a passphrase injection path
   that is not a plaintext field in a dashboard other parties can read.
6. **A credible availability record**: a published ≥99.9% infrastructure SLA *or* a multi-year public
   track record, plus the operator's own commitment to restore service within the committee's agreed
   response window.
7. **Provider and jurisdiction are declared in the on-chain committee register**, so the concentration
   caps in section 1 are publicly auditable rather than aspirational.
8. **No third party gains root or update authority** as a condition of the hosting arrangement — which
   also rules out managed-hosting arrangements where the vendor administers the container.

---

## Open questions

- **Did Akash's confidential computing actually ship?** The 2026 roadmap targets trusted-execution
  hardware verification (May 2026) and Kata Containers confidential compute (July 2026). Unverified as
  of this writing; it materially changes Akash's suitability if it landed.
- **Do BitPay / CoinGate / Coinbase Commerce impose payer-side KYC above thresholds?** Not verified.
  Relevant only to the optional pay-in-stipend-currency convenience, but an operator planning on it
  should confirm before committing.
- **Can the provider/jurisdiction concentration cap be enforced on-chain** at committee registration
  (e.g. a declared-provider field with a cap check in `pallet-identity`), or only socially through the
  register plus review? Worth a design pass — a socially-enforced cap will drift.
- **Does `committee-node`'s arm64 image actually build?** Its own README marks this **unverified** (no
  Docker daemon was available). This gates cheap ARM instances (Hetzner ARM, Oracle Ampere, AWS
  Graviton) and the Raspberry Pi path alike.
- **Ten-year durability of the micro-providers.** 1984, FlokiNET, OrangeWebsite and Njalla are small
  companies. A government identity system has a much longer horizon than most of their customers do.
- **Reputational adjacency policy.** Some of the strongest jurisdictional-diversity options market
  themselves on takedown resistance. Whether a state-facing identity committee should sit there is a
  political question this document cannot settle — but it should be settled deliberately, not by
  accident.
- **Sanctions exclusion.** US-domiciled providers and US-domiciled payment processors (Vultr/BitPay,
  Phala/Coinbase Commerce) are closed to operators in sanctioned jurisdictions. This constrains which
  members can use which options — and is itself a further argument for the menu.
- **Pricing and crypto-support volatility.** Every figure and payment claim here is an August 2026
  snapshot. This market changes support quietly; re-verify at purchase.

---

## Verdict

**Publish a menu with hard requirements and concentration caps. Do not select a vendor, and never hold
the payment instrument.**

The hosting question is not primarily a procurement question — it is a continuation of the same trust
argument that produced 5 committees of named institutions across jurisdictions in the first place. A
shared Agora-controlled cloud account would collapse a 12-of-35 threshold into a single admin
credential, hand one legal system reach over every share, and give Agora an off-chain kill switch over
a body specifically designed to be independent of it. That trade is strictly worse than the
member-owned-Raspberry-Pi model it would replace, despite looking more professional.

The good news is that the menu is genuinely available in 2026. Real single-tenant hardware is
purchasable with USDC in the EU (Cherry Servers) and the US (Vultr); Iceland and Nevis provide
jurisdictionally distinct, Monero-accepting, minimal-KYC options for operators who want them; hardware
isolation from the host is available with crypto payment via Phala's confidential VMs; and Akash gives
one or two members per committee a genuinely decentralized architectural alternative — though at 58
average active providers, no SLA, and no shipped compute-layer encryption, it is a pilot slot in 2026,
not a default.

The single best option, and the one to list first, is the least exotic: **let each institution run it
on infrastructure it already owns.** A university hosting the node in its own datacentre, funded by a
treasury stipend it receives rather than a bill Agora pays, is the maximal expression of exactly the
independence the whole design is buying.

---

## Sources

- [Vultr — What Payment Methods Do You Accept? (official docs)](https://docs.vultr.com/support/platform/billing/what-payment-methods-do-you-accept)
- [Vultr — Bare Metal single-tenant dedicated servers](https://www.vultr.com/products/bare-metal/)
- [Cherry Servers — Buy dedicated server with crypto](https://www.cherryservers.com/bitcoin-dedicated-server)
- [Cherry Servers — Bare metal dedicated servers](https://www.cherryservers.com/bare-metal-dedicated-servers)
- [Cherry Servers — Does Hetzner accept crypto?](https://www.cherryservers.com/blog/does-hetzner-accept-crypto)
- [1984 Hosting (official site)](https://1984.hosting/)
- [FlokiNET (official site)](https://flokinet.is/)
- [CoinGate — Web hosting services that accept crypto (2026)](https://coingate.com/blog/post/web-hosting-accept-bitcoin-crypto)
- [CoinGate — Best VPS hosting providers in 2026 (PayPal, Crypto)](https://coingate.com/blog/post/best-vps-hosting-providers)
- [HostAdvice — Contabo review 2026](https://hostadvice.com/hosting-company/contabo-reviews/)
- [Cunicula — Njalla private VPS hosting review](https://cunicula.com/en/provider/njalla)
- [KYCnot.me — Njalla no-KYC review](https://kycnot.me/service/njalla)
- [WebsitePlanet — OrangeWebsite review 2026](https://www.websiteplanet.com/web-hosting/orangewebsite/)
- [Messari — State of Akash Q1 2026](https://messari.io/report/state-of-akash-q1-2026-final)
- [Akash Network — 2026 roadmap](https://akash.network/roadmap/2026/)
- [Akash Network — Providers & Leases (docs)](https://akash.network/docs/learn/core-concepts/providers-leases/)
- [Own Your Mind — Akash Network review 2026 (AKT, BME burn, Cosmos migration)](https://ownyourmind.ai/projects/akash/)
- [Fluence — Complete guide to decentralized cloud computing (2026)](https://www.fluence.network/blog/decentralized-cloud-computing-guide/)
- [RunOnFlux — Flux against the world: state of compute networks 2026](https://runonflux.com/flux-against-the-world-state-of-compute-networks-2026/)
- [Phala — Confidential VM / private cloud computing with TEE](https://phala.com/confidential-vm)
- [Phala Cloud — Billing](https://cloud.phala.network/about/billing)
- [Phala Cloud — Documentation overview](https://docs.phala.com/network/overview/phala-network)
- [Google Cloud — Confidential VM release notes](https://docs.cloud.google.com/confidential-computing/confidential-vm/docs/release-notes)
- [ACM — Confidential VMs explained: an empirical analysis of AMD SEV-SNP and Intel TDX](https://dl.acm.org/doi/10.1145/3700418)
- [arXiv — AMD SEV-SNP: a confidential computing primer](https://arxiv.org/html/2608.04039v1)
- [Wikipedia — Hostinger](https://en.wikipedia.org/wiki/Hostinger)
