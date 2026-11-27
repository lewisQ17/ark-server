#@lib shellcheck shell=bash
#@lib Per-map beheer: instellingen, backup/restore, logs, bestanden, verwijderen, type, batch/autostart — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  SERVER SETTINGS (per-map)
#═══════════════════════════════════════════════════════════════════════════════

server_settings_menu() {
    local map="$1"
    local gus="$MAPS_DIR/$map/GameUserSettings.ini"

    while true; do
        clear
        echo ""
        echo "  ${BLD}${MAG}═══ Server Settings — ${CYN}$map${MAG} ═══${R}"

        local xp taming harvest respawn mating baby hatch stack max_tamed auto_save max_struct
        xp=$(read_conf_value "$gus" "XPMultiplier" "1.0")
        taming=$(read_conf_value "$gus" "TamingSpeedMultiplier" "1.0")
        harvest=$(read_conf_value "$gus" "HarvestAmountMultiplier" "1.0")
        respawn=$(read_conf_value "$gus" "ResourcesRespawnPeriodMultiplier" "1.0")
        mating=$(read_conf_value "$gus" "MatingIntervalMultiplier" "1.0")
        baby=$(read_conf_value "$gus" "BabyMatureSpeedMultiplier" "1.0")
        hatch=$(read_conf_value "$gus" "EggHatchSpeedMultiplier" "1.0")
        stack=$(read_conf_value "$gus" "ItemStackSizeMultiplier" "1.0")
        max_tamed=$(read_conf_value "$gus" "MaxTamedDinos" "5000")
        auto_save=$(read_conf_value "$gus" "AutoSavePeriodMinutes" "15")
        max_struct=$(read_conf_value "$gus" "TheMaxStructuresInRange" "10500")

        echo ""
        echo "  ${BLD}${WHT}  Multipliers${R}"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 1 "XP Multiplier" "$xp"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 2 "Taming Speed" "$taming"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 3 "Harvest Amount" "$harvest"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 4 "Resource Respawn" "$respawn"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 5 "Mating Interval" "$mating"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 6 "Baby Mature Speed" "$baby"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 7 "Egg Hatch Speed" "$hatch"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 8 "Item Stack Size" "$stack"
        echo ""
        echo "  ${BLD}${WHT}  Limits${R}"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 9 "Max Tamed Dinos" "$max_tamed"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 10 "Auto-Save Interval (min)" "$auto_save"
        printf "  ${CYN}%2d${R}) %-30s : ${BLD}${WHT}%s${R}\n" 11 "Max Structures In Range" "$max_struct"
        echo ""
        echo "  ${BLD}${WHT}  Advanced${R}"
        printf "  ${CYN}%2d${R}) %-30s   ${DIM}(low/medium/high/auto)${R}\n" 12 "Graphics Preset"
        printf "  ${CYN}%2d${R}) %-30s   ${DIM}(nano)${R}\n" 13 "Edit GameUserSettings.ini"
        printf "  ${CYN}%2d${R}) %-30s   ${DIM}(nano)${R}\n" 14 "Edit Game.ini"
        printf "  ${CYN}%2d${R}) %-30s   ${DIM}(ports/passwords)${R}\n" 15 "Edit map.conf"
        echo ""
        echo "  ${DIM} 0) Back${R}"
        echo ""
        echo -n "  ${WHT}Choice: ${R}"
        read -r choice

        case "$choice" in
            1)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "XPMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            2)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "TamingSpeedMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            3)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "HarvestAmountMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            4)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "ResourcesRespawnPeriodMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            5)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "MatingIntervalMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            6)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "BabyMatureSpeedMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            7)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "EggHatchSpeedMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            8)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "ItemStackSizeMultiplier" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            9)  echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "MaxTamedDinos" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            10) echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "AutoSavePeriodMinutes" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            11) echo -n "  ${WHT}New value ${DIM}(empty=cancel)${R}: "; read -r val
                [[ -n "$val" ]] && { write_conf_value "$gus" "TheMaxStructuresInRange" "$val"; log_ok "Saved."; sleep 0.5; } ;;
            12) graphics_preset_menu "$map" ;;
            13) ${EDITOR:-nano} "$gus" ;;
            14) ${EDITOR:-nano} "$MAPS_DIR/$map/Game.ini" ;;
            15) ${EDITOR:-nano} "$MAPS_DIR/$map/map.conf" ;;
            0|"") return ;;
            *) ;;
        esac
    done
}

#═══════════════════════════════════════════════════════════════════════════════
#  BACKUP & RESTORE
#═══════════════════════════════════════════════════════════════════════════════

backup_map() {
    local map="$1"
    local save_dir; save_dir=$(read_conf_value "$MAPS_DIR/$map/map.conf" "SaveDir" "$map")
    local source="$SERVER_DIR/ShooterGame/Saved/SavedArks/$save_dir"
    local backup_dir="$MAPS_DIR/$map/backups"
    local backup_file="$backup_dir/${map}_$(date +%Y%m%d_%H%M%S).tar.gz"
    [[ ! -d "$source" ]] && { log_err "No save data for '${BLD}$map${R}'."; return 1; }
    mkdir -p "$backup_dir"
    log_info "Backing up '${BLD}$map${R}'..."
    tar -czf "$backup_file" -C "$SERVER_DIR/ShooterGame/Saved/SavedArks" "$save_dir"
    local size; size=$(du -h "$backup_file" | cut -f1)
    log_ok "Backup: ${DIM}$backup_file${R} (${BLD}$size${R})"
}

