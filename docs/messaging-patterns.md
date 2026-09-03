# Messaging & Communication Patterns

Technology **reference**: when to use Redis Streams, Kafka, RabbitMQ, and gRPC.

> **Looking for what *we* should do?** This page compares technologies, most of which we
> do not run. For the decision guide covering the three transports this repo actually
> operates — NATS JetStream, gRPC, HTTP — plus the dual-write problem and saga patterns,
> see [`communication-and-consistency.md`](./communication-and-consistency.md).

## Quick Decision Guide

| Need | Use |
|------|-----|
| Request/Response, low latency | **gRPC** |
| Simple task queue | **Redis Streams** or **RabbitMQ** |
| Complex routing, work queues | **RabbitMQ** |
| High-throughput event streaming | **Kafka** |
| Event sourcing, replay | **Kafka** |
| Lightweight pub/sub (already have Redis) | **Redis Streams** |

---

## gRPC

**What**: Synchronous RPC framework using HTTP/2 and Protocol Buffers.

**Use when**:
- Service-to-service communication requiring immediate response
- Low latency is critical (sub-millisecond)
- Strong typing and contract-first API design
- Bi-directional streaming needed
- Polyglot environment (multiple languages)

**Don't use when**:
- Fire-and-forget messaging
- Need message persistence/replay
- Decoupling producers from consumers
- Fan-out to many consumers

**Examples**:
- API gateway to microservices
- Real-time data fetching
- Inter-service calls in request path
- Mobile/web clients to backend

```
┌─────────┐  request   ┌─────────┐
│  API    │ ─────────► │  Tasks  │
│ (Axum)  │ ◄───────── │ Service │
└─────────┘  response  └─────────┘
```

See [grpc.md](./grpc.md) for detailed gRPC streaming patterns.

---

## Redis Streams

**What**: Append-only log data structure in Redis with consumer groups.

**Use when**:
- Already using Redis in your stack
- Simple event streaming needs
- Moderate throughput (10k-100k msg/sec)
- Need consumer groups but not complex routing
- Short-term message retention (hours/days)
- Want minimal operational overhead

**Don't use when**:
- Need long-term storage (weeks/months)
- Very high throughput (millions msg/sec)
- Complex routing logic needed
- Strong durability guarantees required

**Examples**:
- Activity feeds
- Real-time notifications
- Simple task distribution
- Cache invalidation events
- Lightweight event sourcing

```
┌──────────┐         ┌─────────────┐         ┌──────────┐
│ Producer │ ──────► │ Redis Stream│ ──────► │ Consumer │
└──────────┘  XADD   │  (orders)   │  XREAD  │  Group   │
                     └─────────────┘         └──────────┘
```

---

## RabbitMQ

**What**: Traditional message broker implementing AMQP protocol.

**Use when**:
- Complex routing requirements (topic, headers, fanout)
- Work queues with acknowledgments
- Need message priorities
- Request/reply patterns over messaging
- Delayed/scheduled messages
- Need fine-grained delivery guarantees
- Moderate throughput (10k-50k msg/sec)

**Don't use when**:
- Need to replay old messages
- Very high throughput requirements
- Long-term message storage
- Event sourcing patterns

**Examples**:
- Background job processing
- Email/notification queues
- Order processing workflows
- RPC over messaging
- Distributing work across workers

```
┌──────────┐      ┌──────────┐      ┌─────────┐      ┌──────────┐
│ Producer │ ───► │ Exchange │ ───► │  Queue  │ ───► │ Consumer │
└──────────┘      └──────────┘      └─────────┘      └──────────┘
                   (routing)        (buffering)
```

**Exchange types**:
- `direct` - route by exact key match
- `topic` - route by pattern (e.g., `orders.*.created`)
- `fanout` - broadcast to all queues
- `headers` - route by message headers

---

## Kafka

**What**: Distributed event streaming platform with persistent log.

**Use when**:
- Very high throughput (millions msg/sec)
- Need to replay/reprocess events
- Event sourcing architecture
- Long-term message retention (days/weeks/forever)
- Multiple consumers need same events
- Stream processing (with Kafka Streams/Flink)
- Audit logs, change data capture

**Don't use when**:
- Simple task queues
- Need complex routing per message
- Low message volume (overkill)
- Need message priorities
- Want minimal operational complexity

**Examples**:
- Event sourcing
- Change data capture (CDC)
- Log aggregation
- Metrics/analytics pipelines
- Cross-datacenter replication
- Real-time ETL

