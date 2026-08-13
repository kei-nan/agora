#!/usr/bin/env bash
# Host-level hardening for a cloud VM dedicated to running committee-node.
#
# Implements Tier 0/1 items 1-4, 13, 15 of
# docs/project/research/oprf-alternatives/09-cloud-security-hardening.md. Read that document
# before running this anywhere real — this script is the mechanical part of the checklist, not
# a substitute for understanding it.
#
# Target: a fresh Debian/Ubuntu VM, dedicated to this workload only (checklist item 1 — do not
# run this on a host that serves anything else). Idempotent: safe to re-run.
#
# NOT executed against any real infrastructure as part of authoring this — same honesty
# convention as this component's Dockerfile (see its own "NOTE ON VERIFICATION"). Reasoned
# through against documented `ufw`/`sysctl`/`unattended-upgrades` behaviour, not empirically
# verified end-to-end. Review before running.
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "Run as root (sudo $0)." >&2
  exit 1
fi

echo "== committee-node host hardening =="

# --- 1. Firewall: zero inbound, all outbound (checklist items 2-3) -----------------------------
# Default-deny incoming, allow all outgoing (the process only ever makes outbound calls to
# NODE_RPC_URL — see 09's "Network exposure" section for the full trust-boundary discussion).
# Deliberately does NOT open 22/tcp. If this host has no other access path configured (provider
# console / SSM / IAP — see the deploy README), do that FIRST in another terminal before running
# this script, or you can lock yourself out.
if command -v ufw >/dev/null 2>&1; then
  ufw --force reset >/dev/null
  ufw default deny incoming
  ufw default allow outgoing
  ufw --force enable
  echo "  [ok] ufw: deny all inbound, allow all outbound, no SSH rule added"
else
  echo "  [!!] ufw not found — install it (apt-get install -y ufw) or apply an equivalent" \
       "default-deny-inbound ruleset via your provider's firewall/security-group console instead." >&2
fi

# Block the cloud metadata service from anything running as a non-root user (checklist item 3's
# note). The container itself never needs it; this is defense-in-depth against a compromised
# process trying to reach instance credentials.
if command -v iptables >/dev/null 2>&1; then
  iptables -C OUTPUT -d 169.254.169.254 -m owner ! --uid-owner 0 -j DROP 2>/dev/null || \
    iptables -A OUTPUT -d 169.254.169.254 -m owner ! --uid-owner 0 -j DROP
  echo "  [ok] iptables: non-root processes blocked from 169.254.169.254 (cloud metadata service)"
fi

# --- 2. No core dumps, no hibernation (checklist item 13) -----------------------------------
# The OPRF share is plaintext in process memory for the container's entire lifetime
# (see 09's "The share is plaintext in process memory, 24/7"). A core dump or a hibernation
# image is a copy of that memory written to disk.
cat > /etc/sysctl.d/99-committee-node-hardening.conf <<'EOF'
# Disable setuid core dumps and route all core dumps to nowhere.
fs.suid_dumpable = 0
kernel.core_pattern = |/bin/false
EOF
sysctl -p /etc/sysctl.d/99-committee-node-hardening.conf >/dev/null
echo "  [ok] sysctl: core dumps disabled (fs.suid_dumpable=0, kernel.core_pattern=/bin/false)"

if command -v systemctl >/dev/null 2>&1; then
  systemctl mask hibernate.target suspend.target hybrid-sleep.target suspend-then-hibernate.target 2>/dev/null || true
  echo "  [ok] systemd: hibernate/suspend targets masked"
fi

# --- 3. OS security updates only — never auto-update the application image (checklist item 15) -
# unattended-upgrades patches the HOST OS. It must never be configured to touch the committee-node
# container image — that would silently recreate the push-update channel
# docs/.../09-cloud-security-hardening.md's "Update and patch governance" section explains why to
# avoid (whoever controls the image registry should not be able to force all 5 committees to run
# new code at once). This script only ever touches the OS package layer.
if command -v apt-get >/dev/null 2>&1; then
  apt-get update -qq
  apt-get install -y -qq unattended-upgrades >/dev/null
  dpkg-reconfigure -f noninteractive unattended-upgrades >/dev/null 2>&1 || true
  echo "  [ok] unattended-upgrades installed for OS security patches (application image is NOT" \
       "auto-updated by this or anything else — see deploy/README.md's update policy)"
fi

# --- 4. Verification -----------------------------------------------------------------------
echo
echo "== Verify from OUTSIDE this host, not from a shell on it =="
echo "  nmap -Pn <this-host-ip>     # should show no open ports"
echo
echo "== Reminders this script does NOT handle =="
echo "  - Disable the provider's default SSH ingress rule in their console/API (provider-specific)."
echo "  - Set up out-of-band access (provider console, SSM Session Manager, IAP, Azure Bastion) BEFORE"
echo "    you lose the ability to reach this host any other way."
echo "  - Enable hardware-key MFA + least-privilege IAM on the cloud account itself — with no SSH,"
echo "    cloud IAM is the primary way in (checklist item 5)."
echo "  - Enable a Confidential VM (SEV-SNP/TDX) at instance-creation time if your provider offers it"
echo "    — that's a provider-console setting, not something this script can do after the fact."
echo
echo "See docs/project/research/oprf-alternatives/09-cloud-security-hardening.md for the full"
echo "checklist and the reasoning behind each item."
