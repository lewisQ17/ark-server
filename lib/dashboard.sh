#@lib shellcheck shell=bash
#@lib Live dashboard (show_dashboard) — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  DASHBOARD (live data only — prints with CL for flicker-free refresh)
#═══════════════════════════════════════════════════════════════════════════════

show_dashboard() {
    local maps; maps=( $(get_maps) )

    # Header
    echo "${CL}"
    echo "  ${BLD}${BLU}╔══════════════════════════════════════════════════════════════════════════════╗${R}${CL}"
    echo "  ${BLD}${BLU}║${R}  ${BLD}${WHT}  🦖  ARK: Survival Ascended — Server Manager${R}  ${DIM}v${VERSION}${R}                    ${BLD}${BLU}║${R}${CL}"
    echo "  ${BLD}${BLU}╚══════════════════════════════════════════════════════════════════════════════╝${R}${CL}"
    echo "${CL}"

    # System info (fast: /proc/stat instead of top -bn1)
    local cpu_use mem_total mem_used disk_free host_uptime server_ip
    cpu_use=$(get_system_cpu)
    mem_total=$(free -g 2>/dev/null | awk '/Mem:/{print $2}' || echo "?")
    mem_used=$(free -g 2>/dev/null | awk '/Mem:/{print $3}' || echo "?")
    disk_free=$(df -h "$SCRIPT_DIR" 2>/dev/null | awk 'NR==2{print $4}' || echo "?")
    host_uptime=$(uptime -p 2>/dev/null | sed 's/up //' || echo "?")
    server_ip=$(hostname -I 2>/dev/null | awk '{print $1}' || echo "?")

    echo "  ${GRY}┌─ System ─────────────────────────────────────────────────────────────────────┐${R}${CL}"
    printf "  ${GRY}│${R}  IP ${BLD}%s${R}  ${GRY}│${R}  CPU ${BLD}%s%%${R}  ${GRY}│${R}  RAM ${BLD}%sG${R}/%sG  ${GRY}│${R}  Disk ${BLD}%s${R}  ${GRY}│${R}  Up ${BLD}%s${R}${CL}\n" \
        "$server_ip" "$cpu_use" "$mem_used" "$mem_total" "$disk_free" "$host_uptime"
    echo "  ${GRY}└──────────────────────────────────────────────────────────────────────────────┘${R}${CL}"
    echo "${CL}"

    if (( ${#maps[@]} == 0 )); then
        echo "  ${DIM}No maps configured yet. Use ${WHT}Add Map${DIM} to get started.${R}${CL}"
    else
        # Table header — fixed column widths for perfect alignment
        printf "  ${BLD}${WHT}%s %s %s %s %s %s %s %s %s${R}${CL}\n" \
            "$(pad 14 "MAP")" "$(pad 10 "STATUS")" "$(pad 9 "HEALTH")" \
            "$(pad 8 "PLAYERS")" "$(pad 7 "UPTIME")" "$(pad 6 "RAM")" \
            "$(pad 5 "CPU")" "$(pad 4 "JOIN")" "$(pad 4 "AUTO")"
        separator

        for map in "${maps[@]}"; do
            local p_map; p_map="$(pad 14 "$map")"

            local status_col
            if is_map_running "$map"; then
                status_col="${GRN}${BLD}$(pad 10 "● ONLINE")${R}"
            else
                status_col="${RED}$(pad 10 "● OFFLINE")${R}"
            fi

            local health_raw health_col
            health_raw=$(check_map_health "$map")
            case "$health_raw" in
                HEALTHY)  health_col="${GRN}$(pad 9 "$health_raw")${R}" ;;
                DEGRADED) health_col="${YEL}$(pad 9 "$health_raw")${R}" ;;
                CRASHED|ZOMBIE|FROZEN) health_col="${RED}$(pad 9 "$health_raw")${R}" ;;
                *)        health_col="${DIM}$(pad 9 "$health_raw")${R}" ;;
            esac

            local p_players
            if is_map_running "$map"; then
                local pc max_p
                pc=$(get_player_count "$map" 2>/dev/null || echo "?")
                max_p=$(read_conf_value "$MAPS_DIR/$map/map.conf" "MaxPlayers" "70")
                p_players="$(pad 8 "${pc}/${max_p}")"
            else
                p_players="${DIM}$(pad 8 "-")${R}"
            fi

            local p_uptime p_ram p_cpu
            p_uptime="$(pad 7 "$(get_map_uptime "$map")")"
            p_ram="$(pad 6 "$(get_map_memory "$map")")"
            p_cpu="$(pad 5 "$(get_map_cpu "$map")")"

            local join_col
            if is_map_running "$map"; then
                local join_raw
                join_raw=$(check_server_queryable "$map")
                if [[ "$join_raw" == "YES" ]]; then
                    join_col="${GRN}$(pad 4 "✔")${R}"
                else
                    join_col="${RED}$(pad 4 "✖")${R}"
                fi
            else
                join_col="${DIM}$(pad 4 "-")${R}"
            fi

            local auto_col
            if is_autostart_enabled "$map" 2>/dev/null; then
                auto_col="${GRN}$(pad 4 "YES")${R}"
            else
                auto_col="${RED}$(pad 4 "NO")${R}"
            fi

            echo "  ${p_map} ${status_col} ${health_col} ${p_players} ${p_uptime} ${p_ram} ${p_cpu} ${join_col} ${auto_col}${CL}"
        done
    fi
    echo "${CL}"

    # Show feedback if any (always reserve 1 line for consistent layout)
    if [[ -n "$FEEDBACK" ]]; then
        thin_sep
        echo "  $FEEDBACK${CL}"
        FEEDBACK=""
    else
        echo "${CL}"
        echo "${CL}"
    fi
}

