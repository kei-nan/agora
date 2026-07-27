// Stub responses for every Tauri command.
// Used when running in a browser (not the native Tauri window) — e.g. `npm run dev`.

export const MOCKS: Record<string, (...args: unknown[]) => unknown> = {
  chain_status: () => ({ best: 12345, finalized: 12340 }),

  fetch_proposals: () => [
    {
      id: "prop-1",
      title: "Reduce income tax for households below median wage",
      status: "active",
      proposer: "0xabcd…1234",
      votesFor: 142800,
      votesAgainst: 31200,
      endsAt: Math.floor(Date.now() / 1000) + 60 * 60 * 24 * 3,
      ipfsHash: "QmStubProposal1",
      summary:
        "This proposal lowers the base income tax rate by 2 percentage points for citizens earning below the national median wage, effective from the next fiscal quarter.",
    },
    {
      id: "prop-2",
      title: "Establish public broadband infrastructure fund",
      status: "pending",
      proposer: "0xef01…5678",
      votesFor: 0,
      votesAgainst: 0,
      endsAt: Math.floor(Date.now() / 1000) + 60 * 60 * 24 * 10,
      ipfsHash: "QmStubProposal2",
      summary:
        "Allocates 120M from the infrastructure budget to build fibre broadband in rural areas currently underserved by private providers.",
    },
  ],

  fetch_laws: () => [
    {
      id: "law-1",
      title: "Digital Rights and Privacy Act",
      tier: "constitutional",
      version: 3,
      enactedAt: Math.floor(Date.now() / 1000) - 60 * 60 * 24 * 90,
      ipfsHash: "QmStubLaw1",
      summary:
        "Guarantees citizens the right to encrypted communication, prohibits warrantless data collection, and establishes an independent Digital Rights Tribunal.",
    },
    {
      id: "law-2",
      title: "Open Budget Transparency Act",
      tier: "ordinary",
      version: 1,
      enactedAt: Math.floor(Date.now() / 1000) - 60 * 60 * 24 * 30,
      ipfsHash: "QmStubLaw2",
      summary:
        "Requires all government expenditures above 10,000 to be published on-chain within 48 hours of approval, with department-level tagging.",
    },
  ],

  fetch_treasury: () => [
    {
      id: "tx-1",
      department: "Infrastructure",
      amount: "4,200,000",
      currency: "USD",
      description: "Q2 road maintenance contracts — northern region",
      timestamp: Math.floor(Date.now() / 1000) - 3600,
      ipfsHash: "QmStubTx1",
    },
    {
      id: "tx-2",
      department: "Education",
      amount: "890,000",
      currency: "USD",
      description: "School tablet programme — cohort 3 device procurement",
      timestamp: Math.floor(Date.now() / 1000) - 7200,
      ipfsHash: "QmStubTx2",
    },
  ],

  fetch_rulings: () => [
    {
      id: "ruling-1",
      caseTitle: "Re: Surveillance Amendment Act — constitutionality challenge",
      level: 0,
      outcome: "overturned",
      summary:
        "The AI judge found that Section 4(b) of the Surveillance Amendment Act violates Article 12 of the Digital Rights and Privacy Act by authorising bulk metadata retention without judicial oversight. The section is automatically suspended pending legislative revision.",
      ipfsHash: "QmStubRuling1",
      timestamp: Math.floor(Date.now() / 1000) - 86400,
    },
  ],

  fetch_anticorruption_data: () => ({
    assetDisclosures: [
      {
        account: "0x1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b",
        ipfsHash: "QmStubDisclosure1",
        disclosedAt: 12000,
        updateDueAt: 12000 + 60 * 60 * 24 * 365,
      },
    ],
    conflicts: [
      {
        account: "0x1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b",
        entityId: 7,
        conflictType: "former_employer",
        registeredAt: 12100,
      },
    ],
    reports: [
      {
        id: 0,
        contentHash: "0x" + "ab".repeat(32),
        submittedAt: 12200,
        status: "under_investigation",
      },
    ],
    investigatorCount: 3,
  }),

  auth_generate_challenge: () =>
    `agora://auth?challenge=mock-${Math.random().toString(36).slice(2)}`,

  // Returns "pending" until the real mobile flow completes
  auth_poll_session: () => {
    throw new Error("pending");
  },

  agent_ask: (args: unknown) => {
    const { question } = args as { question: string };
    return `[Mock AI] You asked: "${question}". In production this will call the Claude API with the full item context fetched from IPFS.`;
  },
};
