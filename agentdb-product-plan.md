# AgentDB: Product Scope & Planning Document

*Working name. Rename before public launch.*

---

## Executive Summary

AgentDB is an open-source-led, agent-first hosted database platform. The open-source project is an MCP server for PostgreSQL, Redis, and ClickHouse that becomes the default way agents interact with databases. The commercial product is a managed cloud platform that adds a semantic schema layer, capability-based agent authorization, query guardrails, and full audit trails.

The company bets on one thesis: within 2-3 years, agents will be the primary consumers of databases, not human developers writing SQL. The current database hosting market (Supabase, Neon, PlanetScale) is built entirely for humans. AgentDB is built for agents first, and excellent for AI-assisted developers as the bridge market.

---

## Part 1: Product Definition

### 1.1 What We Ship

The product has two layers:

**Layer 1 — Open Source: `agentdb-mcp`**

A standalone MCP server that connects to any self-hosted or cloud-hosted PostgreSQL, Redis, or ClickHouse instance and exposes structured tools for agents to discover schemas, read and write data, manage migrations, and introspect their own permissions. This is a Go binary (single static binary, no runtime dependencies) that anyone can run locally or in production. It is Apache 2.0 licensed.

The open-source project is the GTM engine. Its job is to become the default MCP database server referenced in every agent framework's documentation.

**Layer 2 — Commercial: AgentDB Cloud**

A managed platform where you provision databases and connect agents in one step. Cloud adds everything the open-source project deliberately excludes:

- Managed PostgreSQL, Redis, and ClickHouse provisioning with connection pooling baked in
- Semantic schema layer: natural-language descriptions, example values, relationship explanations, and constraint rationale on every database object, auto-inferred on creation and human-refinable
- Capability-based agent authorization: short-lived, scoped JWT credentials that agents can introspect
- Query guardrails: cost estimation before execution, automatic LIMIT injection, read-only transaction modes, forbidden operation blocking
- Audit trail: append-only log of every agent operation, queryable via MCP
- Database branching: agents can create sandbox copies to test migrations safely
- Cross-database discovery: a single MCP connection that spans multiple databases
- Alerting: webhooks when agents error, hit rate limits, access PII, or exceed cost thresholds

### 1.2 What We Don't Ship

Clarity on what we refuse to build is as important as what we build.

**No dashboard-first product.** The dashboard exists for human oversight: database list, schema editor, audit log viewer, billing. We do not build a query editor, visual schema designer, data browser, or ERD tool. If a human needs to run SQL, they use their existing tools. Our interface is MCP.

**No client libraries.** We do not write SDKs in Python, TypeScript, Go, etc. Anything that speaks MCP uses our product. The MCP protocol is our SDK.

**No migration framework.** We do not compete with Prisma Migrate, Drizzle Kit, Alembic, or Flyway. Our `propose_migration` and `apply_migration` tools generate standard SQL that is compatible with any existing migration framework. We are the execution layer, not the migration management layer.

**No GraphQL or REST API at launch.** The MCP server is the API. If non-agent integrations need HTTP access, we auto-generate a REST API from the same tool definitions the MCP server uses. We do not hand-build or maintain a separate API surface.

**No AI features that require us to run models.** Schema inference uses Claude API calls, billed to us. We do not train models, fine-tune, or run inference infrastructure. We are a database company that uses AI, not an AI company that does databases.

### 1.3 The Open Source / Commercial Line

This is the most critical product decision. The line must be clean, defensible, and perceived as fair by the developer community.

**In open source (free, Apache 2.0):**

- Full MCP tool surface for Postgres, Redis, ClickHouse (every tool available in cloud is also available in OSS)
- Single-connection auth (one connection string per database, no multi-tenant scoping)
- Schema introspection from pg_catalog / information_schema (reads existing column comments, constraints, types)
- Stdout logging
- Works against any existing database instance — self-hosted, AWS RDS, Supabase, Neon, anything
- Excellent for local development, single-agent prototyping, CI/CD pipelines

**In cloud only (paid):**

- Managed database provisioning with connection pooling (PgBouncer)
- Semantic schema layer (LLM-inferred descriptions, structured metadata beyond what pg_catalog provides)
- Capability-based auth with scoped JWT tokens and introspection
- Query guardrails and cost estimation
- Audit logging to ClickHouse with retention policies
- Database branching
- Cross-database MCP (single connection discovers multiple databases)
- Webhook alerting
- Team management, SSO, SOC 2

**Why this line works:** A developer using the OSS server locally will hit real friction when moving to production. They can't give their customer-support agent and their analytics agent the same database password. They can't figure out which agent corrupted a row because there's no audit log. Their agent misreads `usr_flg_2` because there are no descriptions. Each of these is a genuine pain point, not an artificial gate.

**Why this line is defensible:** The semantic layer requires LLM inference (costs money to generate). The auth system requires a token-issuing service (stateful, needs uptime). The audit log requires a durable append-only store (infrastructure). None of these are things a contributor would casually add to a fork and self-host.

---

## Part 2: MCP Tool Surface (Complete Specification)

This is the canonical list of tools the MCP server exposes. Both OSS and Cloud implement the same tool names and signatures. Cloud tools return richer responses (semantic descriptions, cost estimates) but the interface is identical.

### 2.1 PostgreSQL Tools

