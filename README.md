# Agora

A blockchain-based distributed democracy platform for real government adoption, built on
[Substrate](https://substrate.io/) / the [Polkadot SDK](https://github.com/paritytech/polkadot-sdk).
Full separation of powers — legislature, executive, judiciary, elections commission, human
rights protections, anti-corruption, audit — is enforced by smart-contract (pallet) boundaries
rather than by institutional norms alone.

Citizens register with a one-time biometric passport NFC scan and a zero-knowledge proof
generated on-device (nothing leaves the phone); voting is anonymous and receipt-free (MACI),
supports liquid democracy (transitive, revocable, per-topic delegation), and feeds a petition →
signature threshold → referendum pipeline. Laws live on an on-chain, versioned ledger with an
AI-first court system (AI judge → human jury appeal, rulings auto-enforced on-chain), and a
public, real-time treasury ledger with per-department spend caps and audit hooks.

This README covers building and running the chain. For the full architecture, current
implementation status, and the reasoning behind key design decisions, see:

- [`CLAUDE.md`](./CLAUDE.md) — authoritative project context: what Agora is, architecture
  decisions, government structure, identity/voting/court/treasury design, monorepo layout,
  and remaining work in priority order.
- [`docs/project/README.md`](./docs/project/README.md) — developer handoff index: per-pallet
  storage/calls docs, runtime wiring, ZK verifier status, mobile/desktop app status, and a
  chronological changelog.

## Monorepo layout

```
democracy-chain/
├── node/       ← chain binary (agora-node)
├── runtime/    ← WASM runtime (agora-runtime) — 11 pallets: identity, voting,
│                 treasury-ledger, courts, constitution, legislature, elections,
│                 emergency-council, audit, anticorruption, executive
├── pallets/    ← one crate per pallet (see docs/project/pallets/ for per-pallet docs)
├── circuits/   ← Noir ZK circuits (ZKPassport passport-proof circuits)
├── mobile/     ← React Native app — passport NFC scan, on-device ZK proof, voting
├── desktop/    ← Tauri 2 app — read-heavy chain browser + optional Claude AI agent panel
└── CLAUDE.md   ← authoritative project context (read this first)
```

## Getting started

Depending on your operating system and Rust version, there might be additional packages
required to compile this project. Check the [Install](https://docs.substrate.io/install/)
instructions for your platform for the most common dependencies, or use one of the
[alternative installation](#alternative-installations) options below.

### Build

🔨 This project pins dependencies that make the WASM runtime build fail on Rust 1.84+ under the
default flags (a `substrate-wasm-builder` 26.0.1 incompatibility). Always build with:

```sh
WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" cargo build --release
```

### Embedded docs

After you build the project, you can use the following command to explore its parameters and
subcommands:

```sh
./target/release/agora-node -h
```

You can generate and view the [Rust Docs](https://doc.rust-lang.org/cargo/commands/cargo-doc.html)
for this project with:

```sh
cargo +nightly doc --open
```

### Single-node development chain

The following command starts a single-node development chain that doesn't persist state:

```sh
./target/release/agora-node --dev --tmp
```

To purge the development chain's state, run:

```sh
./target/release/agora-node purge-chain --dev
```

To start the development chain with detailed logging, run:

```sh
RUST_BACKTRACE=1 ./target/release/agora-node -ldebug --dev
```

Development chains:

- Maintain state in a `tmp` folder while the node is running.
- Use the **Alice** and **Bob** accounts as default validator authorities.
- Use the **Alice** account as the default `sudo` account.
- Are preconfigured with a genesis state (`/node/src/chain_spec.rs`) that includes several
  pre-funded development accounts.

To persist chain state between runs, specify a base path:

```sh
// Create a folder to use as the db base path
$ mkdir my-chain-state

// Use of that folder to store the chain state
$ ./target/release/agora-node --dev --base-path ./my-chain-state/

// Check the folder structure created inside the base path after running the chain
$ ls ./my-chain-state
chains
$ ls ./my-chain-state/chains/
dev
$ ls ./my-chain-state/chains/dev
db keystore network
```

### Connect with Polkadot-JS Apps front-end

After you start the node locally, you can interact with it using the hosted version of the
[Polkadot/Substrate Portal](https://polkadot.js.org/apps/#/explorer?rpc=ws://localhost:9944)
front-end by connecting to the local node endpoint (`ws://localhost:9944`). A hosted version is
also available on [IPFS](https://dotapps.io/). You can also find the source code and
instructions for hosting your own instance in the
[`polkadot-js/apps`](https://github.com/polkadot-js/apps) repository.

### Multi-node local testnet

If you want to see the multi-node consensus algorithm in action, see [Simulate a
network](https://docs.substrate.io/tutorials/build-a-blockchain/simulate-network/).

## Project structure

A Substrate project such as this consists of a number of components spread across a few
directories.

### Node

A blockchain node is an application that allows users to participate in a blockchain network.
Substrate-based blockchain nodes expose a number of capabilities:

- Networking: Substrate nodes use the [`libp2p`](https://libp2p.io/) networking stack to allow
  the nodes in the network to communicate with one another.
- Consensus: Blockchains must have a way to come to
  [consensus](https://docs.substrate.io/fundamentals/consensus/) on the state of the network.
  Substrate makes it possible to supply custom consensus engines and also ships with several
  consensus mechanisms built on top of [Web3 Foundation
  research](https://research.web3.foundation/Polkadot/protocols/NPoS).
- RPC Server: A remote procedure call (RPC) server is used to interact with Substrate nodes.

There are several files in the `node` directory. Take special note of the following:

- [`chain_spec.rs`](./node/src/chain_spec.rs): A [chain
  specification](https://docs.substrate.io/build/chain-spec/) is a source code file that
  defines the chain's initial (genesis) state. Take note of the `development_config` function,
  used to define the genesis state for the local development chain configuration.
- [`service.rs`](./node/src/service.rs): This file defines the node implementation. Take note
  of the libraries it imports and the functions it invokes — in particular, references to
  consensus-related topics such as [block finalization and
  forks](https://docs.substrate.io/fundamentals/consensus/#finalization-and-forks) and
  Aura (block authoring) / GRANDPA (finality).

### Runtime

In Substrate, the terms "runtime" and "state transition function" are analogous. Both refer to
the core logic of the blockchain responsible for validating blocks and executing the state
changes they define. Agora's runtime (`agora-runtime`) is built with
[FRAME](https://docs.substrate.io/learn/runtime-development/#frame), which lets domain-specific
logic be declared in modules called "pallets" and composed into a single runtime.

Review the [FRAME runtime implementation](./runtime/src/lib.rs) and note:

- `runtime/src/configs/mod.rs` configures every pallet in the runtime plus the cross-pallet
  trait wiring between them (which pallet implements which other pallet's callback trait) —
  see [`docs/project/architecture.md`](./docs/project/architecture.md) for the full picture.
- The pallets are composed into a single runtime by way of the
  [`#[runtime]`](https://paritytech.github.io/polkadot-sdk/master/frame_support/attr.runtime.html)
  macro, part of the [core FRAME pallet
  library](https://docs.substrate.io/reference/frame-pallets/#system-pallets).

### Pallets

The runtime is constructed from a mix of FRAME pallets that ship with [the Substrate
repository](https://github.com/paritytech/polkadot-sdk/tree/master/substrate/frame) (balances,
timestamp, sudo, Aura, GRANDPA, transaction-payment) and Agora's own pallets defined under
[`pallets/`](./pallets/) — identity, voting, treasury-ledger, courts, constitution,
legislature, elections, emergency-council, audit, and anticorruption. See
[`docs/project/pallets/`](./docs/project/pallets/) for storage/calls/config documentation per
pallet.

A FRAME pallet is comprised of a number of blockchain primitives, including:

- Storage: FRAME defines a rich set of powerful [storage
  abstractions](https://docs.substrate.io/build/runtime-storage/) that make it easy to use
  Substrate's efficient key-value database to manage the evolving state of a blockchain.
- Dispatchables: FRAME pallets define special types of functions that can be invoked
  (dispatched) from outside of the runtime in order to update its state.
- Events: Substrate uses [events](https://docs.substrate.io/build/events-and-errors/) to notify
  users of significant state changes.
- Errors: When a dispatchable fails, it returns an error.

Each pallet has its own `Config` trait which serves as a configuration interface to
generically define the types and parameters it depends on.

## Alternative installations

Instead of installing dependencies and building this source directly, consider the following
alternatives.

### Nix

Install [nix](https://nixos.org/) and
[nix-direnv](https://github.com/nix-community/nix-direnv) for a fully plug-and-play experience
for setting up the development environment. To get all the correct dependencies, activate
direnv with `direnv allow`.

### Docker

Please follow the [Substrate Docker instructions
here](https://github.com/paritytech/polkadot-sdk/blob/master/substrate/docker/README.md) to
build a Docker container with the Agora node binary.
