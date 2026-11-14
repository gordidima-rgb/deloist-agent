#!/usr/bin/env bash
# Сетевая клетка для контейнеров пользователей (сети 172.30.0.0/16). Повторный запуск безопасен.
# - В интернет можно.
# - Во внутренние и служебные сети нельзя: частные адреса, VPN сервера (10.8.0.0/24 внутри 10/8),
#   метаданные облака 169.254.169.254, CGNAT, multicast.
# - К самому серверу нельзя (ни к одному порту): цепочка INPUT.
# - Между контейнерами разных пользователей нельзя: каждый в своей сети, Docker это уже режет;
#   здесь — второй замок.
set -euo pipefail
CAGE=172.30.0.0/16
BLOCKED="10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 169.254.0.0/16 100.64.0.0/10 224.0.0.0/4 0.0.0.0/8"

iptables -N DELOIST-CAGE 2>/dev/null || iptables -F DELOIST-CAGE
iptables -A DELOIST-CAGE -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN
for net in $BLOCKED; do
  iptables -A DELOIST-CAGE -s "$CAGE" -d "$net" -j DROP
done
iptables -A DELOIST-CAGE -j RETURN

# FORWARD: трафик контейнеров наружу. DOCKER-USER Docker создаёт сам и не трогает.
iptables -C DOCKER-USER -s "$CAGE" -j DELOIST-CAGE 2>/dev/null || iptables -I DOCKER-USER 1 -s "$CAGE" -j DELOIST-CAGE

# INPUT: контейнер -> сам сервер (любой его адрес). Ответы на проброшенный 127.0.0.1-порт идут как ESTABLISHED.
iptables -N DELOIST-CAGE-IN 2>/dev/null || iptables -F DELOIST-CAGE-IN
iptables -A DELOIST-CAGE-IN -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN
# DNS встроенного резолвера Docker живёт в сетевом пространстве контейнера, сюда не приходит.
iptables -A DELOIST-CAGE-IN -s "$CAGE" -j DROP
iptables -C INPUT -s "$CAGE" -j DELOIST-CAGE-IN 2>/dev/null || iptables -I INPUT 1 -s "$CAGE" -j DELOIST-CAGE-IN
echo "клетка на месте для $CAGE"
