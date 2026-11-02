-- Run once as database owner, with secrets supplied through psql environment.
\set ON_ERROR_STOP on
\getenv migration_password SENTINEL_MIGRATION_PASSWORD
\getenv monitor_password SENTINEL_MONITOR_PASSWORD
\getenv delivery_password SENTINEL_DELIVERY_PASSWORD
\getenv reader_password SENTINEL_READER_PASSWORD
\getenv operator_password SENTINEL_OPERATOR_PASSWORD
CREATE ROLE sentinel_migrate LOGIN PASSWORD :'migration_password';
CREATE ROLE sentinel_monitor LOGIN PASSWORD :'monitor_password';
CREATE ROLE sentinel_delivery LOGIN PASSWORD :'delivery_password';
CREATE ROLE sentinel_reader LOGIN PASSWORD :'reader_password';
CREATE ROLE sentinel_operator LOGIN PASSWORD :'operator_password';
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE, CREATE ON SCHEMA public TO sentinel_migrate;
GRANT USAGE ON SCHEMA public TO sentinel_monitor,sentinel_delivery,sentinel_reader,sentinel_operator;
