#!/bin/sh
# Restoration is allowed only to a NEW, explicitly named *_restore database.
set -eu
container=${1:?Container required}
backup=${2:?Backup directory required}
database=${3:?New *_restore database required}
role=${4:?Database role required}
case "$database" in *_restore) ;; *) echo 'Refusing non-drill database name' >&2; exit 1;; esac
(cd "$backup" && shasum -a 256 -c SHA256SUMS)
docker exec "$container" createdb -U "$role" "$database"
docker exec -i "$container" pg_restore -U "$role" -d "$database" --exit-on-error --no-owner < "$backup/database.dump"
docker exec "$container" psql -U "$role" -d "$database" -v ON_ERROR_STOP=1 -c 'SELECT count(*) AS incidents FROM incidents; SELECT count(*) AS outbox FROM outbox; SELECT count(*) AS committed_streams FROM chain_streams;'
