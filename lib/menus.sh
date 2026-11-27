#@lib shellcheck shell=bash
#@lib Interactieve menu's (map kiezen, mapmenu, hoofdmenu) — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  SELECT MAP HELPER
#═══════════════════════════════════════════════════════════════════════════════

select_map() {
    local maps; maps=( $(get_maps) )
    (( ${#maps[@]} == 0 )) && { FEEDBACK="${YEL}⚠ No maps configured.${R}"; return 1; }

    clear
    echo ""
    echo "  ${BLD}${WHT}Select a map:${R}"
    thin_sep
    local i=1
    for m in "${maps[@]}"; do
        local icon
        if is_map_running "$m"; then icon="${GRN}●${R}"; else icon="${RED}●${R}"; fi
        printf "  ${icon} ${CYN}%2d${R}) %s\n" "$i" "$m"
        ((i++))
    done
    echo ""; echo "  ${DIM} 0) Back${R}"; echo ""
    echo -n "  ${WHT}Choice: ${R}"
    read -r choice
    [[ "$choice" == "0" || -z "$choice" ]] && return 1
    if (( choice > 0 && choice <= ${#maps[@]} )) 2>/dev/null; then
        SELECTED_MAP="${maps[$((choice-1))]}"
        return 0
    fi
    return 1
}

#═══════════════════════════════════════════════════════════════════════════════
#  MAP MENU — live status (2s), static options, input preserved
#═══════════════════════════════════════════════════════════════════════════════

map_menu() {
    local map="$1"
    local map_feedback="" pending="" need_clear=true

    while true; do
        # Guard: if map was deleted, exit this menu
        [[ ! -d "$MAPS_DIR/$map" ]] && return

        # Screen management: clear once, then cursor-home for flicker-free refresh
        if $need_clear; then
            clear
            need_clear=false
        else
            printf '\e[?25l\e[H'
        fi

        local internal
        internal=$(read_conf_value "$MAPS_DIR/$map/map.conf" "MapName" "?")

        echo "${CL}"
        echo "  ${BLD}${BLU}╔═══════════════════════════════════════════════════════════╗${R}${CL}"
        echo "  ${BLD}${BLU}║${R}  ${BLD}${WHT}$map${R}  ${DIM}($internal)${R}${CL}"
        echo "  ${BLD}${BLU}╚═══════════════════════════════════════════════════════════╝${R}${CL}"

        # === LIVE STATUS BLOCK (refreshes every 2s) ===
        if is_map_running "$map"; then
            local health_raw
            health_raw=$(check_map_health "$map")
            local status_text="${BG_GRN}${BLD}${WHT} ONLINE ${R}"
            local health_text
            case "$health_raw" in
                HEALTHY)  health_text="${GRN}${BLD}HEALTHY${R}" ;;
                DEGRADED) health_text="${YEL}${BLD}DEGRADED${R}" ;;
                *)        health_text="${RED}${BLD}$health_raw${R}" ;;
            esac
            local pc up mem cpu join_raw
            pc=$(get_player_count "$map" 2>/dev/null || echo "?")
            local max_p; max_p=$(read_conf_value "$MAPS_DIR/$map/map.conf" "MaxPlayers" "70")
            up=$(get_map_uptime "$map")
            mem=$(get_map_memory "$map")
            cpu=$(get_map_cpu "$map")
            join_raw=$(check_server_queryable "$map")
            local join_text
            [[ "$join_raw" == "YES" ]] && join_text="${GRN}${BLD}✔ YES${R}" || join_text="${RED}${BLD}✖ NO${R}"

            echo "  ${status_text}  ${health_text}  │  Players: ${BLD}${pc}/${max_p}${R}  │  Uptime: ${BLD}${up}${R}${CL}"
            echo "  RAM: ${BLD}${mem}${R}  │  CPU: ${BLD}${cpu}${R}  │  Join: ${join_text}${CL}"
        else
            echo "  ${BG_RED}${BLD}${WHT} OFFLINE ${R}  ${DIM}STOPPED${R}${CL}"
            echo "${CL}"
        fi

        # Groep / Auto-start status
        local groep_s auto_s
        is_batch_enabled "$map" && groep_s="${GRN}AAN${R}" || groep_s="${RED}UIT${R}"
        is_autostart_enabled "$map" 2>/dev/null && auto_s="${GRN}AAN${R}" || auto_s="${RED}UIT${R}"
        echo "  Groep: ${groep_s}  │  Auto-start: ${auto_s}${CL}"

        # Feedback area (always 1 line for consistent layout)
        if [[ -n "$map_feedback" ]]; then
            echo "  $map_feedback${CL}"
            map_feedback=""
        else
            echo "${CL}"
        fi
        echo "${CL}"

        # === STATIC MENU OPTIONS (reprinted but identical = no visible change) ===
        printf "  ${CYN}%2d${R}) ${GRN}%-20s${R}  ${CYN}%2d${R}) %s${CL}\n" 1 "Start" 8 "Restore Backup"
        printf "  ${CYN}%2d${R}) ${RED}%-20s${R}  ${CYN}%2d${R}) %s${CL}\n" 2 "Stop" 9 "View Logs"
        printf "  ${CYN}%2d${R}) ${YEL}%-20s${R}  ${CYN}%2d${R}) %s${CL}\n" 3 "Restart" 10 "Browse Files"
        printf "  ${CYN}%2d${R}) %-20s  ${CYN}%2d${R}) %s${CL}\n" 4 "RCON Console" 11 "Change Map Type"
        printf "  ${CYN}%2d${R}) %-20s  ${CYN}%2d${R}) %s${CL}\n" 5 "Server Settings" 12 "Toggle Groep"
        printf "  ${CYN}%2d${R}) %-20s  ${CYN}%2d${R}) %s${CL}\n" 6 "Mod Management" 13 "Toggle Auto-Start"
        printf "  ${CYN}%2d${R}) %-20s  ${CYN}%2d${R}) ${RED}%s${R}${CL}\n" 7 "Backup World" 14 "Delete Map"
        echo "${CL}"
        echo "  ${DIM} 0) Back to Dashboard${R}${CL}"
        echo "${CL}"

        # Clear old content below + show cursor
        printf '\e[J\e[?25h'

        # === PROMPT with preserved input ===
        if [[ -n "$pending" ]]; then
            printf "  ${GRY}↻ %s │${R} ${WHT}Keuze ${DIM}(Enter=ok, c=wis)${R}: ${BLD}%s${R}" "$(date +%H:%M:%S)" "$pending"
        else
            printf "  ${GRY}↻ %s │${R} ${WHT}Keuze: ${R}" "$(date +%H:%M:%S)"
        fi

        # Read input with 2s timeout — preserves partial input on timeout
        local input=""
        if read -t 2 -r input; then
            # Enter pressed — combine pending + new input
            if [[ ("$input" == "c" || "$input" == "C") && -n "$pending" ]]; then
                pending=""; continue
            fi
            local choice="${pending}${input}"
            pending=""
            [[ -z "$choice" ]] && continue

            case "$choice" in
                1)  need_clear=true; clear; start_map "$map"; press_enter ;;
                2)  need_clear=true; clear; stop_map "$map"; press_enter ;;
                3)  need_clear=true; clear; restart_map "$map"; press_enter ;;
                4)  need_clear=true; rcon_console "$map" ;;
                5)  need_clear=true; server_settings_menu "$map" ;;
                6)  need_clear=true; manage_mods "$map" ;;
                7)  need_clear=true; clear; backup_map "$map"; press_enter ;;
                8)  need_clear=true; restore_map "$map" ;;
                9)  need_clear=true; view_logs "$map" ;;
                10) need_clear=true; browse_files "$map" ;;
                11) need_clear=true; change_map_type "$map" ;;
                12) map_feedback=$(toggle_batch "$map") ;;
                13) map_feedback=$(toggle_autostart "$map") ;;
                14) need_clear=true; clear; delete_map "$map" && return ;;
                0)  return ;;
                *)  map_feedback="${RED}${BLD}✖${R} Ongeldige keuze: '${choice}'" ;;
            esac
        else
            # Timeout — preserve any partially typed input
            pending="${pending}${input}"
        fi
    done
}

