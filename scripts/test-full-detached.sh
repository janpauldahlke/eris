#!/usr/bin/env bash
# Run cargo test-full outside the GNOME session so a desktop logout does not kill it.
# Does not change GPU, prime, keymap, or other display settings.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
JOB_LOG="target/test-full-detached-${STAMP}.log"
RUNNER="$ROOT/target/test-full-detached-runner.sh"
TAG="eris-test-full-detached"

cat >"$RUNNER" <<EOF
#!/usr/bin/env bash
set -euo pipefail
cd "$ROOT"
export PATH="\${HOME}/.cargo/bin:\${PATH}"
(
  echo "=== detached test-full start \$(date -Is) ==="
  cargo test-full
  echo "=== detached test-full end \$(date -Is) exit=\$? ==="
) >>"$JOB_LOG" 2>&1
(crontab -l 2>/dev/null | grep -v "$TAG" || true) | crontab - 2>/dev/null || true
EOF
chmod +x "$RUNNER"

LOCK="$ROOT/target/test-full-detached.lock"
# One-shot cron entry (next minute). flock avoids duplicate starts if the suite runs >1 min.
(crontab -l 2>/dev/null | grep -v "$TAG" || true
 echo "* * * * * flock -n $LOCK $RUNNER # $TAG") | crontab -

echo "scheduled detached test-full (cron, within 60s)"
echo "  detached log: $JOB_LOG"
echo "  batch log:    target/test-full.log"
echo "  cancel:       crontab -l | grep -v $TAG | crontab -"
