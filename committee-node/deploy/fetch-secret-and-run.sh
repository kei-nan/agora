#!/bin/sh
# Wrapper entrypoint: fetches the age passphrase from a secret manager into a tmpfs mount, then
# execs the real committee-node binary. Requires ZERO changes to committee-node itself — it
# already reads KEY_PASSPHRASE_FILE from a path (see src/config.rs) and this script just makes
# sure that path is tmpfs-backed and never touches persistent disk.
#
# Why this exists: docs/project/research/oprf-alternatives/09-cloud-security-hardening.md's
# "Secrets and key-share custody" section — both README.md's docker-run example and
# docker-compose.example.yml mount the age-encrypted key file *and* its passphrase from the same
# host disk, which on a cloud VM means "snapshot the volume" (one API call, available to the
# provider, its backup system, and anyone with console credentials) yields both. Fetching the
# passphrase at container start from a secret manager, into tmpfs, closes that gap: the
# passphrase survives no reboot and no disk snapshot, and the fetch itself becomes an auditable,
# alarmable API call.
#
# Operator responsibility: set SECRET_FETCH_CMD to a shell command that prints the passphrase to
# stdout and nothing else. Examples (uncomment/adapt one, or write your own):
#
#   AWS Secrets Manager:
#     SECRET_FETCH_CMD='aws secretsmanager get-secret-value --secret-id committee-passphrase --query SecretString --output text'
#
#   GCP Secret Manager:
#     SECRET_FETCH_CMD='gcloud secrets versions access latest --secret=committee-passphrase'
#
#   Azure Key Vault:
#     SECRET_FETCH_CMD='az keyvault secret show --vault-name <vault> --name committee-passphrase --query value -o tsv'
#
#   HashiCorp Vault:
#     SECRET_FETCH_CMD='vault kv get -field=passphrase secret/committee-passphrase'
#
# The CLI tool for whichever of these you use must be installed in the image or bind-mounted in
# — this repo does not bundle any of them, to keep the base image provider-agnostic. See
# deploy/README.md for a worked example of extending the image with one.
#
# NOT executed against any real secret manager as part of authoring this — reasoned through, not
# empirically verified. Review before running (same honesty convention as the rest of this
# component — see committee-node/README.md).
set -eu

SECRET_DIR="/run/secrets"
SECRET_PATH="${SECRET_DIR}/passphrase"

if [ -z "${SECRET_FETCH_CMD:-}" ]; then
  echo "fetch-secret-and-run.sh: SECRET_FETCH_CMD is not set." >&2
  echo "  Set it to a command that prints the age passphrase to stdout and nothing else." >&2
  echo "  See this script's header comment for examples, or set KEY_PASSPHRASE_FILE directly" >&2
  echo "  to skip this wrapper entirely (e.g. for local dev — see docker-compose.example.yml)." >&2
  exit 1
fi

# Confirm the destination is actually tmpfs before writing anything sensitive to it — a
# misconfigured compose/systemd unit that forgot the tmpfs mount would otherwise silently write
# the passphrase to persistent disk, exactly the failure mode this script exists to prevent.
mount_type="$(findmnt -n -o FSTYPE --target "${SECRET_DIR}" 2>/dev/null || echo unknown)"
if [ "${mount_type}" != "tmpfs" ]; then
  echo "fetch-secret-and-run.sh: ${SECRET_DIR} is not tmpfs (got '${mount_type}')." >&2
  echo "  Refusing to write the passphrase to persistent storage. Check the tmpfs mount in" >&2
  echo "  deploy/docker-compose.prod.yml." >&2
  exit 1
fi

umask 077
sh -c "${SECRET_FETCH_CMD}" > "${SECRET_PATH}"
if [ ! -s "${SECRET_PATH}" ]; then
  echo "fetch-secret-and-run.sh: SECRET_FETCH_CMD produced empty output." >&2
  exit 1
fi

export KEY_PASSPHRASE_FILE="${SECRET_PATH}"
unset SECRET_FETCH_CMD

exec /usr/local/bin/committee-node