**Discovery:**

`list_tables` — Returns all tables the agent has access to, with row counts, column counts, and descriptions (if available). Cloud adds: semantic descriptions, relationship graph, sensitivity tags.

`describe_table(table_name)` — Returns full schema: column names, types, nullability, defaults, constraints, foreign keys, indexes, and descriptions. Cloud adds: natural-language descriptions, example values, enum explanations, common query patterns.

`search_schema(query)` — Natural-language search across table and column names and descriptions. "Find anything related to customer emails." Cloud only (requires semantic layer).

**Reading:**

`query_read(sql_or_intent)` — Accepts either raw SQL SELECT or a natural-language intent ("get the 10 most recent orders for customer X"). If intent, generates SQL and returns it alongside results. Automatically injects LIMIT if none present (default 100, configurable). Cloud adds: cost estimate before execution, column descriptions in response metadata.

`explain_query(sql)` — Runs EXPLAIN ANALYZE and returns the plan with plain-language interpretation. "This query does a sequential scan on a 2M row table — consider adding an index on customer_id."

**Writing:**

`query_write(sql_or_intent)` — INSERT, UPDATE, DELETE. Returns affected row count and a preview of changes before committing (configurable: auto-commit or require confirmation via a follow-up `confirm_write` call). Cloud adds: guardrail enforcement (blocked if scope doesn't allow writes to this table).

`confirm_write(write_id)` — Confirms a pending write operation. Only needed if the agent's credential scope requires write confirmation.

**Schema Management:**

`list_migrations` — Returns migration history (reads from a `_agentdb_migrations` table that the MCP server manages).

`propose_migration(intent)` — Agent describes what it wants ("add a `notes` column to orders, text, nullable"). Server generates SQL DDL, returns it as a diff against current schema. Does not execute.

`apply_migration(migration_id)` — Executes a previously proposed migration. Requires `schema_write` permission scope.

`branch_create(name)` — Creates a copy of the database for safe experimentation. Cloud only (requires provisioning infrastructure).

`branch_list` — Lists active branches with creation time and divergence point.

`branch_merge(branch_name)` — Merges branch back to parent. Detects conflicting schema changes. Cloud only.

`branch_delete(branch_name)` — Drops a branch database. Cloud only.

**Introspection:**

`my_permissions` — Returns the agent's current credential scope: which databases, tables, operations, rate limits, and TTL. In OSS, returns "full access" (single connection string = full permissions).

`query_cost_estimate(sql)` — Returns estimated cost, row count, and execution time without running the query. Cloud only (requires statistics collector).

`recent_activity(options)` — Returns recent operations on this database from the audit log. Filterable by agent, table, operation type, time range. Cloud only.

`suggest_index(sql)` — Given a slow query, analyzes the plan and suggests index creation. Returns the CREATE INDEX statement and estimated improvement.

### 2.2 Redis Tools

`redis_get(key)` — Get a key's value with type detection (string, hash, list, set, sorted set).

`redis_set(key, value, ttl?)` — Set a key with optional TTL.

`redis_delete(key)` — Delete a key.

`redis_list_keys(pattern)` — List keys matching a glob pattern. Includes type and TTL for each. Uses SCAN internally (never KEYS).

`redis_inspect(key)` — Returns type, TTL, memory usage, encoding, and value preview.

`redis_hash_ops(key, operation, field?, value?)` — HGET, HSET, HDEL, HGETALL on hash keys.

`redis_list_ops(key, operation, value?, count?)` — LPUSH, RPUSH, LPOP, RPOP, LRANGE on list keys.

`redis_pub_sub(channel, message?)` — Publish a message or list active channels.

### 2.3 ClickHouse Tools

`ch_list_tables` — Tables with engine type, row count, partition info.

`ch_describe_table(table_name)` — Schema with ClickHouse-specific metadata (partition key, order key, TTL expressions).

`ch_query_read(sql)` — SELECT with automatic LIMIT. Returns estimated bytes scanned.

`ch_insert(table, rows)` — Batch insert. Accepts JSON array of row objects.

`ch_recent_queries` — Recent query log with execution stats.

### 2.4 Cross-Cutting Tools (Cloud Only)

`list_databases` — All databases across types (Postgres, Redis, ClickHouse) that the agent has access to, with descriptions and connection status.

`cross_query(intent)` — Natural-language query that spans multiple databases. "Get the customer from Postgres, their recent page views from ClickHouse, and their session data from Redis." Generates and executes queries against each database, joins results.

---

## Part 3: Semantic Schema Layer (Detailed Design)

### 3.1 Data Model

Every database object carries structured metadata:

```
Table Metadata:
  name: string
  description: string (natural language, 1-3 sentences)
  owner: string (team or person responsible)
  sensitivity: enum [public, internal, contains_pii, contains_phi, restricted]
  created_at: timestamp
  row_count_approx: integer
  update_frequency: string ("~1000 inserts/day", "batch updated nightly")
  common_queries: array of {intent: string, sql: string}

Column Metadata:
  name: string
  description: string (natural language)
  semantic_type: enum [identifier, timestamp, currency, email, phone, url, 
                       enum, free_text, counter, percentage, foreign_key, json_blob]
  example_values: array of string (3-5 representative values)
  pii: boolean
  unit: string (nullable — "cents_usd", "seconds", "bytes")
  enum_values: array of string (if applicable)
  enum_descriptions: map of string→string (what each enum value means)
  never_null_in_practice: boolean (even if schema allows null)
  typical_range: {min, max} (for numeric columns)

Relationship Metadata:
  from_table: string
  from_column: string
  to_table: string
  to_column: string
  type: enum [one_to_one, one_to_many, many_to_one, many_to_many]
  description: string ("each order belongs to exactly one customer")
  join_pattern: string (the SQL join clause an agent should use)
```

### 3.2 Population Strategy

**Phase 1 — Auto-inference (on database creation or connection):**

When a database is provisioned or an existing database is connected, run a schema analysis job:

1. Read all tables, columns, constraints, indexes, and existing comments from pg_catalog / information_schema.
2. Sample 100 rows from each table (random sample, not first 100).
3. Send schema + samples to Claude API with a structured prompt: "For each column, infer a natural-language description, semantic type, whether it contains PII, and 3-5 example values. For each table, write a 1-3 sentence description of what it stores."
4. Store results in the semantic schema service.

Cost: ~$0.05-0.15 per table in Claude API calls. For a 50-table database, ~$5 one-time cost. Acceptable as a provisioning cost, not a recurring cost.

Accuracy: Expect 70-80% accuracy on descriptions, 90%+ on semantic types and PII detection. The remaining 20-30% requires human refinement.

**Phase 2 — Human refinement:**

Dashboard UI with inline editing. For each table, show the auto-generated metadata side-by-side with the actual schema. Developer can edit descriptions, correct PII flags, add enum descriptions. Changes are versioned.

Prioritize refinement by usage: the semantic schema service tracks which tables and columns agents query most frequently. Surface the most-queried, least-described objects first in a "needs attention" queue.

**Phase 3 — Agent-assisted refinement:**

When an agent interacts with a column and discovers something the metadata doesn't capture (e.g., the agent notices that `status` has a value `archived` that isn't in the enum list), the agent can call a `suggest_schema_update` tool. This queues a suggestion for human review.

