#!/bin/sh
# Apply the shop's migrations, then start the server.
#
# `zen migrate deploy` and not `zen db push`: `deploy` applies the committed
# migration files and refuses to invent a schema, so the container's idea of
# the database is the one in `zenstack/migrations/` and nothing else. It is
# idempotent — a database already at the latest migration is a no-op — which
# is what makes a restart safe and what seeds the catalogue exactly once (the
# catalogue is the second migration; see the note in its `migration.sql`).
#
# It was `prisma migrate deploy --schema /app/prisma/schema.prisma` until the
# ZenStack 3 upgrade (2026-09-06). Prisma Migrate is still what applies the
# SQL — v3's migration engine drives it — but the schema it works from is
# derived from the zmodel on the fly, into a temporary `~schema.prisma` beside
# it, so there is no committed Prisma schema for this line to name any more.
# That temporary file is why the schema is COPIED to /tmp before the CLI is
# pointed at it. The compose service runs this image with `read_only: true`
# and a tmpfs on /tmp, so `/app/zenstack` cannot be written and a CLI that
# tried would fail with EROFS after the container had already reported
# healthy-ish. The migrations directory travels with the copy, which is how
# `zen migrate deploy` still finds all three of them.
#
# If it fails, the container fails. A shop that started with no `products`
# table would serve an empty catalogue and blame nothing.
set -eu

echo "vpay-shop: applying migrations"
rm -rf /tmp/zenstack
cp -R /app/zenstack /tmp/zenstack
zen migrate deploy --schema /tmp/zenstack/schema.zmodel --no-version-check

echo "vpay-shop: starting Next.js on :3000"
exec node /app/examples/shop/server.js
