#@lib shellcheck shell=bash
#@lib CLI: status, help, plan-install — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  CLI INTERFACE
#═══════════════════════════════════════════════════════════════════════════════

cli_status() {
    local maps; maps=( $(get_maps) )
    (( ${#maps[@]} == 0 )) && { echo "No maps configured."; return; }
    printf "${BLD}%-15s %-10s %-9s %-8s %-7s %-6s %-5s %-4s %-4s${R}\n" "MAP" "STATUS" "HEALTH" "PLAYERS" "UPTIME" "RAM" "CPU" "JOIN" "AUTO"
    echo "──────────────────────────────────────────────────────────────────────────"
    for map in "${maps[@]}"; do
        local status health players uptime mem cpu join auto_s
        is_map_running "$map" && status="${GRN}ONLINE${R}" || status="${RED}OFFLINE${R}"
        health=$(check_map_health "$map")
        case "$health" in
            HEALTHY) health="${GRN}${health}${R}" ;;
            DEGRADED) health="${YEL}${health}${R}" ;;
            CRASHED|ZOMBIE|FROZEN) health="${RED}${health}${R}" ;;
            *) health="${DIM}${health}${R}" ;;
        esac
        if is_map_running "$map"; then
            local pc max_p
            pc=$(get_player_count "$map" 2>/dev/null || echo "?")
            max_p=$(read_conf_value "$MAPS_DIR/$map/map.conf" "MaxPlayers" "70")
            players="${pc}/${max_p}"
            local join_raw
            join_raw=$(check_server_queryable "$map")
            [[ "$join_raw" == "YES" ]] && join="${GRN}✔${R}" || join="${RED}✖${R}"
        else
            players="-"; join="${DIM}-${R}"
        fi
        uptime=$(get_map_uptime "$map"); mem=$(get_map_memory "$map"); cpu=$(get_map_cpu "$map")
        is_autostart_enabled "$map" 2>/dev/null && auto_s="${GRN}YES${R}" || auto_s="${RED}NO${R}"
        echo "$(pad 15 "$map") ${status}    ${health}  $(pad 8 "$players") $(pad 7 "$uptime") $(pad 6 "$mem") $(pad 5 "$cpu") ${join}    ${auto_s}"
    done
}

show_help() {
    echo ""
    echo "  ${BLD}ARK: Survival Ascended — Server Manager${R}  ${DIM}v${VERSION}${R}"
    echo ""
    echo "  ${BLD}Usage:${R} $(basename "$0") ${CYN}[command]${R} ${DIM}[arguments]${R}"
    echo ""
    echo "  ${CYN}(none)${R}              Interactive dashboard"
    echo "  ${CYN}install${R}             Install/update server"
    echo "  ${CYN}plan-install${R}        Preview install/system changes (no writes)"
    echo "  ${CYN}start${R} <map>         Start a map"
    echo "  ${CYN}stop${R} <map>          Stop a map"
    echo "  ${CYN}restart${R} <map>       Restart a map"
    echo "  ${CYN}start-all${R}           Start all groep maps"
    echo "  ${CYN}stop-all${R}            Stop all groep maps"
    echo "  ${CYN}add-map${R} <Name>      Create a map without the menu (for scripts/install.sh)"
    echo "  ${DIM}                        [--internal N] [--game-port N] [--query-port N]${R}"
    echo "  ${DIM}                        [--rcon-port N] [--max-players N] [--admin-pw PW] [--server-pw PW]${R}"
    echo "  ${CYN}status${R}              Show all map statuses"
    echo "  ${CYN}rcon${R} <map> \"cmd\"    Send RCON command"
    echo "  ${CYN}backup${R} <map>        Backup map world"
    echo "  ${CYN}broadcast${R} \"msg\"     Send message to all maps"
    echo "  ${CYN}help${R}                Show this help"
    echo ""
}

plan_install() {
    echo ""
    echo "  ${BLD}ARK ASA install preview (no changes)${R}"
    echo ""
    echo "  ${BLD}Paths:${R}"
    echo "    Script dir       : ${DIM}$SCRIPT_DIR${R}"
    echo "    Server files     : ${DIM}$SERVER_DIR${R}"
    echo "    SteamCMD         : ${DIM}$STEAMCMD_DIR${R}"
    echo "    Proton           : ${DIM}$PROTON_DIR (${PROTON_VERSION})${R}"
    echo ""
    echo "  ${BLD}System integration (if enabled):${R}"
    local enable_tuning
    enable_tuning=$(read_conf_value "$OPT_CONF" "EnableSystemTuning" "true")
    echo "    EnableSystemTuning : ${BLD}$enable_tuning${R}"
    echo "    sysctl profile     : ${DIM}/etc/sysctl.d/99-ark-server.conf${R}"
    echo "    logrotate config   : ${DIM}/etc/logrotate.d/ark-server${R}"
    echo "    healthcheck script : ${DIM}$SCRIPT_DIR/healthcheck.sh${R}"
    echo "    healthcheck cron   : ${DIM}*/5 * * * * healthcheck.sh${R}"
    echo ""
    echo "  ${BLD}Maps discovered:${R}"
    local maps; maps=( $(get_maps) )
    if (( ${#maps[@]} == 0 )); then
        echo "    ${DIM}(none yet)${R}"
    else
        for m in "${maps[@]}"; do
            echo "    - ${BLD}$m${R}"
        done
    fi
    echo ""
    echo "  ${DIM}No files were changed. Use 'install' to actually perform the install/update.${R}"
}