Over time, this creates a feedback loop: agents use the schema, discover gaps, suggest improvements, humans approve, schema gets better, agents perform better.

### 3.3 How Agents Consume the Semantic Layer

The semantic layer is not a separate API. It's embedded in every tool response.

When an agent calls `describe_table("orders")`, the response includes:

```json
{
  "table": "orders",
  "description": "Customer purchase orders. One row per order, created at checkout.",
  "sensitivity": "contains_pii",
  "row_count": 1847293,
  "columns": [
    {
      "name": "id",
      "type": "uuid",
      "description": "Unique order identifier",
      "semantic_type": "identifier",
      "example_values": ["ord_a1b2c3", "ord_d4e5f6"],
      "pii": false
    },
    {
      "name": "total_cents",
      "type": "integer",
      "description": "Order total in US cents. Divide by 100 for dollars.",
      "semantic_type": "currency",
      "unit": "cents_usd",
      "typical_range": {"min": 499, "max": 250000},
      "pii": false
    }
  ],
  "relationships": [
    {
      "column": "customer_id",
      "references": "customers.id",
      "type": "many_to_one",
      "description": "The customer who placed this order",
      "join_pattern": "JOIN customers ON orders.customer_id = customers.id"
    }
  ],
  "common_queries": [
    {
      "intent": "Recent orders for a customer",
      "sql": "SELECT * FROM orders WHERE customer_id = $1 ORDER BY created_at DESC LIMIT 20"
    }
  ]
}
```

When an agent calls `query_read`, the response includes column descriptions alongside the data:

```json
{
  "columns": [
    {"name": "total_cents", "description": "Order total in US cents", "unit": "cents_usd"},
    {"name": "status", "description": "Order lifecycle state", "current_value_meaning": "shipped = payment received, item dispatched"}
  ],
  "rows": [...],
  "row_count": 20,
  "query_cost": {"estimated_ms": 12, "rows_scanned": 45000}
}
```

This means agents never have to make a separate call to understand what they're looking at. Every data response is self-describing.

---

## Part 4: Agent Authorization System

### 4.1 Design Principles

1. **Agents are not users.** They don't click "Authorize" in a browser. Credentials are issued programmatically by the developer's backend before an agent session starts.

2. **Least privilege by default.** A credential scope defines exactly what an agent can do. Anything not explicitly granted is denied.

3. **Introspectable.** An agent can ask "what am I allowed to do?" and get a structured answer. This lets agents adapt their behavior instead of failing on 403s.

4. **Short-lived.** Default TTL is 1 hour. Maximum is 24 hours. No permanent credentials for agents.

5. **Auditable.** Every credential issuance is logged. Every operation records which credential was used.

### 4.2 Credential Scope Specification

```
Credential Scope:
  id: uuid
  agent_identity: string (developer-defined label, e.g., "customer-support-agent")
  databases: array of string (database names or glob patterns)
  permissions: array of Permission
  rate_limit: integer (operations per minute, default 60)
  ttl: duration (default 1h, max 24h)
  can_delegate: boolean (can this agent create sub-credentials?)
  require_write_confirmation: boolean (writes require confirm_write call?)
  ip_allowlist: array of CIDR (optional, for fixed infrastructure)
  
Permission:
  tables: array of string (table names or "*")
  operations: array of enum [read, write, schema_read, schema_write]
  columns: array of string (optional — restrict to specific columns)
  row_filter: string (optional — SQL WHERE clause for row-level filtering)
```

