#!/bin/sh
# PostgreSQL container name and a NEW destination; never overwrite a backup.
set -eu
container=${1:?Usage: backup.sh CONTAINER NEW_DIRECTORY DATABASE USER}
destination=${2:?New destination required}
database=${3:?Database required}
role=${4:?Database role required}
umask 077
mkdir "$destination"
trap 'rm -f "$destination/database.dump.partial"' EXIT HUP INT TERM
docker exec "$container" pg_dump -U "$role" -d "$database" --format=custom --no-owner > "$destination/database.dump.partial"
mv "$destination/database.dump.partial" "$destination/database.dump"
(cd "$destination" && shasum -a 256 database.dump > SHA256SUMS)
date -u +%Y-%m-%dT%H:%M:%SZ > "$destination/completed-at"
# Upload/encrypt off-host using the deployment's managed backup service.
