-- =============================================================================
-- demo/seed.sql
--
-- E-commerce demo database for pgmcp.
-- Auto-loaded by docker-compose via /docker-entrypoint-initdb.d/01-seed.sql
--
-- Compatible with PostgreSQL 14-17.
-- =============================================================================

-- ─────────────────────────────────────────────────────────────────────────────
-- Extensions
-- ─────────────────────────────────────────────────────────────────────────────

CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ─────────────────────────────────────────────────────────────────────────────
-- Schemas
-- ─────────────────────────────────────────────────────────────────────────────

CREATE SCHEMA IF NOT EXISTS analytics;
CREATE SCHEMA IF NOT EXISTS audit;

-- ─────────────────────────────────────────────────────────────────────────────
-- Enum types
-- ─────────────────────────────────────────────────────────────────────────────

CREATE TYPE order_status AS ENUM (
    'pending', 'confirmed', 'shipped', 'delivered', 'cancelled', 'refunded'
);

CREATE TYPE payment_method AS ENUM (
    'credit_card', 'debit_card', 'paypal', 'bank_transfer', 'crypto'
);

CREATE TYPE product_category AS ENUM (
    'electronics', 'clothing', 'books', 'home', 'sports', 'food', 'toys'
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Tables
-- ─────────────────────────────────────────────────────────────────────────────

-- public.customers — 200 rows
-- Designed to trigger infer.rs heuristic patterns (email, password_hash,
-- avatar_url, phone_number, is_verified, is_active, tags, metadata, etc.)
CREATE TABLE customers (
    id              SERIAL PRIMARY KEY,
    email           TEXT NOT NULL UNIQUE CHECK (email LIKE '%@%'),
    username        TEXT NOT NULL UNIQUE,
    password_hash   TEXT NOT NULL,
    first_name      TEXT,
    last_name       TEXT,
    phone_number    TEXT,
    avatar_url      TEXT,
    bio             TEXT,
    is_verified     BOOLEAN DEFAULT false,
    is_active       BOOLEAN DEFAULT true,
    tags            TEXT[] DEFAULT '{}',
    metadata        JSONB DEFAULT '{}',
    order_count     INTEGER DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at      TIMESTAMPTZ,   -- soft delete
    last_login_at   TIMESTAMPTZ
);

-- public.products — 100 rows
CREATE TABLE products (
    id              SERIAL PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT,
    category        product_category NOT NULL,
    price_cents     INTEGER NOT NULL CHECK (price_cents > 0),
    cost_cents      INTEGER CHECK (cost_cents > 0),
    sku             TEXT NOT NULL UNIQUE,
    is_published    BOOLEAN DEFAULT true,
    weight_grams    INTEGER,
    version         INTEGER DEFAULT 1,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- public.orders — 2000 rows
-- DELIBERATELY no index on status or customer_id (for suggest_index demos)
CREATE TABLE orders (
    id                  SERIAL PRIMARY KEY,
    customer_id         INTEGER NOT NULL REFERENCES customers(id),
    status              order_status NOT NULL DEFAULT 'pending',
    payment_method      payment_method,
    subtotal_cents      INTEGER NOT NULL,
    tax_cents           INTEGER NOT NULL DEFAULT 0,
    shipping_fee_cents  INTEGER NOT NULL DEFAULT 0,
    total_cents         INTEGER NOT NULL CHECK (total_cents > 0),
    shipping_address    JSONB,
    is_flagged          BOOLEAN DEFAULT false,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    confirmed_at        TIMESTAMPTZ,
    shipped_at          TIMESTAMPTZ,
    delivered_at        TIMESTAMPTZ
);

-- public.order_items — 5000 rows
CREATE TABLE order_items (
    id              SERIAL PRIMARY KEY,
    order_id        INTEGER NOT NULL REFERENCES orders(id),
    product_id      INTEGER NOT NULL REFERENCES products(id),
    quantity        INTEGER NOT NULL CHECK (quantity > 0),
    unit_price_cents INTEGER NOT NULL CHECK (unit_price_cents > 0),
    discount_cents  INTEGER DEFAULT 0
);

-- public.reviews — 500 rows
CREATE TABLE reviews (
    id              SERIAL PRIMARY KEY,
    customer_id     INTEGER NOT NULL REFERENCES customers(id),
    product_id      INTEGER NOT NULL REFERENCES products(id),
    rating          INTEGER NOT NULL CHECK (rating BETWEEN 1 AND 5),
    title           TEXT,
    body            TEXT,
    is_verified     BOOLEAN DEFAULT false,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- analytics.inventory_events — 3000 rows
-- No indexes, for seq scan demos
CREATE TABLE analytics.inventory_events (
    id              SERIAL PRIMARY KEY,
    product_id      INTEGER NOT NULL,
    event_type      TEXT NOT NULL,   -- 'restock', 'sale', 'return', 'adjustment'
    quantity_change INTEGER NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- audit.change_log — 1000 rows
-- Restricted schema: demo_reader has no grants here
CREATE TABLE audit.change_log (
    id          SERIAL PRIMARY KEY,
    table_name  TEXT NOT NULL,
    operation   TEXT NOT NULL,   -- 'INSERT', 'UPDATE', 'DELETE'
    row_id      INTEGER,
    changed_by  TEXT NOT NULL DEFAULT current_user,
    changed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    old_values  JSONB,
    new_values  JSONB
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Indexes (selective — some deliberately omitted)
-- ─────────────────────────────────────────────────────────────────────────────

-- Range queries on customers
CREATE INDEX idx_customers_created_at ON customers(created_at);

-- Composite — covers order_items lookups by order and product together
CREATE INDEX idx_order_items_order_product ON order_items(order_id, product_id);

-- Partial — only pending orders need fast range scan by created_at
CREATE INDEX idx_orders_pending ON orders(created_at) WHERE status = 'pending';

-- GIN trigram index on product names — wrapped so it skips gracefully if
-- pg_trgm is unavailable (shouldn't happen, but defensive for edge cases)
DO $$ BEGIN
    CREATE INDEX idx_products_name_trgm ON products USING gin(name gin_trgm_ops);
EXCEPTION WHEN undefined_object THEN
    RAISE NOTICE 'pg_trgm not available, skipping trigram index';
END $$;

-- NOTE: No indexes on orders(status) or orders(customer_id) — those are
-- deliberately absent so suggest_index demos produce actionable recommendations.

-- ─────────────────────────────────────────────────────────────────────────────
-- Views
-- ─────────────────────────────────────────────────────────────────────────────

CREATE VIEW active_customers AS
SELECT id, email, username, order_count, last_login_at, created_at
FROM customers
WHERE is_active = true AND deleted_at IS NULL;

CREATE VIEW order_summary AS
SELECT
    o.id            AS order_id,
    c.email         AS customer_email,
    c.username,
    o.status,
    o.total_cents,
    o.payment_method,
    o.created_at
FROM orders o
JOIN customers c ON c.id = o.customer_id;

-- ─────────────────────────────────────────────────────────────────────────────
-- Materialized view
-- ─────────────────────────────────────────────────────────────────────────────

CREATE MATERIALIZED VIEW analytics.daily_revenue AS
SELECT
    date_trunc('month', o.created_at)::date  AS month,
    count(DISTINCT o.customer_id)            AS unique_customers,
    count(*)                                 AS total_orders,
    sum(o.total_cents)                       AS revenue_cents,
    avg(o.total_cents)::integer              AS avg_order_cents
FROM orders o
WHERE o.status NOT IN ('cancelled', 'refunded')
GROUP BY date_trunc('month', o.created_at)
ORDER BY month;

-- ─────────────────────────────────────────────────────────────────────────────
-- Demo role
-- ─────────────────────────────────────────────────────────────────────────────

DO $$ BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'demo_reader') THEN
        CREATE ROLE demo_reader LOGIN PASSWORD 'demo_reader';
    END IF;
END $$;

GRANT USAGE ON SCHEMA public TO demo_reader;
GRANT USAGE ON SCHEMA analytics TO demo_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO demo_reader;
GRANT SELECT, INSERT ON analytics.inventory_events TO demo_reader;
GRANT SELECT ON analytics.daily_revenue TO demo_reader;
-- No grants on audit schema — demo_reader cannot see it

-- ─────────────────────────────────────────────────────────────────────────────
-- Data: customers (200 rows)
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO customers (
    email, username, password_hash, first_name, last_name,
    phone_number, avatar_url, bio,
    is_verified, is_active,
    tags, metadata, order_count,
    created_at, updated_at, last_login_at, deleted_at
)
SELECT
    'user' || i || '@example.com',
    'user_' || i,
    encode(sha256(('password' || i)::bytea), 'hex'),
    (ARRAY['Alice','Bob','Carol','Dave','Eve','Frank','Grace','Hank','Iris','Jack'])[1 + (i % 10)],
    (ARRAY['Smith','Johnson','Williams','Brown','Jones','Garcia','Miller','Davis','Rodriguez','Martinez'])[1 + ((i * 7) % 10)],
    '+1555' || lpad((1000000 + i)::text, 7, '0'),
    'https://example.com/avatars/' || i || '.jpg',
    CASE WHEN i % 5 = 0 THEN 'Bio for user ' || i ELSE NULL END,
    i % 3 = 0,      -- ~33% verified
    i % 20 != 0,    -- ~95% active; rows where i%20=0 are soft-deleted below
    CASE
        WHEN i % 4 = 0 THEN ARRAY['vip','early-adopter']
        WHEN i % 3 = 0 THEN ARRAY['newsletter']
        ELSE '{}'
    END,
    jsonb_build_object(
        'signup_source', (ARRAY['organic','referral','ad','social'])[1 + (i % 4)],
        'locale',        (ARRAY['en-US','en-GB','de-DE','fr-FR','ja-JP'])[1 + (i % 5)]
    ),
    0,   -- updated after orders are inserted
    now() - ((200 - i) || ' days')::interval,
    now() - ((200 - i) || ' days')::interval + interval '1 hour',
    CASE WHEN i % 3 = 0 THEN now() - ((i % 30) || ' hours')::interval ELSE NULL END,
    CASE WHEN i % 20 = 0 THEN now() - ((i % 60) || ' days')::interval ELSE NULL END
FROM generate_series(1, 200) AS i;

-- ─────────────────────────────────────────────────────────────────────────────
-- Data: products (100 rows)
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO products (
    name, description, category, price_cents, cost_cents,
    sku, is_published, weight_grams, version, created_at
)
SELECT
    (ARRAY[
        'Wireless Headphones', 'Running Shoes', 'Python Cookbook', 'Coffee Maker',
        'Yoga Mat', 'Organic Granola', 'LEGO City Set', 'Denim Jacket',
        'Smart Watch', 'Camping Tent'
    ])[1 + (i % 10)] || ' Model ' || i,
    CASE WHEN i % 3 != 0 THEN 'High quality product ' || i || '. Great for everyday use.' ELSE NULL END,
    (ARRAY[
        'electronics','clothing','books','home','sports',
        'food','toys','electronics','clothing','books'
    ]::product_category[])[1 + (i % 10)],
    -- price: $9.99 to $499.99 in cents
    (999 + (i * 4973) % 49000),
    -- cost: ~60% of price
    ((999 + (i * 4973) % 49000) * 6 / 10),
    'SKU-' || lpad(i::text, 5, '0'),
    i % 10 != 0,    -- 90% published
    CASE WHEN i % 5 != 0 THEN 100 + (i * 37) % 2000 ELSE NULL END,
    1 + (i % 3),
    now() - ((365 - i) || ' days')::interval
FROM generate_series(1, 100) AS i;

-- ─────────────────────────────────────────────────────────────────────────────
-- Data: orders (2000 rows)
-- Status distribution: ~35% delivered, ~20% shipped, ~20% confirmed,
--                      ~15% pending, ~5% cancelled, ~5% refunded
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO orders (
    customer_id, status, payment_method,
    subtotal_cents, tax_cents, shipping_fee_cents, total_cents,
    shipping_address, is_flagged,
    created_at, confirmed_at, shipped_at, delivered_at
)
SELECT
    1 + (i % 200),   -- spread across all 200 customers
    CASE
        WHEN i % 20 IN (0, 1, 2, 3, 4, 5, 6) THEN 'delivered'   -- 35%
        WHEN i % 20 IN (7, 8, 9, 10)         THEN 'shipped'      -- 20%
        WHEN i % 20 IN (11, 12, 13, 14)      THEN 'confirmed'    -- 20%
        WHEN i % 20 IN (15, 16, 17)          THEN 'pending'      -- 15%
        WHEN i % 20 = 18                     THEN 'cancelled'    -- 5%
        ELSE                                      'refunded'     -- 5%
    END::order_status,
    (ARRAY['credit_card','debit_card','paypal','bank_transfer','crypto']::payment_method[])[1 + (i % 5)],
    -- subtotal: $10 to $500 in cents
    (1000 + (i * 1777) % 49000),
    -- tax: 8% of subtotal
    ((1000 + (i * 1777) % 49000) * 8 / 100),
    -- shipping: $0, $499, $999
    (ARRAY[0, 499, 999])[1 + (i % 3)],
    -- total = subtotal + tax + shipping
    (1000 + (i * 1777) % 49000)
        + ((1000 + (i * 1777) % 49000) * 8 / 100)
        + (ARRAY[0, 499, 999])[1 + (i % 3)],
    jsonb_build_object(
        'street',  (i * 13 % 9999 + 1)::text || ' Main St',
        'city',    (ARRAY['New York','Los Angeles','Chicago','Houston','Phoenix','Seattle','Boston','Denver'])[1 + (i % 8)],
        'country', 'US',
        'zip',     lpad((10000 + i % 90000)::text, 5, '0')
    ),
    i % 50 = 0,   -- 2% flagged
    now() - ((2000 - i) || ' hours')::interval,
    -- confirmed_at: set for confirmed, shipped, delivered, refunded
    CASE
        WHEN i % 20 NOT IN (15, 16, 17, 18)
            THEN now() - ((2000 - i) || ' hours')::interval + interval '2 hours'
        ELSE NULL
    END,
    -- shipped_at: set for shipped and delivered
    CASE
        WHEN i % 20 IN (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
            THEN now() - ((2000 - i) || ' hours')::interval + interval '26 hours'
        ELSE NULL
    END,
    -- delivered_at: set only for delivered
    CASE
        WHEN i % 20 IN (0, 1, 2, 3, 4, 5, 6)
            THEN now() - ((2000 - i) || ' hours')::interval + interval '98 hours'
        ELSE NULL
    END
FROM generate_series(1, 2000) AS i;

-- ─────────────────────────────────────────────────────────────────────────────
-- Data: order_items (5000 rows)
-- Each order gets 2-3 items on average; distribute across 2000 orders
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO order_items (
    order_id, product_id, quantity, unit_price_cents, discount_cents
)
SELECT
    1 + (i % 2000),                          -- order_id
    1 + ((i * 37 + i / 2000) % 100),        -- product_id (varied spread)
    1 + (i % 4),                             -- quantity 1-4
    999 + (i * 1973) % 49000,               -- unit price
    CASE WHEN i % 5 = 0 THEN (100 + (i % 500)) ELSE 0 END  -- 20% have discount
FROM generate_series(1, 5000) AS i;

-- ─────────────────────────────────────────────────────────────────────────────
-- Data: reviews (500 rows)
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO reviews (
    customer_id, product_id, rating, title, body, is_verified, created_at
)
SELECT
    1 + (i % 200),
    1 + (i % 100),
    -- ratings: skew positive (4-5 stars dominate)
    CASE
        WHEN i % 10 IN (0, 1)      THEN 5
        WHEN i % 10 IN (2, 3, 4)   THEN 4
        WHEN i % 10 IN (5, 6)      THEN 3
        WHEN i % 10 = 7            THEN 2
        ELSE                            1
    END,
    CASE
        WHEN i % 4 = 0 THEN 'Excellent product!'
        WHEN i % 4 = 1 THEN 'Good value for money'
        WHEN i % 4 = 2 THEN 'Meets expectations'
        ELSE NULL
    END,
    CASE
        WHEN i % 3 = 0 THEN 'Really happy with this purchase. Would recommend to anyone looking for quality.'
        WHEN i % 3 = 1 THEN 'Arrived quickly and works as described. No complaints so far.'
        ELSE NULL
    END,
    i % 2 = 0,   -- 50% verified purchases
    now() - ((500 - i) || ' days')::interval
FROM generate_series(1, 500) AS i;

-- ─────────────────────────────────────────────────────────────────────────────
-- Data: analytics.inventory_events (3000 rows)
-- event_type distribution: mostly 'sale', occasional restock/return/adjustment
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO analytics.inventory_events (
    product_id, event_type, quantity_change, created_at
)
SELECT
    1 + (i % 100),
    CASE
        WHEN i % 10 IN (0, 1, 2, 3, 4) THEN 'sale'
        WHEN i % 10 IN (5, 6)          THEN 'restock'
        WHEN i % 10 = 7                THEN 'return'
        ELSE                                'adjustment'
    END,
    CASE
        WHEN i % 10 IN (0, 1, 2, 3, 4) THEN -(1 + i % 5)   -- sales: negative
        WHEN i % 10 IN (5, 6)          THEN  (10 + i % 90)  -- restock: positive
        WHEN i % 10 = 7                THEN  (1 + i % 3)    -- return: positive
        ELSE                                 (-(i % 5) + 2) -- adjustment: small +/-
    END,
    now() - ((3000 - i) * 720 / 3000 || ' minutes')::interval
FROM generate_series(1, 3000) AS i;

-- ─────────────────────────────────────────────────────────────────────────────
-- Data: audit.change_log (1000 rows)
-- ─────────────────────────────────────────────────────────────────────────────

INSERT INTO audit.change_log (
    table_name, operation, row_id, changed_by, changed_at,
    old_values, new_values
)
SELECT
    (ARRAY['customers','orders','products','order_items','reviews'])[1 + (i % 5)],
    (ARRAY['INSERT','UPDATE','UPDATE','DELETE','UPDATE'])[1 + (i % 5)],
    1 + (i % 500),
    CASE WHEN i % 3 = 0 THEN 'pgmcp' WHEN i % 3 = 1 THEN 'app_user' ELSE 'admin' END,
    now() - ((1000 - i) || ' minutes')::interval,
    CASE
        WHEN i % 5 = 3 THEN jsonb_build_object('status', 'confirmed', 'updated_at', now() - interval '2 days')
        WHEN i % 5 = 1 THEN jsonb_build_object('is_active', true)
        ELSE NULL
    END,
    CASE
        WHEN i % 5 = 3 THEN jsonb_build_object('status', 'shipped', 'updated_at', now() - interval '1 day')
        WHEN i % 5 = 1 THEN jsonb_build_object('is_active', false)
        ELSE jsonb_build_object('id', 1 + (i % 500), 'action', 'insert')
    END
FROM generate_series(1, 1000) AS i;

-- ─────────────────────────────────────────────────────────────────────────────
-- Post-insert maintenance
-- ─────────────────────────────────────────────────────────────────────────────

-- Update customer order counts to reflect actual order data
UPDATE customers c
SET order_count = (
    SELECT count(*) FROM orders o WHERE o.customer_id = c.id
);

-- Populate the materialized view with the seeded data
REFRESH MATERIALIZED VIEW analytics.daily_revenue;

-- Ensure the query planner has fresh statistics for all seeded tables
ANALYZE;

-- ═══════════════════════════════════════════════════════════════════════════════
-- DEMO QUERIES — use these with pgmcp tools
-- ═══════════════════════════════════════════════════════════════════════════════
--
-- list_schemas: (no params needed)
-- list_tables: {"schema": "public", "kind": "all"}
-- list_enums: {"schema": "public"}
-- describe_table: {"schema": "public", "table": "customers"}
-- describe_table: {"schema": "public", "table": "orders"}
-- list_extensions: (no params needed)
-- table_stats: {"schema": "public", "table": "orders"}
--
-- query (aggregation):
--   {"sql": "SELECT category, count(*) AS cnt, avg(price_cents)/100.0 AS avg_price FROM products GROUP BY category ORDER BY avg_price DESC"}
--
-- query (join):
--   {"sql": "SELECT c.username, count(o.id) AS orders, sum(o.total_cents)/100.0 AS total_spent FROM customers c JOIN orders o ON o.customer_id = c.id GROUP BY c.id, c.username ORDER BY total_spent DESC LIMIT 10"}
--
-- explain (will show seq scan on orders -- no index on status):
--   {"sql": "SELECT * FROM orders WHERE status = 'pending' AND created_at > now() - interval '30 days'"}
--
-- suggest_index (should recommend index on orders.status):
--   {"sql": "SELECT * FROM orders WHERE status = 'pending'"}
--
-- suggest_index (should recommend index on orders.customer_id):
--   {"sql": "SELECT * FROM orders WHERE customer_id = 42"}
--
-- propose_migration:
--   {"sql": "ALTER TABLE customers ADD COLUMN loyalty_tier TEXT DEFAULT 'bronze'"}
--
-- my_permissions: {"schema": "public"}
-- my_permissions (as demo_reader -- connect with demo_reader role): {"schema": "audit"}
--
-- connection_info: (no params needed)
-- health: (no params needed)
-- server_info: (no params needed)
-- list_databases: (no params needed)