### 4.3 Example Credential Scopes

**Customer support agent:**
```json
{
  "agent_identity": "support-agent-v2",
  "databases": ["prod_commerce"],
  "permissions": [
    {"tables": ["orders", "order_items"], "operations": ["read"]},
    {"tables": ["order_notes"], "operations": ["read", "write"]},
    {"tables": ["customers"], "operations": ["read"], "columns": ["id", "name", "email", "region"], "row_filter": "region = 'US'"}
  ],
  "rate_limit": 60,
  "ttl": "1h",
  "require_write_confirmation": true
}
```

**Data analysis agent:**
```json
{
  "agent_identity": "weekly-report-agent",
  "databases": ["prod_commerce", "analytics_warehouse"],
  "permissions": [
    {"tables": ["*"], "operations": ["read"]}
  ],
  "rate_limit": 200,
  "ttl": "4h",
  "require_write_confirmation": false
}
```

**Migration agent (branches only):**
```json
{
  "agent_identity": "migration-agent",
  "databases": ["branch_*"],
  "permissions": [
    {"tables": ["*"], "operations": ["read", "write", "schema_read", "schema_write"]}
  ],
  "rate_limit": 30,
  "ttl": "30m",
  "can_delegate": false
}
```

### 4.4 Token Issuance Flow

1. Developer's backend calls AgentDB Cloud API: `POST /v1/credentials` with the scope definition and their API key.
2. AgentDB returns a signed JWT containing the scope. The JWT is self-contained — the MCP gateway can validate and enforce permissions without calling back to the auth service.
3. Developer passes the JWT to their agent as a bearer token when connecting via MCP.
4. The MCP gateway validates the JWT, extracts the scope, and enforces it on every tool call.
5. When the JWT expires, the agent's MCP connection is terminated. The developer's backend must issue a new credential.

### 4.5 Introspection

When an agent calls `my_permissions`, it receives:

```json
{
  "agent_identity": "support-agent-v2",
  "databases": ["prod_commerce"],
  "can_read": ["orders", "order_items", "order_notes", "customers (US only, limited columns)"],
  "can_write": ["order_notes"],
  "cannot": ["schema changes", "delete operations", "access non-US customers"],
  "rate_limit": "60 ops/min (42 remaining this minute)",
  "expires_in": "47 minutes",
  "write_confirmation_required": true
}
```

This response is designed for LLM consumption — it uses natural language alongside structured data so the agent can reason about its constraints.

---

## Part 5: Infrastructure & Architecture

### 5.1 System Components

```
┌────────────────────────────────────────────────────────────┐
│                        Load Balancer                        │
│                    (Caddy or nginx, TLS)                    │
└────────────┬───────────────────────────────────┬────────────┘
             │                                   │
     ┌───────▼────────┐                 ┌────────▼────────┐
     │  MCP Gateway   │                 │   REST API      │
     │  (Go, stateless│                 │  (Go, stateless │
     │   SSE/WebSocket│                 │   credential    │
     │   pool of N)   │                 │   mgmt, billing)│
     └───┬────┬───┬───┘                 └────────┬────────┘
         │    │   │                              │
    ┌────▼┐ ┌─▼──┐ ┌▼─────┐          ┌──────────▼──────────┐
    │ PG  │ │Red-│ │Click-│          │  Metadata Store     │
    │Pools│ │is  │ │House │          │  (Postgres — semantic│
    │     │ │    │ │      │          │   schemas, scopes,   │
    └──┬──┘ └─┬──┘ └──┬───┘          │   billing, teams)   │
       │      │       │              └─────────────────────┘
    ┌──▼──┐ ┌─▼──┐ ┌──▼───┐
    │Cust.│ │Cust.│ │Cust. │          ┌─────────────────────┐
    │PG   │ │Redis│ │CH    │          │  Audit Log          │
    │Inst.│ │Inst.│ │Inst. │          │  (ClickHouse —      │
    └─────┘ └────┘ └──────┘          │   append-only)      │
                                      └─────────────────────┘
```

### 5.2 MCP Gateway (The Core Service)

Language: Go. Rationale: single static binary, excellent concurrency (goroutines for SSE connections), fast compile, large hiring pool. Rust would be marginally faster but harder to hire for and slower to iterate.

Responsibilities:
- Accept MCP connections via SSE (initially) and WebSocket (later)
- Authenticate via JWT validation (no auth service call on hot path)
- Route tool calls to appropriate database backend
- Enforce permission scopes, rate limits, and guardrails
- Inject semantic metadata into responses (fetched from metadata store, cached aggressively)
- Write audit log entries asynchronously (buffered, batched writes to ClickHouse)

Scaling: Horizontally scalable. Each gateway instance is stateless. SSE connections are pinned to an instance for their lifetime but reconnect to any instance if dropped. Target: 10K concurrent MCP connections per instance.

Connection pooling: Every gateway instance maintains a PgBouncer-like internal connection pool to each customer database. Agents open MCP connections to the gateway; the gateway multiplexes onto a small pool of actual database connections. This is non-negotiable — without it, 50 concurrent agent sessions would exhaust PostgreSQL's default 100-connection limit.

