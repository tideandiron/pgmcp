# pgmcp Demo Database

A pre-built e-commerce database for evaluating all 15 pgmcp tools against realistic data.

## What's inside

| Object | Schema | Rows |
|--------|--------|------|
| `customers` | public | 200 |
| `products` | public | 100 |
| `orders` | public | 2,000 |
| `order_items` | public | 5,000 |
| `reviews` | public | 500 |
| `inventory_events` | analytics | 3,000 |
| `change_log` | audit | 1,000 |
| `active_customers` (view) | public | — |
| `order_summary` (view) | public | — |
| `daily_revenue` (mat. view) | analytics | — |

**Enums:** `order_status`, `payment_method`, `product_category`

**Extensions:** `pgcrypto`, `pg_trgm`

**Roles:** `demo_reader` — SELECT on public + analytics, no access to audit schema

Indexes are **deliberately absent** on `orders(status)` and `orders(customer_id)` so that `suggest_index` produces actionable recommendations.

## Starting the demo

```bash
docker compose up -d
```

PostgreSQL starts on port 5432. The seed script runs automatically on first boot via `/docker-entrypoint-initdb.d/`. pgmcp starts in SSE mode on port 3000.

To reset to a clean state:

```bash
docker compose down -v && docker compose up -d
```

## Connecting pgmcp

With the default `docker-compose.yml`, pgmcp connects to PostgreSQL automatically. To connect your MCP client:

- **SSE endpoint:** `http://localhost:3000/mcp`
- **Direct psql:** `psql postgres://pgmcp:pgmcp@localhost:5432/pgmcp`
- **As demo_reader:** `psql postgres://demo_reader:demo_reader@localhost:5432/pgmcp`

## Tool walkthrough

### Discovery tools

**list_schemas** — see all three schemas:

```json
{}
```

Expected: `public`, `analytics`, `audit`

---

**list_tables** — all table kinds in public:

```json
{"schema": "public", "kind": "all"}
```

Expected: 5 tables + 2 views

---

**list_enums** — the three domain enums:

```json
{"schema": "public"}
```

Expected: `order_status` (6 values), `payment_method` (5 values), `product_category` (7 values)

---

**describe_table** — full column introspection with inferred descriptions:

```json
{"schema": "public", "table": "customers"}
```

The `customers` table exercises most of the infer.rs heuristic patterns: `email`, `password_hash`, `avatar_url`, `phone_number`, `is_verified`, `is_active`, `tags` (array), `metadata` (JSONB), `deleted_at` (soft delete), `last_login_at`.

```json
{"schema": "public", "table": "orders"}
```

Shows enum column types, FK references, JSONB shipping address, and the missing indexes that `suggest_index` will find.

---

**list_extensions** — confirm pgcrypto and pg_trgm are installed:

```json
{}
```

---

**table_stats** — row counts, sizes, and vacuum timestamps:

```json
{"schema": "public", "table": "orders"}
```

---

**server_info** — PostgreSQL version and configuration:

```json
{}
```

---

**list_databases** — all databases on the server:

```json
{}
```

### SQL-accepting tools

**query** — aggregation by product category:

```json
{
  "sql": "SELECT category, count(*) AS cnt, avg(price_cents)/100.0 AS avg_price FROM products GROUP BY category ORDER BY avg_price DESC"
}
```

---

**query** — top 10 customers by spend:

```json
{
  "sql": "SELECT c.username, count(o.id) AS orders, sum(o.total_cents)/100.0 AS total_spent FROM customers c JOIN orders o ON o.customer_id = c.id GROUP BY c.id, c.username ORDER BY total_spent DESC LIMIT 10"
}
```

---

**explain** — seq scan demo (no index on `status`):

```json
{
  "sql": "SELECT * FROM orders WHERE status = 'pending' AND created_at > now() - interval '30 days'"
}
```

Expected: sequential scan on `orders` because no index exists on `status`. The partial index `idx_orders_pending` only covers `created_at` for pending rows, so the planner may still scan.

---

**suggest_index** — missing index on `orders.status`:

```json
{"sql": "SELECT * FROM orders WHERE status = 'pending'"}
```

Expected recommendation: `CREATE INDEX CONCURRENTLY ON orders(status)`

---

**suggest_index** — missing index on `orders.customer_id`:

```json
{"sql": "SELECT * FROM orders WHERE customer_id = 42"}
```

Expected recommendation: `CREATE INDEX CONCURRENTLY ON orders(customer_id)`

---

**propose_migration** — add a column with safety analysis:

```json
{"sql": "ALTER TABLE customers ADD COLUMN loyalty_tier TEXT DEFAULT 'bronze'"}
```

Expected: migration plan with lock analysis and downtime risk assessment.

### Introspection tools

**my_permissions** — what the current role can do:

```json
{"schema": "public"}
```

---

**my_permissions as demo_reader** — connect via `postgres://demo_reader:demo_reader@localhost:5432/pgmcp` then:

```json
{"schema": "audit"}
```

Expected: no privileges on the `audit` schema, demonstrating the permission boundary.

---

**connection_info** — host, port, SSL status, pool statistics:

```json
{}
```

---

**health** — liveness with latency measurement:

```json
{}
```

## Materialized view

`analytics.daily_revenue` aggregates monthly order revenue. It is populated by `REFRESH MATERIALIZED VIEW` at the end of the seed script. Query it directly:

```sql
SELECT month, total_orders, revenue_cents / 100.0 AS revenue_usd
FROM analytics.daily_revenue
ORDER BY month DESC;
```

## Seq scan targets for demos

These queries will produce sequential scans and are ideal for `explain` / `suggest_index` demos:

```sql
-- No index on status
SELECT * FROM orders WHERE status = 'shipped';

-- No index on customer_id
SELECT * FROM orders WHERE customer_id = 1;

-- No index on event_type in analytics schema
SELECT * FROM analytics.inventory_events WHERE event_type = 'restock';
```
