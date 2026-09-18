# Email worker work-queue assertions — imported by the root justfile.
# See manifests/mprocs/local.yaml and docs/architecture-backlog.md item 0.1.

# Wipe MailHog AND purge the stream. Purging matters: a freshly created durable
# consumer starts from its DeliverPolicy, so leftover history is replayed and
# pollutes the count.
#
# Clear MailHog and purge the EMAILS stream
[group('email')]
email-reset:
    @curl -s -X DELETE http://localhost:8025/api/v1/messages > /dev/null
    @nats stream purge EMAILS -f > /dev/null 2>&1 || true
    @echo "MailHog cleared, EMAILS stream purged"

# Publish exactly ONE welcome-email job onto the EMAILS stream
[group('email')]
email-publish-one:
    @cargo run -q -p zerg_email_nats --example publish_test

# Emails actually delivered. One published job must deliver ONCE, no matter how
# many worker replicas are running.
#
# Count emails delivered to MailHog
[group('email')]
email-count:
    @curl -s http://localhost:8025/api/v2/messages | jq '.total'

# Durable consumers on EMAILS. A work queue has exactly ONE, shared by every
# replica. One-per-replica means each replica receives every message.
#
# List durable consumers on the EMAILS stream
[group('email')]
email-consumers:
    @nats consumer ls EMAILS 2>/dev/null || echo "(nats CLI or EMAILS stream unavailable)"

# Delete every durable consumer on EMAILS. Use before a run: consumers leak on
# each worker restart today, and stale ones distort the count.
#
# Delete all durable consumers on EMAILS
[group('email')]
email-consumers-clean:
    #!/usr/bin/env bash
    for c in $(nats consumer ls EMAILS -j 2>/dev/null | jq -r '.[]?'); do
      nats consumer rm EMAILS "$c" -f > /dev/null 2>&1 && echo "  removed $c"
    done

# Count live email workers by probing the health ports local.yaml assigns them
# (8081 = `zerg-email`, 8091/8092 = `email-replica-a`/`-b`, 8093/8094 = ad-hoc).
# The NATS consumer count CANNOT do this: correctly-behaving replicas share one
# durable group, so `consumer ls` returns 1 whether one worker runs or five.
[group('email')]
email-workers-live:
    #!/usr/bin/env bash
    n=0
    for p in 8081 8091 8092 8093 8094; do
      if curl -s -o /dev/null --max-time 1 "http://localhost:$p/health"; then
        n=$((n + 1))
      fi
    done
    echo "$n"

# Work-queue assertion. Start `email-replica-a` and `email-replica-b` in
# manifests/mprocs/local.yaml first, then run this.
#
# Asserts three things, all of which must hold:
#   1. >= 2 workers are live      - otherwise "one delivery" is trivially true
#                                   and the gate proves nothing
#   2. exactly 1 consumer group   - replicas SHARE `email-worker`; 2+ groups is
#                                   the per-process-UUID bug (item 0.1)
#   3. exactly 1 delivery         - the job was handled once, not once per replica
#
# Assert one published job delivers exactly once across replicas
[group('email')]
email-replica-check:
    #!/usr/bin/env bash
    set -euo pipefail
    workers=$(just email-workers-live)
    if [ "$workers" -lt 2 ]; then
      echo "  FAIL - only $workers email worker(s) live; this gate needs >= 2."
      echo "         With a single worker 'one job, one delivery' is vacuous."
      echo "         Start email-replica-a and email-replica-b (mprocs: press 's')."
      exit 1
    fi
    just email-reset
    groups=$(nats consumer ls EMAILS -j 2>/dev/null | jq 'length')
    just email-publish-one > /dev/null
    sleep 5
    delivered=$(just email-count)
    echo "  workers live      : $workers"
    echo "  consumer groups   : $groups (must be 1 - replicas share one)"
    echo "  jobs published    : 1"
    echo "  emails delivered  : $delivered"
    if [ "$groups" != "1" ]; then
      echo "  FAIL - $groups consumer groups; each replica invented its own name."
      echo "         That is the fan-out bug. See docs/architecture-backlog.md item 0.1"
      exit 1
    fi
    if [ "$delivered" = "1" ]; then
      echo "  PASS - work queue: one job, one delivery across $workers workers"
    else
      echo "  FAIL - fan-out: $delivered deliveries for 1 job (one per consumer)."
      echo "         See docs/architecture-backlog.md item 0.1"
      exit 1
    fi

