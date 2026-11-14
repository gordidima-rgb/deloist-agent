#!/usr/bin/env bash
# Создать или пересоздать контейнер пользователя на свежем образе. Повторный запуск безопасен:
# данные лежат в томе deloist-u-<id>, ключ шифрования — в /etc/deloist-cloud/<id>.env и не меняется.
# Запуск: sudo bash user-up.sh <id> <номер сети 1..254> <порт на 127.0.0.1> <origin> <открытый ключ шлюза> [тег образа]
set -euo pipefail
ID=$1; NET=$2; PORT=$3; ORIGIN=$4; GWPUB=$5; TAG=${6:-main}
[[ $ID =~ ^[a-z0-9_-]{1,32}$ ]] || { echo "плохой id"; exit 1; }
[[ $NET =~ ^[0-9]+$ ]] && (( NET >= 1 && NET <= 254 )) || { echo "плохой номер сети"; exit 1; }
[[ $GWPUB =~ ^[0-9a-f]{64}$ ]] || { echo "открытый ключ шлюза: 64 hex"; exit 1; }
IMAGE=ghcr.io/gordidima-rgb/deloist-server:$TAG
NAME=deloist-u-$ID
ENVF=/etc/deloist-cloud/$ID.env

install -d -m 0700 /etc/deloist-cloud
if [ ! -f "$ENVF" ]; then
  umask 077
  {
    echo "CREW_CLOUD=1"
    echo "CREW_USER_ID=$ID"
    echo "CREW_PUBLIC_ORIGIN=$ORIGIN"
    echo "CREW_GATEWAY_PUBKEY=$GWPUB"
    echo "CREW_SECRETS_KEY=$(openssl rand -hex 32)"
    echo "HOST=0.0.0.0"
    echo "PORT=8899"
  } > "$ENVF"
  echo "создан $ENVF (ключ шифрования пользователя сгенерирован здесь и никуда не уходит)"
fi

docker network inspect "$NAME" >/dev/null 2>&1 || docker network create --subnet "172.30.$NET.0/24" "$NAME" >/dev/null
# Встроенный резолвер Docker (127.0.0.11) под gVisor не отвечает: свой resolv.conf с внешними DNS.
RESOLV=/opt/deloist-cloud/resolv.conf
[ -f "$RESOLV" ] || printf 'nameserver 77.88.8.8\nnameserver 1.1.1.1\noptions edns0 ndots:0\n' > "$RESOLV"
docker pull -q "$IMAGE"
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" \
  --runtime=runsc \
  --restart unless-stopped \
  --network "$NAME" \
  --mount "type=bind,src=$RESOLV,dst=/etc/resolv.conf,readonly" \
  -p "127.0.0.1:$PORT:8899" \
  --memory 1g --memory-swap 1g --cpus 1 --pids-limit 256 \
  --cap-drop ALL --security-opt no-new-privileges \
  --read-only --tmpfs /tmp:size=256m --tmpfs /home/deloist:size=256m,uid=10001,gid=10001 \
  -v "$NAME:/data" \
  --env-file "$ENVF" \
  --label deloist.image="$IMAGE" \
  "$IMAGE" >/dev/null
echo "$NAME запущен на 127.0.0.1:$PORT из $IMAGE ($(docker inspect -f '{{.Image}}' "$NAME" | cut -c1-19))"
