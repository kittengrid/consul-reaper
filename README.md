# consul-reaper

`consul-reaper` watches Consul nodes and health checks, runs external probe checks itself, updates Consul with the latest health check status, emits node-level webhook events when a node transitions between `healthy` and `unhealthy`, and deregisters nodes that keep failing beyond a configured threshold.

## What it does

For each matching node in Consul:

1. watches the node's health checks
2. starts a Tokio task per health check to execute the probe periodically
3. writes probe results back to Consul
4. aggregates all check states for that node
5. sends a webhook when the node's derived network status changes
6. sends a delete webhook and cleans up when a node is removed
7. deregisters a node from Consul if a health check fails more than the configured threshold

The current node webhook payload is:

```json
{
  "node": "node-name",
  "network_status": "healthy"
}
```

or:

```json
{
  "node": "node-name",
  "network_status": "unhealthy"
}
```

The status webhook is sent with an HTTP `POST` to `node_event_webhook_url`.

When a node is removed, a delete webhook is sent with an HTTP `DELETE` to the same endpoint using this payload:

```json
{
  "node": "node-name"
}
```

---

## High-level architecture

There are three main moving parts:

- **Consul node watcher**: discovers matching nodes
- **Per-node health check watcher**: tracks health checks attached to one node
- **Per-health-check runner**: executes the actual HTTP/TCP check loop

On top of that, each node has a **node status aggregator task** that reacts to health check status changes and decides whether the node is currently `healthy` or `unhealthy`.

## Tokio tasks

At runtime, the application starts these classes of tasks:

### 1. Main node watcher task
Started in `main()`.

Responsibility:
- subscribe to matching Consul nodes using `consul.watch_nodes(...)`
- create a `NodeHealthChecker` for each newly added node

This stream is asynchronous, but internally the Consul client currently polls Consul every 1 second to detect changes.

### 2. Per-node check watcher task
Started by `NodeHealthChecker::start()`.

Responsibility:
- subscribe to `watch_health_checks(node_name)` for a single node
- react to:
  - `HealthCheckEvent::Added`
  - `HealthCheckEvent::Updated`
  - `HealthCheckEvent::Removed`
- create/replace/stop health check runner tasks as needed
- forward health check status changes into the node-local status channel

### 3. Per-node status aggregator task
Also started by `NodeHealthChecker::start()`.

Responsibility:
- receive `CheckStatusEvent`s over a Tokio `mpsc` channel
- maintain an in-memory map of `check_name -> CheckStatus`
- derive the node's `network_status`
- send webhook notifications only when the node status changes

This task is fully event-driven. There is no `sleep()` loop at the node aggregation layer anymore.

### 4. Per-health-check runner task
Started by `HealthCheckRunner::try_new(...)`.

Responsibility:
- execute one Consul health check repeatedly at its configured interval
- convert probe results into Consul `CheckStatus`
- update the check in Consul
- emit a status event when the check status changes
- deregister the node from Consul if failures exceed the configured threshold

There is one such task per active health check.

---

## Event flow

## 1. Node discovery

In `main()`, the application watches only nodes that match:

```text
Meta.external_probe == "true" and Meta.wg_network == "<wg_network>"
```

When a matching node is added:

- a `NodeHealthChecker` is created
- `NodeHealthChecker::start()` spawns the per-node tasks

## 2. Health check discovery

For each node, `watch_health_checks(node_name)` yields health check lifecycle events.

### Added
When a health check is added:

- its current Consul status is pushed into the node status channel as `CheckStatusEvent::Upsert`
- a new `HealthCheckRunner` task is started for that check
- the runner is stored in `tasks`

### Updated
When a health check is updated:

- the latest Consul status is pushed into the node status channel
- the previous runner is stopped
- a new runner is created from the updated definition

This means updates currently restart the runner unconditionally.

### Removed
When a health check is removed:

- the runner is stopped with `abort()`
- the check is removed from the node-local status map via `CheckStatusEvent::Removed`

