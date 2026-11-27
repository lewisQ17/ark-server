#@lib shellcheck shell=bash
#@lib Start/stop/restart van maps — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  START / STOP / RESTART
#═══════════════════════════════════════════════════════════════════════════════

start_map() {
    local map="$1"
    local conf="$MAPS_DIR/$map/map.conf"
    [[ ! -f "$conf" ]] && { log_err "Map '$map' not found."; return 1; }
    if is_map_running "$map"; then
        log_warn "Map '${BLD}$map${R}' is already running."
        return 0
    fi
    check_cpu_flags || return 1

    local map_name save_dir game_port query_port rcon_port max_players
    local admin_pw server_pw custom_params cluster_id
    map_name=$(read_conf_value "$conf" "MapName")
    save_dir=$(read_conf_value "$conf" "SaveDir" "$map")
    game_port=$(read_conf_value "$conf" "GamePort" "7777")
    query_port=$(read_conf_value "$conf" "QueryPort" "27015")
    rcon_port=$(read_conf_value "$conf" "RCONPort" "27020")
    max_players=$(read_conf_value "$conf" "MaxPlayers" "70")
    admin_pw=$(read_conf_value "$conf" "AdminPassword" "")
    server_pw=$(read_conf_value "$conf" "ServerPassword" "")
    custom_params=$(read_conf_value "$conf" "CustomStartParams" "-NoBattlEye -crossplay -NoHangDetection")
    cluster_id=$(read_conf_value "$conf" "ClusterID" "")

    local mod_ids; mod_ids=$(get_enabled_mod_ids "$map")

    if ss -lunp 2>/dev/null | grep -q ":${game_port} "; then
        log_err "Port ${BLD}$game_port${R} is already in use!"
        return 1
    fi

    log_info "Starting map: ${BLD}$map${R} (${CYN}$map_name${R})..."

    export STEAM_COMPAT_DATA_PATH="$SERVER_DIR/steamapps/compatdata/$ARK_APPID"
    export STEAM_COMPAT_CLIENT_INSTALL_PATH="$STEAMCMD_DIR"
    export SteamAppId=$ARK_APPID
    export SteamGameId=$ARK_APPID

    # Hier werd bij elke start EEN globale symlink (Saved/Config/WindowsServer ->
    # maps/<map>) herlegd, zonder per-map component. Bij meerdere maps wees die dus naar
    # de laatst gestarte map: gameplay-settings lekten over en de config-write-back bij
    # afsluiten overschreef de ini's van een ANDERE map. Nu blijft maps/<map>/*.ini de
    # bron van deze map en zetten we alleen een werkkopie op de engine-locatie, zodat
    # geen enkele map nog in de config van een andere map schrijft.
    local config_src="$MAPS_DIR/$map"
    local config_dst="$SERVER_DIR/ShooterGame/Saved/Config/WindowsServer"
    [[ -L "$config_dst" ]] && rm -f "$config_dst"
    mkdir -p "$config_dst"
    local ini
    for ini in "$config_src"/*.ini; do
        [[ -f "$ini" ]] || continue
        cp -f "$ini" "$config_dst/$(basename "$ini")"
    done
    mkdir -p "$SERVER_DIR/ShooterGame/Saved/SavedArks/$save_dir"

    local cluster_args=""
    if [[ -n "$cluster_id" ]]; then
        local cluster_dir="$SCRIPT_DIR/clusters/$cluster_id"
        mkdir -p "$cluster_dir"
        cluster_args="-ClusterDirOverride=\"$cluster_dir\" -ClusterId=\"$cluster_id\""
    fi

    local nullrhi=""
    local use_null; use_null=$(read_conf_value "$OPT_CONF" "UseNullRHI" "true")
    [[ "$use_null" == "true" ]] && nullrhi="-nullrhi"
    local nice_lvl; nice_lvl=$(read_conf_value "$OPT_CONF" "ServerNiceLevel" "-5")

    # Admin- en serverwachtwoord stonden permanent in de process-commandline en waren dus
    # via 'ps' voor elke lokale gebruiker leesbaar. ARK leest ServerPassword /
    # ServerAdminPassword / RCON ook uit GameUserSettings.ini [ServerSettings], en dat
    # bestand staat op mode 600 — dus daar (opnieuw) neerzetten en uit argv weglaten.
    # Fail-closed: ontbrak het bestand of de [ServerSettings]-sectie, dan startte de
    # server zonder wachtwoord (of belandden de sleutels onder een andere sectie, waar
    # ARK ze niet leest). Nu maken we bestand en sectie desnoods aan, zetten de sleutels
    # direct onder [ServerSettings], en starten niet als dat niet lukt.
    local gus_live="$config_dst/GameUserSettings.ini"
    if [[ "$server_pw$admin_pw" == *$'\n'* ]]; then
        log_err "Server/admin password for '$map' contains a newline — refusing to start."
        return 1
    fi
    local gus_tmp="$gus_live.tmp.$$"
    if ! ( umask 077
           [[ -f "$gus_live" ]] || : > "$gus_live"
           SP="$server_pw" AP="$admin_pw" RP="$rcon_port" awk '
               function emit() {
                   print "ServerPassword=" ENVIRON["SP"]
                   print "ServerAdminPassword=" ENVIRON["AP"]
                   print "RCONEnabled=True"
                   print "RCONPort=" ENVIRON["RP"]
                   done = 1
               }
               /^(ServerPassword|ServerAdminPassword|RCONEnabled|RCONPort)=/ { next }
               { print }
               /^\[ServerSettings\][[:space:]]*$/ && !done { emit() }
               END { if (!done) { print "[ServerSettings]"; emit() } }
           ' "$gus_live" > "$gus_tmp" && mv -f "$gus_tmp" "$gus_live" && chmod 600 "$gus_live" ); then
        rm -f "$gus_tmp"
        log_err "Could not write passwords to GameUserSettings.ini for '$map' — refusing to start without them."
        return 1
    fi

    nice $nice_lvl "$PROTON_DIR/proton" run \
        "$SERVER_DIR/ShooterGame/Binaries/Win64/ArkAscendedServer.exe" \
        "${map_name}?listen?SessionName=${map}?AltSaveDirectoryName=${save_dir}" \
        $custom_params $nullrhi \
        -WinLiveMaxPlayers=$max_players \
        -Port=$game_port -QueryPort=$query_port -RCONPort=$rcon_port \
        -game $cluster_args -server -log \
        -mods="$mod_ids" \
        > "$MAPS_DIR/$map/server.log" 2>&1 &

    log_ok "Map '${BLD}$map${R}${GRN}' starting... ~60 seconds to come online."
}

stop_map() {
    local map="$1"
    local conf="$MAPS_DIR/$map/map.conf"
    [[ ! -f "$conf" ]] && { log_err "Map '$map' not found."; return 1; }
    if ! is_map_running "$map"; then
        log_warn "Map '${BLD}$map${R}' is not running."
        return 0
    fi
    local save_dir admin_pw rcon_port
    save_dir=$(read_conf_value "$conf" "SaveDir" "$map")
    admin_pw=$(read_conf_value "$conf" "AdminPassword" "")
    rcon_port=$(read_conf_value "$conf" "RCONPort" "27020")

    log_info "Stopping map '${BLD}$map${R}'..."
    # Twee fouten hier: (1) het pgrep/pkill-patroon was niet geankerd, dus 'Ragnarok'
    # matchte ook 'Ragnarok2' en 'stop Ragnarok' kon die andere map hard killen; (2) na
    # een mislukte RCON volgde al na 3s SIGKILL en werd tóch "stopped" gelogd, terwijl
    # ARK bij SIGKILL niets opslaat — stil wereld-verlies. Nu geankerd, ruimer wachten,
    # en een force-kill wordt als fout gerapporteerd (return 1).
    local proc_pat="ArkAscendedServer\.exe.*AltSaveDirectoryName=${save_dir}( |\$)"
    local stop_rc=0
    local response=""
    response=$(python3 "$RCON_SCRIPT" "localhost:$rcon_port" -p "$admin_pw" -c "DoExit" 2>/dev/null || echo "")
    if [[ "$response" == *"Exiting"* ]]; then
        log_info "Waiting for graceful shutdown..."
        local waited=0
        while pgrep -f "$proc_pat" &>/dev/null; do
            sleep 2; (( waited += 2 ))
            (( waited >= 120 )) && {
                log_err "Timeout after ${waited}s — sending SIGKILL. The world save is probably incomplete."
                pkill -9 -f "$proc_pat" || true
                stop_rc=1
                break
            }
        done
    else
        log_warn "RCON gave no 'Exiting' reply — falling back to a signal-based stop."
        pkill -f "$proc_pat" || true
        local waited=0
        while pgrep -f "$proc_pat" &>/dev/null && (( waited < 30 )); do
            sleep 2; (( waited += 2 ))
        done
        if pgrep -f "$proc_pat" &>/dev/null; then
            log_err "Map '$map' still alive after ${waited}s — sending SIGKILL. NO world save was written; the last autosave is the newest data."
            pkill -9 -f "$proc_pat" 2>/dev/null || true
            stop_rc=1
        else
            log_warn "Map '$map' exited on SIGTERM without an RCON confirmation — check maps/$map/server.log to verify the world was saved."
        fi
    fi
    pkill -f "wineserver.*${save_dir}( |\$)" 2>/dev/null || true
    if (( stop_rc != 0 )); then
        log_err "Map '${BLD}$map${R}${RED}' was force-stopped, not saved. Restore a backup if the world looks wrong."
        return 1
    fi
    log_ok "Map '${BLD}$map${R}${GRN}' stopped."
}

restart_map() {
    local map="$1"
    stop_map "$map"
    sleep 3
    start_map "$map"
}

start_all_maps() {
    local maps; maps=( $(get_maps) )
    (( ${#maps[@]} == 0 )) && { log_warn "No maps configured."; return; }
    local started=0
    for map in "${maps[@]}"; do
        if is_batch_enabled "$map"; then
            start_map "$map"
            ((started++))
            sleep 5
        else
            log_info "Skipping ${BLD}$map${R} ${DIM}(not in groep)${R}"
        fi
    done
    log_ok "Started ${BLD}$started${R}${GRN} maps in groep."
}

stop_all_maps() {
    local maps; maps=( $(get_maps) )
    local stopped=0
    for map in "${maps[@]}"; do
        if is_batch_enabled "$map" && is_map_running "$map"; then
            stop_map "$map"
            ((stopped++))
        elif is_map_running "$map"; then
            log_info "Skipping ${BLD}$map${R} ${DIM}(not in groep)${R}"
        fi
    done
    log_ok "Stopped ${BLD}$stopped${R}${GRN} maps in groep."
}