### 5.3 Database Provisioning

**PostgreSQL:** Dedicated VM per customer database (shared-nothing isolation). Patroni for high availability with synchronous replication to one standby. PgBouncer in transaction mode co-located on the VM. Automated backups via pgBackRest to object storage (S3-compatible, Hetzner Object Storage for cost).

**Redis:** Dedicated process per customer, potentially multiple per VM for smaller instances. Redis Sentinel for HA. Persistence via RDB snapshots + AOF.

**ClickHouse:** Shared cluster with tenant isolation via database-level separation. ReplicatedMergeTree for durability. This is the one component where multi-tenancy at the database engine level makes sense — ClickHouse is expensive to run and most tenants will have low volume.

### 5.4 Infrastructure Provider Strategy

**Launch on Hetzner dedicated servers.** A Hetzner AX52 (AMD Ryzen 9, 128GB RAM, 2x1TB NVMe) costs €77/month. An equivalent AWS instance (r6a.4xlarge + gp3 storage) costs ~$900/month. That's an 11x cost difference. For a database company where compute is the primary cost, this is existential — running on AWS means 60-70% of revenue goes to Amazon.

**Add AWS as an enterprise option.** Enterprise customers with compliance requirements (data residency, AWS PrivateLink, specific regions) get AWS-hosted instances at a premium. This is a straightforward surcharge: "enterprise tier includes AWS hosting."

**Hetzner risks and mitigations:**
- No managed Kubernetes → run k3s for stateless services, VMs for databases
- No managed load balancer → Caddy or nginx on dedicated instances, Hetzner has basic LB service
- Limited regions (Germany, Finland, US-East) → sufficient for launch, add regions via OVH or Vultr if needed
- No SOC 2 inheritance → we pursue our own SOC 2 (harder but doable, many startups do this on Hetzner)
- DDoS protection is basic → Cloudflare in front for HTTP, direct IP protection is Hetzner's weak point

### 5.5 Branching Architecture

Database branching lets agents create sandbox copies to test migrations, experiment with data, or run destructive operations safely.

**Implementation (practical, not elegant):**

1. `branch_create` triggers a `pg_dump` of the source database and `pg_restore` into a new database on the same or different VM. For a 1GB database, this takes ~30 seconds.
2. The branch gets its own connection pool and appears as a separate database in the MCP tool responses.
3. `branch_merge` diffs the schemas of branch and parent using `migra` (open-source schema diff tool), generates migration SQL, and applies it to the parent after human or agent confirmation.
4. `branch_delete` drops the branch database and reclaims storage.

**Why not copy-on-write (like Neon)?** Copy-on-write branching requires a custom storage engine (Neon rebuilt Postgres storage from scratch). We don't have the engineering resources for that. `pg_dump`/`pg_restore` is slow for large databases but works reliably for the database sizes our initial customers will have (<10GB). If branching becomes a killer feature, we can invest in a faster approach later.

**Limits:** 5 concurrent branches on Pro tier, unlimited on Team. Branches auto-delete after 7 days of inactivity.

---

## Part 6: Pricing & Unit Economics

### 6.1 Tier Structure

**Sandbox (Free)**
- 1 PostgreSQL (500MB), 1 Redis (50MB)
- 100K query-operations/month
- 5 credential scopes
- 7-day audit log
- Rate limit: 30 ops/minute
- No branching, no ClickHouse, no cross-database
- Purpose: local development parity, adoption funnel

**Pro ($29/month per database cluster)**
- PostgreSQL (10GB) + Redis (1GB) bundled
- 1M query-operations/month, then $0.50/100K
- Unlimited credential scopes
- 90-day audit log
- Rate limit: 120 ops/minute
- 5 concurrent branches
- Semantic schema layer
- Query guardrails
- $0.10/GB storage overage

**Team ($99/month base)**
- Unlimited database clusters: $19/mo per Postgres, $9/mo per Redis, $29/mo per ClickHouse
- 10M query-operations/month, then $0.30/100K
- 1-year audit log
- Rate limit: 300 ops/minute
- Unlimited branches
- Cross-database MCP
- Team access controls
- Webhook alerting
- Priority support

**Enterprise (Custom)**
- AWS hosting option
- SOC 2, SSO (SAML/OIDC), dedicated instances
- Custom SLAs (99.95%+ uptime)
- On-prem MCP gateway option
- Audit log export to customer's SIEM
- Dedicated support engineer
- Volume discounts

### 6.2 Unit Economics (Per-Customer)

**Sandbox customer on Hetzner:**
- Infrastructure cost: ~$2/month (fractional VM share, 500MB storage)
- Revenue: $0
- Acceptable loss for funnel. Cap at 10K sandbox accounts initially.

**Pro customer on Hetzner:**
- Infrastructure cost: ~$8-12/month (dedicated PG process on shared VM, Redis process, storage, backups, bandwidth)
- Revenue: $29/month
- Gross margin: ~60-70%
- Breakeven at ~50 Pro customers for one engineer's salary

**Team customer on Hetzner (3 PG, 2 Redis, 1 CH):**
- Infrastructure cost: ~$40-60/month
- Revenue: $99 + $57 + $18 + $29 = $203/month
- Gross margin: ~70-75%

