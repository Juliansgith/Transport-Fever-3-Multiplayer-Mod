#!/bin/sh
# Hands a renewed certificate to the TPF3-MP server. Certbot runs every
# script in /etc/letsencrypt/renewal-hooks/deploy/ after any renewal, with
# RENEWED_LINEAGE naming the renewed certificate's folder; this one acts on
# TPF3-MP's alone. Install it there with HOST and DEPLOY set below, and run
# it once by hand for the first copy:
#
#   RENEWED_LINEAGE=/etc/letsencrypt/live/<host> /etc/letsencrypt/renewal-hooks/deploy/tpf3mp.sh
#
# It copies the certificate where the server reads it, readable by the
# image's user (UID 65532) alone, since Certbot's own files are root's, and
# restarts the server, which reads its certificate at start. Rooms survive
# the restart (docs/OPERATIONS.md, "Upgrades"). See docs/OPERATIONS.md,
# "Certificates".
set -eu

HOST=tpf3mp.example.org
DEPLOY=/opt/tpf3mp/deploy

case "${RENEWED_LINEAGE:-}" in
  */live/"$HOST") ;;
  *) exit 0 ;;
esac

mkdir -p "$DEPLOY/certs"
for file in fullchain.pem privkey.pem; do
  install -o 65532 -g 65532 -m 0440 "$RENEWED_LINEAGE/$file" "$DEPLOY/certs/$file"
done

# A server not started yet reads the copy when it starts.
if [ -n "$(docker compose -f "$DEPLOY/compose.yaml" ps -q 2>/dev/null)" ]; then
  docker compose -f "$DEPLOY/compose.yaml" restart
fi
