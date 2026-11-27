#@lib shellcheck shell=bash
#@lib RCON-console, send_rcon en broadcast — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  RCON
#═══════════════════════════════════════════════════════════════════════════════

rcon_console() {
    local map="$1"
    if ! is_map_running "$map"; then
        log_err "Map '${BLD}$map${R}' is not running."; return 1
    fi
    local rcon_port admin_pw
    rcon_port=$(read_conf_value "$MAPS_DIR/$map/map.conf" "RCONPort" "27020")
    admin_pw=$(read_conf_value "$MAPS_DIR/$map/map.conf" "AdminPassword" "")
    clear
    log_info "RCON connected to ${BLD}$map${R}:${CYN}$rcon_port${R}"
    echo "  ${DIM}Type 'exit' or Ctrl+C to disconnect${R}"
    separator
    python3 "$RCON_SCRIPT" "localhost:$rcon_port" -p "$admin_pw"
}

send_rcon() {
    local map="$1" cmd="$2"
    if ! is_map_running "$map"; then log_err "Map '$map' is not running."; return 1; fi
    local rcon_port admin_pw
    rcon_port=$(read_conf_value "$MAPS_DIR/$map/map.conf" "RCONPort" "27020")
    admin_pw=$(read_conf_value "$MAPS_DIR/$map/map.conf" "AdminPassword" "")
    python3 "$RCON_SCRIPT" "localhost:$rcon_port" -p "$admin_pw" -c "$cmd"
}

broadcast_message() {
    local message="$1"
    local maps; maps=( $(get_maps) )
    for map in "${maps[@]}"; do
        is_map_running "$map" && send_rcon "$map" "ServerChat $message" 2>/dev/null || true
    done
    log_ok "Broadcast sent to all running maps."
}

