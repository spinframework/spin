# Setting up Postgres with Certs

## Prerequisite

PostgreSQL installed and on your PATH.  If it isn't, the setup script will display instructions for `brew install`-ing it.

## Create the DB and seed it with 3 rows

```sh
./local-db.sh
```

Test base case without TLS:

```sh
spin build --up
```

Test
```console
$ curl localhost:3000
Done
```

## Enable TLS in Postgres

1. Create SSL Certificates and Configure Postgres

To create certs and configure PG to use them, run `make-certs.sh` to generate self-signed SSL certificates for Postgres. This script will create a directory `postgres-ssl` containing the necessary certificate files.


2. Update the `pg_hba.conf` file to require SSL connections.

```sh
    # Update pg_hba.conf to require TLS
    cat >> "pg/data/pg_hba.conf" << EOF
hostssl all             all             127.0.0.1/32            trust
hostssl all             all             ::1/128                 trust
EOF
```

Then reload:
```sh
pg_ctl reload -D  pg/data 
```

Restart spin to create a new PG connection that will now require TLS. Now calls will fail without TLS:

```console
$ curl localhost:3000       
Error::ConnectionFailed("Error occurred while creating a new object: error performing TLS handshake: The certificate was not trusted.\n\nCaused by:\n    0: error performing TLS handshake: The certificate was not trusted.\n    1: The certificate was not trusted.")%
```

Then run `spin up --runtime-config-file runtime-config.toml` to confirm that it works when the custom root certs are loaded.