**Enterprise customer on AWS:**
- Infrastructure cost: $300-800/month (dedicated instances, cross-region replication)
- Revenue: $2,000-10,000/month (custom pricing)
- Gross margin: 60-85% depending on configuration

### 6.3 Revenue Projections (18-Month)

Assumptions: 2-person founding team, launch OSS at month 1, cloud at month 3.

| Month | Sandbox | Pro | Team | Enterprise | MRR |
|-------|---------|-----|------|-----------|-----|
| 1-2   | 300     | 0   | 0    | 0         | $0 (OSS only) |
| 3     | 800     | 5   | 0    | 0         | $145 |
| 4     | 1,500   | 15  | 1    | 0         | $534 |
| 5     | 2,500   | 30  | 2    | 0         | $1,068 |
| 6     | 4,000   | 60  | 5    | 0         | $2,235 |
| 9     | 8,000   | 150 | 15   | 1         | $7,835 |
| 12    | 15,000  | 400 | 50   | 3         | $25,550 |
| 18    | 30,000  | 1,000| 150 | 8         | $73,650 |

These are conservative. The business is not self-sustaining from revenue for at least 12-18 months. Plan for $500K-1M in runway (seed funding or bootstrapped savings) to reach profitability.

---

## Part 7: Build Plan (Week-by-Week, First 6 Months)

### Month 1: OSS MCP Server for PostgreSQL

**Week 1:**
- Set up Go project structure, CI/CD (GitHub Actions), release pipeline (GoReleaser)
- Implement MCP protocol handler (SSE transport)
- Implement `list_tables` and `describe_table` tools reading from pg_catalog
- Single connection string auth (env var)
- Test with Claude Desktop MCP integration

**Week 2:**
- Implement `query_read` with automatic LIMIT injection
- Implement `query_write` with affected row count
- Implement `explain_query` with plain-language interpretation
- Stdout structured logging (JSON)
- README, quickstart guide, demo video

**Week 3:**
- Implement `propose_migration` and `apply_migration` (migration tracking table)
- Implement `my_permissions` (returns "full access" in OSS)
- Implement `suggest_index`
- Integration tests against PostgreSQL 14, 15, 16
- Docker image published to Docker Hub and GHCR

**Week 4:**
- Launch on GitHub, Hacker News, Reddit r/programming, r/LocalLLaMA
- Write integration guides for LangChain, LlamaIndex, CrewAI
- Submit PRs to framework docs to reference agentdb-mcp
- Gather feedback from first 50-100 users
- Target: 500 GitHub stars by end of month 1

### Month 2: Redis + ClickHouse MCP Servers

**Week 5-6:**
- Implement Redis MCP tools (get, set, delete, list_keys, inspect, hash_ops, list_ops)
- Implement ClickHouse MCP tools (list_tables, describe_table, query_read, insert)
- Multi-database support in single MCP server (configure PG + Redis + CH in one config file)
- Publish updated release

**Week 7-8:**
- Begin cloud infrastructure setup: Hetzner dedicated server provisioning automation (Ansible or Terraform + Hetzner API)
- PgBouncer configuration templates
- Patroni HA setup for PostgreSQL
- Redis Sentinel configuration
- Internal metadata database schema (customers, databases, credential_scopes, audit_log)
- Design dashboard wireframes (minimal: database list, schema viewer, audit log)

### Month 3: Cloud MVP Launch

**Week 9-10:**
- MCP Gateway service: JWT auth, permission enforcement, rate limiting
- Credential issuance API (`POST /v1/credentials`)
- Database provisioning API (create/delete Postgres and Redis instances)
- Audit log writing (to PostgreSQL initially, migrate to ClickHouse in month 6)
- PgBouncer auto-configuration per customer database

**Week 11-12:**
- Dashboard: login, database list, connection details, credential management
- Semantic schema auto-inference (Claude API integration)
- Schema editor in dashboard
- Billing integration (Stripe, usage metering for query-operations)
- Launch cloud beta to 20 waitlist users
- Write cloud documentation

### Month 4: Auth Hardening + Guardrails

**Week 13-14:**
- Row-level filtering in credential scopes
- Column-level access control
- Write confirmation flow (`require_write_confirmation` scope option)
- Query cost estimation (EXPLAIN-based, before execution)
- Automatic LIMIT enforcement per scope

**Week 15-16:**
- Forbidden operation blocking (DROP TABLE, TRUNCATE, etc. unless explicitly scoped)
- Rate limiting with sliding window (per credential, not per connection)
- `search_schema` tool (requires semantic layer)
- Agent activity dashboard (which agents did what, when)
- Webhook alerting (error rate, rate limit hits, PII access)

### Month 5: Branching + Polish

**Week 17-18:**
- Database branching: `branch_create`, `branch_list`, `branch_merge`, `branch_delete`
- pg_dump/pg_restore automation with progress tracking
- Schema diff via migra for branch merging
- Branch auto-cleanup (7-day inactivity)

**Week 19-20:**
- Public launch: remove waitlist, open signups
- ProductHunt launch
- Hacker News Show HN
- Framework integration packages updated for cloud
- Pricing page, comparison page, docs site
- Blog posts: "Why agents need their own database layer", "Building agent auth that actually works"

### Month 6: ClickHouse Cloud + Cross-Database

**Week 21-22:**
- ClickHouse provisioning in cloud
- Audit log migration from PostgreSQL to ClickHouse
- `recent_activity` tool backed by ClickHouse
- Query analytics dashboard (slowest queries, most active agents)