## 3. Health check execution

Each `HealthCheckRunner` loops forever:

1. read the health check definition
2. sleep for the configured interval
3. execute the probe
4. update local health status
5. register the updated health check back into Consul
6. if the check status changed, emit an event to the node aggregator

Supported check types:

- `HTTP`
- `TCP`

Unsupported today:

- script checks
- unknown check types

---

## Health check execution details

The probe logic lives in `src/checkers.rs`.

### HTTP checks

`HTTPChecker`:
- builds a `reqwest::Client` with the configured timeout
- sends a `GET` to the check URL
- returns success only for HTTP success status codes

### TCP checks

`TCPChecker`:
- tries to open a TCP connection to the configured address
- wraps the connection attempt in a Tokio timeout

### Probe scheduling

The interval comes from the Consul check definition:

- `Definition.Interval`

If parsing fails, the Consul helper falls back to its internal default interval.

### Failure threshold

A check is not marked critical on the first failure.

The runner maintains a consecutive failure counter. The threshold is configurable with `--delete-threshold` and defaults to:

```text
3
```

Behavior:

- success => status becomes `passing`, failure counter resets to `0`
- failure => failure counter increments
- after `delete_threshold` consecutive failures => status becomes `critical`
- after more than `delete_threshold` consecutive failures => the node is deregistered from Consul

With the default value `3`:

- 3 consecutive failures => check becomes `critical`
- 4 consecutive failures => node is deregistered

This dampens transient failures while still removing persistently failing nodes.

---

## How node status is derived

The node-local aggregator maintains:

```text
HashMap<String, CheckStatus>
```

where the key is the health check name.

### Current rule

A node is considered:

- **`unhealthy`** if **all** known checks are `critical`
- **`healthy`** otherwise

If there are no known checks yet, no webhook is sent.

### Why this is event-driven now

Previously, node status was recomputed in a loop with a 10-second sleep. That design had two drawbacks:

- up to 10 seconds of delay before emitting a state change
- periodic work even when nothing changed

The current design removes that polling loop.

Now:

- health check runners emit status changes immediately
- the node aggregator reacts immediately
- webhooks are sent on transitions only

---

## How webhook notifications are controlled

The node aggregator keeps:

```text
last_network_status: Option<&'static str>
```

After every incoming check event:

1. recompute node status
2. compare with `last_network_status`
3. if unchanged, do nothing
4. if changed, send webhook and update `last_network_status`

This prevents duplicate notifications when repeated probe results produce the same effective node state.

---

## Consul interaction model

There are two distinct Consul interaction paths:

### 1. Watching
The project exposes:

- `watch_nodes(filter)`
- `watch_health_checks(node_name)`

These return asynchronous streams, but they are currently implemented by repeatedly polling Consul and diffing results internally.

So from the application's point of view the API is async and stream-based, but under the hood it still uses periodic polling against Consul.

### 2. Writing check status
After each probe iteration, the runner calls:

- `consul.register_node(health_check.clone().into())`

This updates Consul with the latest health check state.

### 3. Deregistering persistently failing nodes
If a health check keeps failing beyond the configured threshold, the runner calls:

- `consul.deregister_node(health_check.clone().into())`

This removes the node from the Consul catalog. The main node watcher then receives a removal event, stops per-node tasks, and sends the delete webhook.

---

## Important in-memory state

### `NodeHealthChecker.tasks`

```rust
Arc<RwLock<HashMap<String, HealthCheckRunner>>>
```

This stores the active runner task for each health check on the node.

Used for:
- replacing runners on check updates
- stopping runners on check removal

### Node status channel

Each node creates an unbounded Tokio channel:

```rust
mpsc::unbounded_channel::<CheckStatusEvent>()
```

This channel carries status events from:
- the health check watcher
- the health check runner tasks

to:
- the node status aggregator

### `CheckStatusEvent`

Two event types exist:

