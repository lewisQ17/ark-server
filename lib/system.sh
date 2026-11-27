#@lib shellcheck shell=bash
#@lib Systemd-units en firewallpoorten — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  SYSTEMD & FIREWALL
#═══════════════════════════════════════════════════════════════════════════════

create_systemd_service() {
    local map="$1"
    local service_name="ark-${map,,}.service"
    local service_file="/etc/systemd/system/$service_name"
    log_info "Creating systemd service: ${BLD}$service_name${R}"
    "${SUDO[@]}" tee "$service_file" > /dev/null <<SVCEOF
[Unit]
Description=ARK ASA Server - $map
After=network-online.target
Wants=network-online.target

[Service]
Type=forking
User=${ARK_SERVICE_USER}
WorkingDirectory=$SCRIPT_DIR
Environment=PROTON_LOG=1
Environment=PROTON_LOG_DIR=$(getent passwd "$ARK_SERVICE_USER" 2>/dev/null | cut -d: -f6 || echo "$HOME")
ExecStart=$SCRIPT_DIR/ark-manager.sh start $map
ExecStop=$SCRIPT_DIR/ark-manager.sh stop $map
Restart=on-failure
RestartSec=30
TimeoutStartSec=300
TimeoutStopSec=180
MemoryAccounting=true
CPUAccounting=true

[Install]
WantedBy=multi-user.target
SVCEOF
    "${SUDO[@]}" systemctl daemon-reload
    "${SUDO[@]}" systemctl enable "$service_name" 2>/dev/null || true
    log_ok "Service '${BLD}$service_name${R}${GRN}' created."
}

open_firewall_ports() {
    local game_port="$1" query_port="$2" rcon_port="$3"
    if command -v ufw &>/dev/null; then
        "${SUDO[@]}" ufw allow "${game_port}/udp" 2>/dev/null || true
        "${SUDO[@]}" ufw allow "$(( game_port + 1 ))/udp" 2>/dev/null || true
        "${SUDO[@]}" ufw allow "${query_port}/udp" 2>/dev/null || true
        # De RCON-poort werd hier voor de hele wereld opengezet ('ufw allow <port>/tcp'),
        # terwijl README en installer localhost-only beloven. Loopback heeft geen
        # ufw-regel nodig; alleen het eigen private LAN-subnet krijgt toegang, en kan dat
        # niet bepaald worden dan blijft RCON puur localhost. Een eerder aangemaakte
        # wereldwijde regel wordt hier ook opgeruimd.
        "${SUDO[@]}" ufw delete allow "${rcon_port}/tcp" 2>/dev/null || true
        local lan_cidr=""
        lan_cidr=$(ip -o -f inet route show scope link 2>/dev/null | \
            awk '$1 ~ /^(10\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.)/ {print $1; exit}')
        if [[ -n "$lan_cidr" ]]; then
            "${SUDO[@]}" ufw allow from "$lan_cidr" to any port "${rcon_port}" proto tcp 2>/dev/null || true
            log_info "RCON ${BLD}${rcon_port}/tcp${R} allowed from ${BLD}${lan_cidr}${R} only, never from the internet."
        else
            log_info "RCON ${BLD}${rcon_port}/tcp${R} left closed in the firewall — localhost only."
        fi
    fi
}

