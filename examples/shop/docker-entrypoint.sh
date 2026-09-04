#!/bin/sh
# Apply the shop's migrations, then start the server.
#
# `prisma migrate deploy` and not `db push`: `deploy` applies the committed
# migration files and refuses to invent a schema, so the container's idea of
# the database is the one in `prisma/migrations/` and nothing else. It is
# idempotent — a database already at the latest migration is a no-op — which
# is what makes a restart safe and what seeds the catalogue exactly once (the
# catalogue is the second migration; see the note in its `migration.sql`).
#
# If it fails, the container fails. A shop that started with no `products`
# table would serve an empty catalogue and blame nothing.
set -eu

echo "vpay-shop: applying migrations"
prisma migrate deploy --schema /app/prisma/schema.prisma

echo "vpay-shop: starting Next.js on :3000"
exec node /app/examples/shop/server.js