#═══════════════════════════════════════════════════════════════════════════════
#  MAIN MENU — live dashboard (2s), static options, input preserved
#═══════════════════════════════════════════════════════════════════════════════

main_menu() {
    local pending="" need_clear=true

    while true; do
        # Screen management: clear once, then cursor-home for flicker-free refresh
        if $need_clear; then
            clear
            need_clear=false
        else
            printf '\e[?25l\e[H'
        fi

        # === LIVE DASHBOARD DATA (refreshes every 2s) ===
        show_dashboard

        # === STATIC MENU OPTIONS (reprinted but identical = no visible change) ===
        separator
        echo "${CL}"
        echo "  ${BLD}${WHT}Actions:${R}${CL}"
        echo "${CL}"
        printf "  ${CYN}%2d${R}) ${BLU}%-22s${R}${CYN}%2d${R}) %s${CL}\n" 1 "Select Map" 5 "Send RCON Command"
        printf "  ${CYN}%2d${R}) ${GRN}%-22s${R}${CYN}%2d${R}) %s${CL}\n" 2 "Add Map" 6 "Broadcast Message"
        printf "  ${CYN}%2d${R}) ${GRN}%-14s${R}${DIM}(groep)${R}  ${CYN}%2d${R}) %s${CL}\n" 3 "Start All" 7 "Scheduled Restarts"
        printf "  ${CYN}%2d${R}) ${RED}%-14s${R}${DIM}(groep)${R}  ${CYN}%2d${R}) %s${CL}\n" 4 "Stop All" 8 "Install / Update"
        echo "${CL}"
        echo "  ${DIM} 0) Exit${R}${CL}"
        echo "${CL}"

        # Clear old content below + show cursor
        printf '\e[J\e[?25h'

        # === PROMPT with preserved input ===
        if [[ -n "$pending" ]]; then
            printf "  ${GRY}↻ %s │${R} ${WHT}Keuze ${DIM}(Enter=ok, c=wis)${R}: ${BLD}%s${R}" "$(date +%H:%M:%S)" "$pending"
        else
            printf "  ${GRY}↻ %s │${R} ${WHT}Keuze: ${R}" "$(date +%H:%M:%S)"
        fi

        # Read input with 2s timeout — preserves partial input on timeout
        local input=""
        if read -t 2 -r input; then
            # Enter pressed — combine pending + new input
            if [[ ("$input" == "c" || "$input" == "C") && -n "$pending" ]]; then
                pending=""; continue
            fi
            local choice="${pending}${input}"
            pending=""
            [[ -z "$choice" ]] && continue

            need_clear=true
            case "$choice" in
                1)  select_map && map_menu "$SELECTED_MAP" ;;
                2)  add_map ;;
                3)  clear; start_all_maps; press_enter ;;
                4)  clear; stop_all_maps; press_enter ;;
                5)
                    if select_map; then
                        echo -n "  ${WHT}RCON command: ${R}"
                        read -r cmd
                        [[ -n "$cmd" ]] && { clear; send_rcon "$SELECTED_MAP" "$cmd"; press_enter; }
                    fi
                    ;;
                6)
                    clear
                    echo ""
                    echo -n "  ${WHT}Message: ${R}"
                    read -r msg
                    [[ -n "$msg" ]] && broadcast_message "$msg"
                    sleep 1
                    ;;
                7)  setup_scheduled_restart ;;
                8)  install_server ;;
                0)
                    echo ""
                    echo "  ${GRN}${BLD}Goodbye! 🦖${R}"
                    echo ""
                    exit 0
                    ;;
                *)  FEEDBACK="${RED}${BLD}✖${R} Ongeldige keuze: '${choice}'" ;;
            esac
        else
            # Timeout — preserve any partially typed input
            pending="${pending}${input}"
        fi
    done
}

