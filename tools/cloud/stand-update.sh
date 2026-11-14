#!/usr/bin/env bash
# Таймер стенда: если в GHCR новый образ — пересоздать контейнер стенда. Данные и ключ не меняются.
# Параметры стенда — в /etc/deloist-cloud/stand.conf (ID, NET, PORT, ORIGIN, GWPUB, TAG).
set -euo pipefail
. /etc/deloist-cloud/stand.conf
IMAGE=ghcr.io/gordidima-rgb/deloist-server:$TAG
NAME=deloist-u-$ID
before=$(docker image inspect -f '{{.Id}}' "$IMAGE" 2>/dev/null || echo none)
docker pull -q "$IMAGE" >/dev/null
after=$(docker image inspect -f '{{.Id}}' "$IMAGE")
running=$(docker inspect -f '{{.Image}}' "$NAME" 2>/dev/null || echo none)
if [ "$after" != "$running" ]; then
  echo "$(date -Is) новый образ $after (был $before, в контейнере $running): пересоздаю"
  bash /opt/deloist-cloud/user-up.sh "$ID" "$NET" "$PORT" "$ORIGIN" "$GWPUB" "$TAG"
  docker image prune -f >/dev/null
fi