```
┌──────────┐         ┌─────────────────────────────┐
│ Producer │ ──────► │ Topic: orders               │
└──────────┘         │ ┌─────┬─────┬─────┬─────┐   │
                     │ │ P0  │ P1  │ P2  │ P3  │   │  (partitions)
                     │ └─────┴─────┴─────┴─────┘   │
                     └─────────────────────────────┘
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
         ┌─────────┐    ┌─────────┐    ┌─────────┐
         │Consumer │    │Consumer │    │Consumer │
         │ Group A │    │ Group A │    │ Group B │
         └─────────┘    └─────────┘    └─────────┘
```

---

## Comparison Table

| Feature | gRPC | Redis Streams | RabbitMQ | Kafka |
|---------|------|---------------|----------|-------|
| **Pattern** | Request/Response | Pub/Sub, Streaming | Queue, Pub/Sub | Event Streaming |
| **Latency** | Very Low | Low | Low-Medium | Medium |
| **Throughput** | High | Medium-High | Medium | Very High |
| **Persistence** | None | Short-term | Until consumed | Long-term |
| **Replay** | No | Limited | No | Yes |
| **Ordering** | Per-stream | Per-stream | Per-queue | Per-partition |
| **Routing** | None | Basic | Advanced | Partition-based |
| **Ops Complexity** | Low | Low | Medium | High |
| **Consumer Groups** | N/A | Yes | Yes | Yes |

---

## Architecture Patterns

### Pattern 1: Synchronous + Async Hybrid

```
┌────────┐  gRPC   ┌────────┐  Kafka   ┌────────────┐
│  API   │ ──────► │ Orders │ ───────► │ Analytics  │
│Gateway │ ◄────── │Service │          │  Service   │
└────────┘         └────────┘          └────────────┘
                        │
                        │ RabbitMQ
                        ▼
                   ┌────────┐
                   │ Email  │
                   │Service │
                   └────────┘
```

- **gRPC**: User-facing API calls (fast response needed)
- **Kafka**: Event stream for analytics (replay, multiple consumers)
- **RabbitMQ**: Task queue for emails (work distribution, retries)

### Pattern 2: CQRS with Event Sourcing

```
┌─────────┐  gRPC   ┌─────────┐  Kafka  ┌──────────┐
│ Command │ ──────► │ Command │ ──────► │  Event   │
│  API    │         │ Service │         │  Store   │
└─────────┘         └─────────┘         └──────────┘
                                              │
                                              ▼
┌─────────┐  gRPC   ┌─────────┐         ┌──────────┐
│  Query  │ ◄────── │  Query  │ ◄────── │  Read    │
│   API   │         │ Service │         │  Model   │
└─────────┘         └─────────┘         └──────────┘
```

### Pattern 3: Simple Microservices

```
┌─────────┐         ┌─────────┐         ┌─────────┐
│  API    │  gRPC   │  Tasks  │  Redis  │ Workers │
│ (Axum)  │ ──────► │ Service │ ──────► │         │
└─────────┘         └─────────┘ Streams └─────────┘
```

---

## Decision Flowchart

```
Need immediate response?
├── Yes → gRPC
└── No → Need message replay?
         ├── Yes → Kafka
         └── No → Complex routing needed?
                  ├── Yes → RabbitMQ
                  └── No → Already have Redis?
                           ├── Yes → Redis Streams
                           └── No → RabbitMQ (simpler) or Kafka (scalable)
```

---

## This Project's Setup

Currently using:
- **NATS JetStream** (`libs/core/messaging`): durable job queues and domain events —
  email jobs consumed by `apps/zerg/email-nats`, `TodoEvent` consumed by
  `apps/todo/worker`. Explicit ack, bounded retries, dead-letter stream. This is our
  cleanest boundary: the queue forces a message contract and forbids shared state.
- **gRPC** (tonic): `zerg_api` → `zerg_tasks`, synchronous on the request path.
  Boundary remediation in progress — see
  [`adr-tasks-service-boundary.md`](./adr-tasks-service-boundary.md).
- **Redis**: sessions, rate limiting, login-flow store.
- **PostgreSQL**: primary data store (per-service databases; see `manifests/db/README.md`).

Potential additions:
- **Kafka**: if needing event sourcing or an analytics pipeline (NATS covers today's needs).
- **RabbitMQ**: if needing complex background job routing.

### Choosing between them here

Before adding a **synchronous** hop, check that a library call or an event won't do.
A gRPC call couples availability (callee down ⇒ caller's route fails) and latency
(caller waits) — pay that only when you need the answer *inside* the current request.
If the work can happen shortly afterwards, publish to JetStream instead: the producer
returns immediately, the consumer retries on its own, and failures land in the DLQ
rather than in the user's response.

Whichever you pick, a transport is not a boundary — see the checklist in
[`modular-monolith-architecture.md`](./modular-monolith-architecture.md#then-the-boundary-checklist).