**Week 23-24:**
- Cross-database MCP: `list_databases`, `cross_query`
- `cross_query` implementation: parse intent, generate queries per database, execute in parallel, join results
- Performance optimization: semantic schema caching, connection pool tuning
- Retrospective: what's working, what's not, what do customers actually use

---

## Part 8: Success Metrics

### 8.1 OSS Metrics (Leading Indicators)

| Metric | Month 1 | Month 3 | Month 6 | Month 12 |
|--------|---------|---------|---------|----------|
| GitHub stars | 500 | 2,000 | 5,000 | 15,000 |
| Monthly active installations (telemetry opt-in) | 100 | 500 | 2,000 | 8,000 |
| Framework integrations referencing us | 2 | 5 | 10 | 20 |
| Contributors (non-team) | 5 | 15 | 30 | 50 |
| Docker pulls / binary downloads per month | 300 | 2,000 | 10,000 | 50,000 |

### 8.2 Cloud Metrics (Business Health)

| Metric | Month 3 | Month 6 | Month 12 | Month 18 |
|--------|---------|---------|----------|----------|
| Registered accounts | 100 | 1,000 | 5,000 | 15,000 |
| Paying customers | 5 | 65 | 453 | 1,158 |
| MRR | $145 | $2,235 | $25,550 | $73,650 |
| Sandbox → Pro conversion rate | 3% | 4% | 5% | 6% |
| Pro → Team upgrade rate | — | 5% | 8% | 10% |
| Monthly churn (Pro) | — | 8% | 5% | 4% |
| Query-operations per paying customer per month | 50K | 150K | 400K | 800K |

### 8.3 Product-Market Fit Signals

Watch for these qualitative signals more than the numbers above:

- Developers share their AgentDB setup unprompted on Twitter/X
- Framework maintainers proactively integrate us (not just accepting our PRs)
- Customers hit tier limits and upgrade (vs. churning)
- Support tickets shift from "how do I set this up" to "how do I do more advanced things"
- Customers ask for features we planned but haven't built yet (branching, ClickHouse, cross-database)
- Enterprise inbound starts without outbound sales effort

### 8.4 Kill Criteria

Be honest about when to pivot or shut down:

- **Month 6:** If OSS has <1,000 stars and <200 active installations, the market doesn't want an MCP database server. Pivot or stop.
- **Month 9:** If cloud has <20 paying customers, the conversion funnel is broken. Either the OSS→Cloud gap is too wide or the paid features aren't valued.
- **Month 12:** If MRR is <$10K, the business cannot sustain even a 2-person team. Raise funding on the OSS traction or wind down the cloud product.
- **Month 18:** If MRR is <$50K, growth is too slow for a venture-scale outcome. Consider pivoting to a services/consulting model around the OSS project, or selling the project/company.

---

## Part 9: Team & Hiring Plan

### 9.1 Founding Team (Month 1-6)

Two people. One backend-infrastructure engineer (Go, PostgreSQL internals, Linux ops) and one product-engineer (dashboard, docs, developer experience, community). Both write code. Neither is a full-time manager. If there's only one founder, hire the other role within month 1.

### 9.2 First Hires (Month 6-12)

**Hire 3: Infrastructure/SRE engineer.** As customer count grows past 50, the operational burden of managing database instances, handling incidents, and scaling infrastructure exceeds what two people can handle alongside feature development. This person owns provisioning automation, monitoring (Prometheus + Grafana), backup verification, and incident response.

**Hire 4: Developer advocate.** The OSS project needs a full-time community presence: writing tutorials, presenting at meetups, engaging on Discord/GitHub, producing demo content. This role directly drives the top of the conversion funnel. Hire someone who has built agents and can credibly demonstrate the product.

### 9.3 Month 12-18

**Hire 5: Second backend engineer.** Feature development velocity on the cloud product.

**Hire 6: Designer.** Dashboard, docs site, marketing site. Until this hire, use templates and keep design minimal.

