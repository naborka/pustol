#!/usr/bin/env bash
# Local PostgreSQL for the integration tests.
#
# The invariant this system rests on — that a table cannot be sold twice — is enforced by a
# PostgreSQL exclusion constraint, so testing it against anything other than a real PostgreSQL
# would test nothing. The schema targets PostgreSQL 17, and so does this.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PG="$ROOT/.tools/pg"
PGDATA="$PG/data"
PGPORT="${PUSTOL_PGPORT:-55432}"
PGHOST=127.0.0.1

PG_VERSION=17.10.0
# The theseus-rs musl build links against ICU by soname, and the one it wants left Alpine after
# 3.20. The runtime libraries are otherwise taken from whatever release this machine already runs.
ICU_REPOSITORY=https://dl-cdn.alpinelinux.org/alpine/v3.20/main

# A binary built for one C library cannot run under the other, so which download is correct is a
# fact about this machine rather than a preference.
if [[ -e /lib/ld-musl-x86_64.so.1 ]]; then
  LIBC=musl
else
  LIBC=gnu
fi

if [[ $LIBC == musl ]]; then
  # One tarball carries both the server and psql, and the libraries it does not carry come from
  # Alpine's own packages, unpacked beside it rather than installed.
  SERVER_BIN="$PG/dist/bin"
  CLIENT_BIN="$PG/dist/bin"
  DEPLIB="$PG/deplib"
  export LD_LIBRARY_PATH="$DEPLIB/usr/lib:$PG/dist/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  ICU_DATA_DIR="$(ls -d "$DEPLIB"/usr/share/icu/*/ 2>/dev/null | head -1 || true)"
  [[ -n $ICU_DATA_DIR ]] && export ICU_DATA="$ICU_DATA_DIR"
else
  SERVER_BIN="$PG/dist/bin"
  CLIENT_BIN="$PG/client"
  export LD_LIBRARY_PATH="$PG/clientlib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

DATABASE_URL="postgres://postgres@$PGHOST:$PGPORT/pustol"

psql_() { "$CLIENT_BIN/psql" -h "$PGHOST" -p "$PGPORT" -U postgres -v ON_ERROR_STOP=1 "$@"; }

download() {
  if command -v curl >/dev/null; then
    curl -sSL -o "$2" "$1"
  elif command -v wget >/dev/null; then
    wget -q -O "$2" "$1"
  else
    echo "neither curl nor wget is available" >&2
    exit 1
  fi
}

require_binaries() {
  if [[ ! -x "$SERVER_BIN/postgres" || ! -x "$CLIENT_BIN/psql" ]]; then
    echo "PostgreSQL binaries are missing. Run: scripts/pg.sh install" >&2
    exit 1
  fi
}

# PostgreSQL reads the zone database from a path fixed when it was compiled, so the copy has to
# land there and nowhere else. A minimal Alpine has no zone database at all, and a server without
# one refuses every timezone including UTC — which for a bar whose Friday runs past midnight would
# be the end of the matter. The container's root filesystem is rebuilt often, so this is checked on
# every start rather than once at install.
require_system_tzdata() {
  [[ $LIBC == musl ]] || return 0
  [[ -e /usr/share/zoneinfo/UTC ]] && return 0
  if [[ ! -d $DEPLIB/usr/share/zoneinfo ]]; then
    echo "the zone database is missing. Run: scripts/pg.sh install" >&2
    exit 1
  fi
  if ! mkdir -p /usr/share/zoneinfo 2>/dev/null || ! cp -r "$DEPLIB/usr/share/zoneinfo/." /usr/share/zoneinfo/ 2>/dev/null; then
    echo "cannot write /usr/share/zoneinfo, which PostgreSQL reads by absolute path" >&2
    exit 1
  fi
}

install_musl() {
  command -v apk >/dev/null || {
    echo "this is a musl system without apk, so the runtime libraries cannot be fetched" >&2
    exit 1
  }
  mkdir -p "$PG"
  cd "$PG"
  echo "downloading server and client binaries..."
  download \
    "https://github.com/theseus-rs/postgresql-binaries/releases/download/$PG_VERSION/postgresql-$PG_VERSION-x86_64-unknown-linux-musl.tar.gz" \
    pg.tar.gz
  rm -rf dist && mkdir dist && tar -xzf pg.tar.gz -C dist --strip-components=1
  rm -f pg.tar.gz

  echo "downloading runtime libraries..."
  rm -rf deps deplib && mkdir -p deps deplib
  # `apk fetch` needs no root: it downloads and resolves, and the unpacking here is a plain
  # extraction into a directory of ours, not an installation into the system.
  apk fetch --no-cache -q -R -o deps lz4-libs libxml2 krb5-libs tzdata
  echo "$ICU_REPOSITORY" > deps/repositories
  apk fetch --no-cache -q --repositories-file deps/repositories -o deps icu-libs icu-data-full
  # An .apk is a tarball with a signature member busybox tar reports and skips; the check that
  # matters is whether the server runs afterwards.
  for package in deps/*.apk; do tar -xzf "$package" -C deplib 2>/dev/null || true; done
  rm -rf deps
}

install_gnu() {
  mkdir -p "$PG"
  cd "$PG"
  # Server binaries from Zonky's embedded build (self-contained), client binaries from the
  # theseus-rs build (which ships psql). Neither needs root, a package manager or a container.
  echo "downloading server binaries..."
  download \
    "https://repo1.maven.org/maven2/io/zonky/test/postgres/embedded-postgres-binaries-linux-amd64/$PG_VERSION/embedded-postgres-binaries-linux-amd64-$PG_VERSION.jar" \
    pg.jar
  unzip -p pg.jar postgres-linux-x86_64.txz > postgres-linux-x86_64.txz
  rm -rf dist && mkdir dist && tar -xJf postgres-linux-x86_64.txz -C dist
  rm -f pg.jar postgres-linux-x86_64.txz
  echo "downloading client binaries..."
  download \
    "https://github.com/theseus-rs/postgresql-binaries/releases/download/$PG_VERSION/postgresql-$PG_VERSION-x86_64-unknown-linux-gnu.tar.gz" \
    client.tar.gz
  rm -rf client-tmp && mkdir client-tmp && tar -xzf client.tar.gz -C client-tmp --strip-components=1
  rm -rf client clientlib && mkdir client
  cp client-tmp/bin/psql client-tmp/bin/pg_isready client/
  cp -r client-tmp/lib clientlib
  rm -rf client-tmp client.tar.gz
}

case "${1:-help}" in
install)
  if [[ $LIBC == musl ]]; then install_musl; else install_gnu; fi
  # A cluster is bound to the binaries that made it, and these are new ones.
  rm -rf "$PGDATA"
  require_system_tzdata
  echo "installed $("$SERVER_BIN/postgres" --version)"
  ;;

init)
  require_binaries
  require_system_tzdata
  rm -rf "$PGDATA"
  mkdir -p "$PGDATA" "$PG/run"
  "$SERVER_BIN/initdb" -D "$PGDATA" -U postgres -A trust -E UTF8 --locale=C >/dev/null
  echo "initialised $PGDATA"
  ;;

start)
  require_binaries
  require_system_tzdata
  [[ -f "$PGDATA/PG_VERSION" ]] || "$0" init
  # Durability is off on purpose: this cluster exists to be thrown away between test runs.
  "$SERVER_BIN/pg_ctl" -D "$PGDATA" -l "$PG/pg.log" -w -o \
    "-p $PGPORT -k $PG/run -c listen_addresses=$PGHOST -c fsync=off -c full_page_writes=off -c synchronous_commit=off -c timezone=UTC -c max_connections=200" \
    start
  psql_ -d postgres -tc "select 1 from pg_database where datname = 'pustol'" | grep -q 1 ||
    psql_ -d postgres -c "create database pustol" >/dev/null
  # Each integration test creates a database of its own; a run killed part-way leaves them behind.
  psql_ -d postgres -tAc \
    "select 'drop database \"' || datname || '\"' from pg_database where datname like 'pustol\\_t%'" |
    while read -r statement; do [[ -n "$statement" ]] && psql_ -d postgres -c "$statement" >/dev/null; done
  echo "$DATABASE_URL"
  ;;

stop)
  require_binaries
  "$SERVER_BIN/pg_ctl" -D "$PGDATA" -m fast stop || true
  ;;

status)
  require_binaries
  "$SERVER_BIN/pg_ctl" -D "$PGDATA" status || true
  ;;

reset)
  "$0" stop
  "$0" init
  "$0" start
  ;;

psql)
  require_binaries
  shift
  psql_ -d pustol "$@"
  ;;

url)
  echo "$DATABASE_URL"
  ;;

*)
  cat <<'USAGE'
scripts/pg.sh <command>

  install   download PostgreSQL 17.10 into .tools/pg (no root, no container)
  init      create a fresh data directory
  start     start the server and ensure the pustol database exists
  stop      stop the server
  status    report whether the server is running
  reset     stop, wipe and start again
  psql      open a shell on the pustol database (extra arguments are passed through)
  url       print the connection string
USAGE
  ;;
esac
