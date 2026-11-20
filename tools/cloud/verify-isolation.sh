#!/usr/bin/env bash
# Operational verification of container isolation on disposable Linux.
# Usage: sudo bash verify-isolation.sh <container-name>
set -euo pipefail
NAME=${1:-deloist}
echo "== inspect =="
docker inspect "$NAME" --format 'user={{.Config.User}} caps={{.HostConfig.CapDrop}} security={{.HostConfig.SecurityOpt}} readonly={{.HostConfig.ReadonlyRootfs}}'
# required flags
docker inspect "$NAME" --format '{{json .HostConfig}}' | grep -q '"CapDrop":\["ALL"\]' || { echo "FAIL: cap-drop ALL missing"; exit 1; }
docker inspect "$NAME" --format '{{json .HostConfig}}' | grep -q 'no-new-privileges' || { echo "FAIL: no-new-privileges missing"; exit 1; }
# no docker socket
if docker inspect "$NAME" --format '{{json .Mounts}}' | grep -q '/var/run/docker.sock'; then echo "FAIL: docker socket mounted"; exit 1; fi
# data dir
docker exec "$NAME" sh -c 'test "$(stat -c %a /data)" = "700" || { echo "bad /data mode"; exit 1; }'
# process user not root
docker exec "$NAME" sh -c 'test "$(id -u)" != "0" || { echo "running as root"; exit 1; }'
echo "OK: isolation constraints present"