**Do not hire:** Sales (until enterprise inbound justifies it), marketing (developer advocate covers this), a CTO (one of the founders is the technical leader), customer success (support is everyone's job until 200+ paying customers).

---

## Part 10: Risks & Mitigations

### 10.1 Market Timing Risk

**Risk:** Autonomous agents plateau at simple tasks and never need database access at scale. The "agents managing databases" future is 5 years away, not 2.

**Mitigation:** The bridge market. AI-assisted developers (Cursor, Claude Code, GitHub Copilot users) benefit from self-describing databases today. Position the product for this audience while building for the autonomous future. If the bridge market sustains the business, the autonomous future is upside, not a requirement.

**Signal to watch:** Are agent framework monthly downloads growing >20% quarter-over-quarter? If LangChain/CrewAI/Autogen usage flattens, the market is telling you something.

### 10.2 Incumbent Response Risk

**Risk:** Supabase ships an MCP server, Neon adds semantic schema, and your differentiation evaporates.

**Mitigation:** Speed and focus. Incumbents have existing customers, existing roadmaps, and existing architectures that constrain them. Supabase's MCP server will be an add-on to their dashboard-centric product, not a rethinking of their architecture. Your advantage is that every decision — from auth to schema to API design — is made for agents. This compounds over time. By the time incumbents respond, you should be 12+ months ahead on the agent-native experience.

**Worst case mitigation:** If an incumbent ships 80% of your features, pivot to being the "agent auth and observability layer" that works with any database. Your auth model and audit trail are portable — they don't require customers to move their database to you.

### 10.3 Open Source Sustainability Risk

**Risk:** The OSS project gets large, community demands grow, and maintaining it consumes all engineering time at the expense of the commercial product.

**Mitigation:** Company-backed open source from day one. You write all the code, you set the roadmap, you accept issues but not major PRs (unless they're exceptional). This is the Neon/Turso/Supabase model. Be explicit in CONTRIBUTING.md that this is a company-led project, not a community-led one. Accept bug fixes and small improvements. Decline architectural changes and major features unless they align with your roadmap.

### 10.4 Security Risk

**Risk:** A vulnerability in the MCP gateway or auth system leads to unauthorized database access. This is an existential threat for a database company.

**Mitigation:** 
- JWT validation is standard, well-audited code — don't roll your own
- Permission enforcement is deny-by-default — if the scope doesn't explicitly grant access, the request is denied
- All customer database connections use TLS
- Automated security scanning (Snyk, GoSec) in CI
- Hire a penetration testing firm before public launch (budget $10-15K)
- Bug bounty program after month 6

### 10.5 Hetzner Risk

**Risk:** Hetzner has a major outage, or a customer requires AWS/GCP and you can't deliver.

**Mitigation:** Design for provider portability from day one. Provisioning scripts should be parameterized by provider. Database VMs should be configured identically regardless of where they run. Keep provider-specific code isolated to a thin provisioning layer. Adding a second provider should be a 2-week project, not a 2-month one.

---

## Appendix A: Competitive Landscape Matrix

| Feature | AgentDB | Supabase | Neon | Turso | Railway |
|---------|---------|----------|------|-------|---------|
| MCP server (primary interface) | Core product | Not built | Not built | Not built | Not built |
| Semantic schema layer | Core product | Not built | Not built | Not built | Not built |
| Agent auth (capability-based) | Core product | RLS (human-oriented) | Not built | Not built | Not built |
| Query guardrails | Core product | Not built | Not built | Not built | Not built |
| Audit trail (agent-aware) | Core product | Basic logs | Basic logs | Not built | Not built |
| Database branching | pg_dump based | Not built | Copy-on-write | Not built | Not built |
| PostgreSQL | Yes (managed) | Yes (managed) | Yes (managed) | No (SQLite) | Yes (addon) |
| Redis | Yes (managed) | No | No | No | Yes (addon) |
| ClickHouse | Yes (managed) | No | No | No | No |
| Open-source core | Yes (Apache 2.0) | Yes (Apache 2.0) | Yes (Apache 2.0) | Yes (MIT) | No |
| Self-hostable | OSS server yes | Yes | Complex | Yes | No |
| Dashboard | Minimal (oversight) | Extensive | Moderate | Minimal | Moderate |
| Client libraries | None (MCP is the SDK) | JS, Python, etc. | Standard PG drivers | JS, Python, etc. | Standard drivers |
| Target user | Agents + AI-assisted devs | Human developers | Human developers | Edge developers | Human developers |

## Appendix B: MCP Server Configuration (OSS)

```yaml
# agentdb.yaml — configuration file for agentdb-mcp server

server:
  port: 8765
  transport: sse  # or "websocket" when supported
  log_level: info
  log_format: json

databases:
  - name: main
    type: postgresql
    connection_string: postgres://user:pass@localhost:5432/mydb
    read_only: false
    max_query_rows: 1000
    query_timeout: 30s

  - name: cache
    type: redis
    connection_string: redis://localhost:6379/0

  - name: analytics
    type: clickhouse
    connection_string: clickhouse://localhost:9000/analytics
    read_only: true

# Optional: restrict which tools are exposed
tools:
  enabled:
    - list_tables
    - describe_table
    - query_read
    - query_write
    - explain_query
    - propose_migration
    - apply_migration
    - my_permissions
    - suggest_index
    # Redis tools auto-enabled for redis databases
    # ClickHouse tools auto-enabled for clickhouse databases
  disabled:
    - raw_sql  # disabled by default, enable explicitly if needed
```

## Appendix C: Semantic Schema Inference Prompt (Claude API)

```
You are analyzing a database schema to generate natural-language metadata 
for each table and column. This metadata will be consumed by AI agents 
that need to understand the database structure to query it correctly.

For each table, provide:
1. A 1-3 sentence description of what the table stores and its role
2. Sensitivity classification: public, internal, contains_pii, contains_phi, restricted
3. Approximate update frequency based on table structure

For each column, provide:
1. A natural-language description (1 sentence)
2. Semantic type: identifier, timestamp, currency, email, phone, url, enum, 
   free_text, counter, percentage, foreign_key, json_blob, boolean_flag, 
   file_path, ip_address, geographic, measurement
3. Whether it likely contains PII (true/false)
4. Unit of measurement if applicable (e.g., cents_usd, seconds, bytes, 
   kilograms, meters)
5. 3-5 example values based on the sample data provided

Schema:
{schema_ddl}

Sample data (100 random rows per table):
{sample_data}

Respond in JSON format matching this structure exactly:
{output_schema}
```