# Load test. Publishes N distinct jobs and asserts every one is delivered exactly
# once across however many replicas are running. Stronger than email-replica-check:
# one job proves the consumer group is shared, N jobs prove it stays correct under
# concurrency, and per-recipient counting catches a duplicate that a bare total hides.
#
# NOTE: MailHog's /api/v2/messages caps `count` at 250 per request regardless of
# `limit`, so recipient-level checks MUST paginate. `.total` alone is accurate but
# cannot distinguish "1000 delivered" from "500 delivered twice".
#
# Assert N published jobs each deliver exactly once (default 2000)
[group('email')]
email-scale-check count="2000":
    #!/usr/bin/env bash
    set -euo pipefail
    workers=$(just email-workers-live)
    if [ "$workers" -lt 2 ]; then
      echo "  FAIL - only $workers email worker(s) live; this gate needs >= 2."
      echo "         Distribution across replicas cannot be measured with one."
      exit 1
    fi
    just email-reset
    groups=$(nats consumer ls EMAILS -j 2>/dev/null | jq 'length')
    started=$(python3 -c 'import time;print(time.time())')
    cargo run -q -p zerg_email_nats --example publish_bulk -- {{count}} | tail -1 | sed 's/^/  /'
    for _ in $(seq 1 600); do
      info=$(nats consumer info EMAILS email-worker -j 2>/dev/null)
      [ "$(echo "$info" | jq '.num_pending')" = "0" ] \
        && [ "$(echo "$info" | jq '.num_ack_pending')" = "0" ] && break
      sleep 1
    done
    ended=$(python3 -c 'import time;print(time.time())')
    python3 - {{count}} "$started" "$ended" "$groups" "$workers" <<'PY'
    import collections, json, sys, urllib.request
    count, started, ended, groups, workers = (
        int(sys.argv[1]), float(sys.argv[2]), float(sys.argv[3]), sys.argv[4], sys.argv[5])
    seen, start = [], 0
    while True:
        d = json.load(urllib.request.urlopen(
            f"http://localhost:8025/api/v2/messages?start={start}&limit=200"))
        items = d.get("items", [])
        if not items:
            break
        seen += [f'{m["To"][0]["Mailbox"]}@{m["To"][0]["Domain"]}' for m in items]
        start += len(items)
        if start >= d["total"]:
            break
    got = collections.Counter(seen)
    expected = {f"scale-{i}@example.com" for i in range(count)}
    missing = expected - set(got)
    dupes = {a: c for a, c in got.items() if c > 1}
    elapsed = ended - started
    print(f"  workers live      : {workers}")
    print(f"  consumer groups   : {groups}")
    print(f"  jobs published    : {count}")
    print(f"  emails delivered  : {len(seen)}")
    print(f"  distinct           : {len(got)}")
    print(f"  duplicated        : {len(dupes)}")
    print(f"  missing           : {len(missing)}")
    print(f"  throughput        : {count/elapsed:.0f} emails/s end-to-end ({elapsed:.1f}s)")
    if missing:
        print(f"  FAIL - {len(missing)} job(s) never delivered, e.g. {sorted(missing)[:3]}")
        raise SystemExit(1)
    if dupes:
        print(f"  FAIL - duplicate delivery, e.g. {list(dupes.items())[:3]}")
        print("         Fan-out regression (item 0.1) or missing idempotency (item 0.2).")
        raise SystemExit(1)
    print(f"  PASS - {count} jobs, {count} deliveries, no duplicates, no loss")
    PY