- `Upsert { check_name, status }`
- `Removed { check_name }`

This is enough for the node aggregator to maintain the current per-check view.

---

## Current lifecycle example

Imagine a node with two checks:

- `http-api`
- `tcp-wireguard`

### Startup
- node is discovered
- node watcher task starts
- check watcher task sees two checks added
- two health check runners are spawned
- current check states are inserted into the node status map
- if at least one check is not critical, node becomes `healthy`
- webhook is sent once with `network_status=healthy`

### One check starts failing
- `http-api` fails once, then twice
- still not critical yet because threshold is 3
- node status may remain `healthy`

### Third consecutive failure
- `http-api` becomes `critical`
- runner emits `Upsert(http-api, critical)`
- aggregator recomputes node status
- if `tcp-wireguard` is still passing, node remains `healthy`
- no webhook is sent because effective node state did not change

### All checks critical
- `tcp-wireguard` also becomes `critical`
- aggregator now sees all checks critical
- node transitions to `unhealthy`
- webhook is sent once with `network_status=unhealthy`

### Recovery
- one check becomes `passing`
- aggregator recomputes immediately
- node transitions back to `healthy`
- webhook is sent once with `network_status=healthy`

### Persistent failure beyond threshold
- a check reaches the configured threshold and becomes `critical`
- if failures continue past the threshold, the runner deregisters the node from Consul
- the node watcher later observes `NodeEvent::Removed`
- the node-local tasks are stopped
- a `DELETE` webhook is sent with the node name

---

## CLI / environment variables

The binary accepts these arguments, all of which can also be provided via environment variables because of Clap's `env` support:

### `--consul-http-addr`
Default:

```text
http://localhost:8500
```

### `--wg-network`
Used in the Consul node filter.

### `--node-event-webhook-url`
The URL that receives node status events via `POST` and node deletion events via `DELETE`.

### `--delete-threshold`
Default:

```text
3
```

A check becomes `critical` after this many consecutive failures. If failures continue beyond this value, the node is deregistered from Consul.

Example:

```bash
cargo run -- \
  --consul-http-addr http://localhost:8500 \
  --wg-network prod-eu \
  --node-event-webhook-url https://example.com/node-events \
  --delete-threshold 3
```

---

## Limitations / current behavior to be aware of

### 1. Watch streams are async but internally poll Consul
Node and health check watches are implemented as streams over internal polling/diffing, currently at a 1-second cadence.

### 2. Health check updates restart runner tasks
`HealthCheckEvent::Updated` stops and recreates the runner even if the update may only be a status change.

### 3. Only HTTP and TCP probes are supported
Script-based checks are explicitly unsupported.

### 4. Empty check set does not emit a node state
If all checks disappear, the node aggregator simply keeps waiting and does not send a webhook for an "unknown" or empty state.

### 5. Node deletion happens after threshold + 1 failures
The current behavior is intentionally split:
- at `delete_threshold` consecutive failures, the check becomes `critical`
- at `delete_threshold + 1` consecutive failures, the node is deregistered

### 6. Channel is unbounded
The per-node `mpsc` channel is unbounded. That is simple and effective here, but it is still worth remembering if event volume grows significantly.

---

## Source map

- `src/main.rs`
  - application orchestration
  - Tokio task creation
  - node aggregation
  - webhook sending
- `src/checkers.rs`
  - HTTP and TCP probe implementations
- `src/consul.rs`
  - Consul client
  - node and health check watch streams
  - Consul models and serialization

---

## Summary

`consul-reaper` is effectively an event-driven health orchestration loop around Consul:

- discover nodes
- discover checks
- run probes
- write status back to Consul
- aggregate per-node state
- emit webhooks on transitions
- deregister persistently failing nodes
- emit delete webhooks on node removal

The most important design point is that **node status propagation is now event-driven**: health check changes are pushed through Tokio channels and webhooks are emitted immediately when the effective node state changes.