restore_map() {
    local map="$1"
    local backup_dir="$MAPS_DIR/$map/backups"
    [[ ! -d "$backup_dir" ]] || [[ -z "$(ls -A "$backup_dir" 2>/dev/null)" ]] && { log_err "No backups for '${BLD}$map${R}'."; return 1; }
    is_map_running "$map" && { log_err "Stop the map first!"; return 1; }

    clear
    echo ""
    echo "  ${BLD}${MAG}═══ Restore Backup — ${CYN}$map${MAG} ═══${R}"
    thin_sep
    local backups=()
    local i=1
    for f in "$backup_dir"/*.tar.gz; do
        local size fname; size=$(du -h "$f" | cut -f1); fname=$(basename "$f")
        printf "  ${CYN}%2d${R}) %-40s ${DIM}(%s)${R}\n" "$i" "$fname" "$size"
        backups+=("$f"); ((i++))
    done
    echo ""; echo "  ${DIM} 0) Back${R}"; echo ""
    echo -n "  ${WHT}Select backup: ${R}"
    read -r choice
    [[ "$choice" == "0" || -z "$choice" ]] && return 0
    (( choice < 1 || choice > ${#backups[@]} )) 2>/dev/null && return 0

    local save_dir; save_dir=$(read_conf_value "$MAPS_DIR/$map/map.conf" "SaveDir" "$map")
    if confirm "Overwrite current save data for '${BLD}$map${R}'?"; then
        log_info "Restoring backup..."
        rm -rf "$SERVER_DIR/ShooterGame/Saved/SavedArks/$save_dir"
        tar -xzf "${backups[$((choice-1))]}" -C "$SERVER_DIR/ShooterGame/Saved/SavedArks"
        log_ok "Backup restored!"
    fi
    sleep 1
}

#═══════════════════════════════════════════════════════════════════════════════
#  LOG VIEWER
#═══════════════════════════════════════════════════════════════════════════════

view_logs() {
    local map="$1"
    local log_file="$MAPS_DIR/$map/server.log"
    local steam_log="$HOME/steam-${ARK_APPID}.log"

    while true; do
        clear
        echo ""
        echo "  ${BLD}${MAG}═══ Logs — ${CYN}$map${MAG} ═══${R}"
        thin_sep
        echo "  ${CYN}1${R}) Server log ${DIM}(last 50 lines)${R}"
        echo "  ${CYN}2${R}) Follow log ${DIM}(live — Ctrl+C to stop)${R}"
        echo "  ${CYN}3${R}) Steam/Proton log"
        echo "  ${CYN}4${R}) Errors only"
        echo ""; echo "  ${DIM}0) Back${R}"; echo ""
        echo -n "  ${WHT}Choice: ${R}"
        read -r choice
        case "$choice" in
            1) [[ -f "$log_file" ]] && { echo ""; separator; tail -50 "$log_file"; separator; } || log_warn "No log."; press_enter ;;
            2) [[ -f "$log_file" ]] && { log_info "Ctrl+C to stop..."; tail -f "$log_file" || true; } || log_warn "No log." ;;
            3) [[ -f "$steam_log" ]] && { separator; tail -80 "$steam_log"; separator; } || log_warn "No Steam log."; press_enter ;;
            4) [[ -f "$log_file" ]] && { separator; grep -iE "error|fatal|crash|exception|fail" "$log_file" | tail -30 || log_info "No errors."; separator; } || log_warn "No log."; press_enter ;;
            0|"") return ;;
        esac
    done
}

#═══════════════════════════════════════════════════════════════════════════════
#  FILE BROWSER
#═══════════════════════════════════════════════════════════════════════════════

browse_files() {
    local map="$1"
    while true; do
        clear
        echo ""
        echo "  ${BLD}${MAG}═══ File Locations — ${CYN}$map${MAG} ═══${R}"
        thin_sep
        echo "  ${CYN}1${R}) Map config dir      ${DIM}$MAPS_DIR/$map/${R}"
        echo "  ${CYN}2${R}) Save data"
        echo "  ${CYN}3${R}) Backups"
        echo "  ${CYN}4${R}) Server files"
        echo "  ${CYN}5${R}) Open shell here"
        echo ""; echo "  ${DIM}0) Back${R}"; echo ""
        echo -n "  ${WHT}Choice: ${R}"
        read -r choice
        case "$choice" in
            1) echo ""; ls -la "$MAPS_DIR/$map/" 2>/dev/null; press_enter ;;
            2) ls -la "$SERVER_DIR/ShooterGame/Saved/SavedArks/$(read_conf_value "$MAPS_DIR/$map/map.conf" "SaveDir" "$map")/" 2>/dev/null; press_enter ;;
            3) ls -la "$MAPS_DIR/$map/backups/" 2>/dev/null; press_enter ;;
            4) ls -la "$SERVER_DIR/" 2>/dev/null; press_enter ;;
            5) log_info "Shell at: ${BLD}$MAPS_DIR/$map/${R}"; cd "$MAPS_DIR/$map" && exec bash ;;
            0|"") return ;;
        esac
    done
}

#═══════════════════════════════════════════════════════════════════════════════
#  DELETE MAP (double confirmation!)
#═══════════════════════════════════════════════════════════════════════════════

delete_map() {
    local map="$1"
    is_map_running "$map" && { log_err "Stop the map first!"; return 1; }

    echo ""
    log_warn "${RED}${BLD}PERMANENTLY DELETE${R} map '${BLD}$map${R}' and all configuration?"
    echo ""
    if ! confirm "Are you sure you want to delete '$map'?"; then
        log_info "Cancelled."
        sleep 1
        return 1
    fi

    echo ""
    log_warn "${RED}${BLD}LAST CHANCE!${R} This will ${RED}permanently${R} remove:"
    echo "    - Map config, mods, backups"
    echo "    - Systemd service"
    echo "    - All data for '${BLD}$map${R}'"
    echo ""
    if ! confirm "Type 'y' to PERMANENTLY delete '$map'"; then
        log_info "Cancelled."
        sleep 1
        return 1
    fi

    local service_name="ark-${map,,}.service"
    "${SUDO[@]}" systemctl stop "$service_name" 2>/dev/null || true
    "${SUDO[@]}" systemctl disable "$service_name" 2>/dev/null || true
    "${SUDO[@]}" rm -f "/etc/systemd/system/$service_name"
    "${SUDO[@]}" systemctl daemon-reload
    sed -i "/^${map}|/d" "$MAPS_CONF"
    rm -rf "$MAPS_DIR/$map"
    rm -f "/tmp/.ark-players-${map}" "/tmp/.ark-join-${map}"
    FEEDBACK="${GRN}${BLD}✔${R} Map '${BLD}$map${R}' deleted."
    return 0
}

#═══════════════════════════════════════════════════════════════════════════════
#  CHANGE MAP TYPE
#═══════════════════════════════════════════════════════════════════════════════

change_map_type() {
    local map="$1"
    is_map_running "$map" && { log_err "Stop the map first!"; return 1; }
    clear
    echo ""
    echo "  ${BLD}${MAG}═══ Change Map Type — ${CYN}$map${MAG} ═══${R}"
    thin_sep
    local i=1; local map_keys=()
    for key in $(echo "${!MAP_NAMES[@]}" | tr ' ' '\n' | sort); do
        printf "  ${CYN}%2d${R}) %-20s  ${DIM}(%s)${R}\n" "$i" "$key" "${MAP_NAMES[$key]}"
        map_keys+=("$key"); ((i++))
    done
    echo ""; echo "  ${DIM} 0) Back${R}"; echo ""
    echo -n "  ${WHT}Select: ${R}"
    read -r choice
    [[ "$choice" == "0" || -z "$choice" ]] && return 0
    if (( choice > 0 && choice <= ${#map_keys[@]} )) 2>/dev/null; then
        local new_internal="${MAP_NAMES[${map_keys[$((choice-1))]}]}"
        write_conf_value "$MAPS_DIR/$map/map.conf" "MapName" "$new_internal"
        log_ok "Changed to ${BLD}${map_keys[$((choice-1))]}${R}${GRN}. Restart to apply."
        sleep 1
    fi
}

#═══════════════════════════════════════════════════════════════════════════════
#  TOGGLE GROEP / AUTO-START (return feedback via stdout)
#═══════════════════════════════════════════════════════════════════════════════

toggle_batch() {
    local map="$1"
    local current; current=$(read_conf_value "$MAPS_DIR/$map/map.conf" "BatchEnabled" "true")
    if [[ "$current" == "true" ]]; then
        write_conf_value "$MAPS_DIR/$map/map.conf" "BatchEnabled" "false"
        echo "${GRN}${BLD}✔${R} ${BLD}$map${R}: Groep ${RED}UIT${R} — wordt overgeslagen bij Start All / Stop All"
    else
        write_conf_value "$MAPS_DIR/$map/map.conf" "BatchEnabled" "true"
        echo "${GRN}${BLD}✔${R} ${BLD}$map${R}: Groep ${GRN}AAN${R} — doet mee met Start All / Stop All"
    fi
}

toggle_autostart() {
    local map="$1"
    local service="ark-${map,,}.service"
    if systemctl is-enabled "$service" 2>/dev/null | grep -q "enabled"; then
        "${SUDO[@]}" systemctl disable "$service" 2>/dev/null
        echo "${GRN}${BLD}✔${R} ${BLD}$map${R}: Auto-start ${RED}UIT${R} — start niet na reboot/stroomstoring"
    else
        "${SUDO[@]}" systemctl enable "$service" 2>/dev/null
        echo "${GRN}${BLD}✔${R} ${BLD}$map${R}: Auto-start ${GRN}AAN${R} — start automatisch na reboot/stroomstoring"
    fi
}

