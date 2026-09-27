#!/bin/sh
# Installs developer access to the TPF3-MP server on this host
# (docs/OPERATIONS.md, "Developer access"): the group tpf3mp-devs, the
# controller its members may run, the sudo rule for it and the SSH rules
# that let them run nothing else. Run as root from this folder, again after
# any of these files change. tpf3mp-add-dev then adds each developer.
#
# Each piece is checked before it takes effect, and put back as it was
# when a check fails: a broken sudo rule would stop sudo for everyone, and
# broken SSH rules could keep root out.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
sshd_rules=/etc/ssh/sshd_config.d/50-tpf3mp-devs.conf

die() {
  echo "install.sh: $*" >&2
  exit 1
}

[ "$(id -u)" -eq 0 ] || die "run it as root"

getent group tpf3mp-devs >/dev/null || groupadd --system tpf3mp-devs
for program in tpf3mp-ctl tpf3mp-dev-shell tpf3mp-add-dev; do
  install -o root -g root -m 0755 "$here/$program" "/usr/local/sbin/$program"
done

# sudo: checked where nothing reads it yet.
staged=$(mktemp)
install -o root -g root -m 0440 "$here/sudoers.tpf3mp-devs" "$staged"
visudo -cf "$staged" >/dev/null || { rm -f "$staged"; die "visudo refuses the sudo rule"; }
mv "$staged" /etc/sudoers.d/tpf3mp-devs

# sshd: root's effective settings must stay exactly as they were.
effective() {
  sshd -T -C user=root,host=check,addr=127.0.0.1
}
before=$(effective)
previous=""
if [ -e "$sshd_rules" ]; then
  previous=$(mktemp)
  cp -p "$sshd_rules" "$previous"
fi
put_back() {
  if [ -n "$previous" ]; then mv "$previous" "$sshd_rules"; else rm -f "$sshd_rules"; fi
}
install -o root -g root -m 0644 "$here/sshd.tpf3mp-devs.conf" "$sshd_rules"
if ! sshd -t; then
  put_back
  die "sshd refuses the rules: nothing changed"
fi
if [ "$(effective)" != "$before" ]; then
  put_back
  die "root's SSH settings would change: nothing changed"
fi
[ -z "$previous" ] || rm -f "$previous"
systemctl reload ssh
echo "installed: add developers with tpf3mp-add-dev <name> <public key file>"
