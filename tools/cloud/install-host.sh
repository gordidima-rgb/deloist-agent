#!/usr/bin/env bash
# Один раз на сервере: Docker + gVisor (runsc) и сетевая клетка для контейнеров пользователей.
# Ничего не трогает в сайте, базе и nginx. Перед изменениями — резервная копия конфигов.
# Запуск: sudo bash install-host.sh
set -euo pipefail

STAMP=$(date +%Y%m%d-%H%M%S)
BACKUP=/root/deloist-cloud-backup-$STAMP.tgz
tar czf "$BACKUP" /etc/nginx /etc/ufw /etc/default/ufw 2>/dev/null || true
echo "резервная копия: $BACKUP"

# 1. Docker из пакетов Ubuntu.
apt-get update -q
apt-get install -y -q docker.io

# 2. gVisor из официального репозитория gvisor.dev (ключ и репозиторий — по их инструкции).
install -d -m 0755 /usr/share/keyrings
curl -fsSL https://gvisor.dev/archive.key | gpg --dearmor --yes -o /usr/share/keyrings/gvisor-archive-keyring.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/gvisor-archive-keyring.gpg] https://storage.googleapis.com/gvisor/releases release main" \
  > /etc/apt/sources.list.d/gvisor.list
apt-get update -q
apt-get install -y -q runsc
runsc install            # прописывает runtime "runsc" в /etc/docker/daemon.json
systemctl restart docker
docker info --format '{{json .Runtimes}}' | grep -q runsc && echo "runsc зарегистрирован"

# 3. Сетевая клетка. Сети пользователей — только из 172.30.0.0/16.
install -d -m 0755 /opt/deloist-cloud
install -m 0755 "$(dirname "$0")/cage.sh" /opt/deloist-cloud/cage.sh
cat > /etc/systemd/system/deloist-cage.service <<'UNIT'
[Unit]
Description=Deloist cloud: network cage for user containers
After=docker.service ufw.service
Requires=docker.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/opt/deloist-cloud/cage.sh

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable --now deloist-cage.service
echo "готово"
