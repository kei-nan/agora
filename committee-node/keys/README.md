# `keys/` — placeholder key storage

This directory is where an operator would put their `age`-encrypted secrets file locally
before mounting it into the container (`-v $PWD/keys:/keys`, or the `KEYS_FILE` env var if
placed elsewhere). **Nothing in this directory is committed to git** (see `.gitignore` /
`.dockerignore` — only this README is tracked).

## What's real, what's not

- **Real**: the file is encrypted at rest with `age` (age-encryption.org/v1), a standard,
  audited format — not a project-invented scheme. See `../src/keystore.rs`'s module docs for
  the exact JSON shape encrypted inside.
- **Not real / explicit placeholder**: this is "encrypted file on a normal filesystem,"
  nothing more. It does **not** provide tamper resistance against someone with physical or
  root access to the running (or powered-off) device — they can read the passphrase out of
  the container's environment/process list while it's running, or, if using
  `KEY_PASSPHRASE_FILE`, read that file directly. Changelog #082's own "Still open" /
  "Tamper-resistance...needs an add-on TPM" point applies here without qualification: **real
  hardware-backed key custody for this scenario is an open, unsolved problem**, out of scope
  for this component by explicit task instruction. Do not deploy this against a real
  committee's actual secret key material believing this file format solves that.

## Creating a keys file

```bash
echo '{"chain_account_seed":"<64 hex chars>","oprf_secret_key":"<hex>"}' \
  | age -p > keys/committee-secrets.age
```

`age` will prompt for (and confirm) a passphrase interactively. Store that passphrase
separately from this file — via `KEY_PASSPHRASE_FILE` pointing at a file your orchestration
layer manages (a Docker secret, a balenaCloud device variable mounted as a file, etc.), not
baked into an image or committed anywhere.
