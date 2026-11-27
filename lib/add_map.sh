#@lib shellcheck shell=bash
#@lib Map toevoegen (menu en CLI) — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  ADD MAP
#═══════════════════════════════════════════════════════════════════════════════

add_map() {
    clear
    echo ""
    echo "  ${BLD}${MAG}═══ Add New Map ═══${R}"
    echo ""

    local i=1
    local map_keys=()
    for key in $(echo "${!MAP_NAMES[@]}" | tr ' ' '\n' | sort); do
        printf "  ${CYN}%2d${R}) %-20s  ${DIM}(%s)${R}\n" "$i" "$key" "${MAP_NAMES[$key]}"
        map_keys+=("$key")
        ((i++))
    done
    printf "  ${CYN}%2d${R}) ${YEL}Custom map name${R}\n" "$i"
    echo ""
    echo "  ${DIM} 0) Back${R}"
    echo ""
    echo -n "  ${WHT}Select map number: ${R}"
    read -r choice

    [[ "$choice" == "0" || -z "$choice" ]] && return 0

    local display_name internal_name
    if (( choice > 0 && choice <= ${#map_keys[@]} )) 2>/dev/null; then
        display_name="${map_keys[$((choice-1))]}"
        internal_name="${MAP_NAMES[$display_name]}"
    elif (( choice == i )) 2>/dev/null; then
        echo -n "  ${WHT}Display name (e.g. MyMap): ${R}"
        read -r display_name
        [[ -z "$display_name" ]] && return 0
        # Een naam met een spatie (of quote/slash) brak de manager stil: get_maps levert
        # de mapnamen spatie-gescheiden op, dus "My Map" werd twee fantoom-maps, en de
        # naam gaat ook ongefilterd in een als root geschreven systemd-unit.
        if [[ ! "$display_name" =~ ^[A-Za-z0-9_]+$ ]]; then
            FEEDBACK="${RED}${BLD}✖${R} Invalid name '${display_name}' — only letters, digits and _ (no spaces)."
            return 1
        fi
        echo -n "  ${WHT}Internal map name (e.g. MyMap_WP): ${R}"
        read -r internal_name
        [[ -z "$internal_name" ]] && return 0
        if [[ ! "$internal_name" =~ ^[A-Za-z0-9_]+$ ]]; then
            FEEDBACK="${RED}${BLD}✖${R} Invalid internal name '${internal_name}' — only letters, digits and _."
            return 1
        fi
    else
        FEEDBACK="${RED}${BLD}✖${R} Invalid selection."
        return 1
    fi

    if [[ -d "$MAPS_DIR/$display_name" ]]; then
        FEEDBACK="${RED}${BLD}✖${R} Map '${BLD}$display_name${R}' already exists."
        return 1
    fi

    # Defaults
    local def_port def_qport def_rport def_maxp def_adminpw
    def_port=$(read_conf_value "$DEFAULTS_CONF" "DefaultGamePort" "7777")
    def_qport=$(read_conf_value "$DEFAULTS_CONF" "DefaultQueryPort" "27015")
    def_rport=$(read_conf_value "$DEFAULTS_CONF" "DefaultRCONPort" "27020")
    def_maxp=$(read_conf_value "$DEFAULTS_CONF" "DefaultMaxPlayers" "70")
    def_adminpw=$(read_conf_value "$DEFAULTS_CONF" "DefaultAdminPassword" "")

    # Auto-increment ports
    local existing_maps; existing_maps=( $(get_maps) )
    if (( ${#existing_maps[@]} > 0 )); then
        local max_gport=0 max_qport=0 max_rport=0
        for em in "${existing_maps[@]}"; do
            local gp qp rp
            gp=$(read_conf_value "$MAPS_DIR/$em/map.conf" "GamePort" "0")
            qp=$(read_conf_value "$MAPS_DIR/$em/map.conf" "QueryPort" "0")
            rp=$(read_conf_value "$MAPS_DIR/$em/map.conf" "RCONPort" "0")
            (( gp > max_gport )) && max_gport=$gp
            (( qp > max_qport )) && max_qport=$qp
            (( rp > max_rport )) && max_rport=$rp
        done
        def_port=$(( max_gport + 2 ))
        def_qport=$(( max_qport + 1 ))
        def_rport=$(( max_rport + 1 ))
    fi

    echo ""
    echo "  ${BLD}Configure ${CYN}${display_name}${R}${BLD}:${R}  ${DIM}(Enter = default, 0 = cancel)${R}"
    thin_sep

    local game_port query_port rcon_port max_players admin_pw server_pw
    echo -n "  ${WHT}Game Port${R} ${DIM}[$def_port]${R}: "; read -r input; [[ "$input" == "0" ]] && return 0; game_port="${input:-$def_port}"
    echo -n "  ${WHT}Query Port${R} ${DIM}[$def_qport]${R}: "; read -r input; [[ "$input" == "0" ]] && return 0; query_port="${input:-$def_qport}"
    echo -n "  ${WHT}RCON Port${R} ${DIM}[$def_rport]${R}: "; read -r input; [[ "$input" == "0" ]] && return 0; rcon_port="${input:-$def_rport}"
    echo -n "  ${WHT}Max Players${R} ${DIM}[$def_maxp]${R}: "; read -r input; [[ "$input" == "0" ]] && return 0; max_players="${input:-$def_maxp}"
    echo -n "  ${WHT}Admin Password${R} ${DIM}[$def_adminpw]${R}: "; read -r input; [[ "$input" == "0" ]] && return 0; admin_pw="${input:-$def_adminpw}"
    if [[ -z "$admin_pw" ]]; then
        log_warn "Admin password left empty — RCON/admin access will be unsecured."
        if ! confirm "Proceed with empty AdminPassword (not recommended)?"; then
            FEEDBACK="${YEL}${BLD}⚠${R} Map creation cancelled. Please choose a safer Admin Password."
            return 1
        fi
    fi
    echo -n "  ${WHT}Server Password${R} ${DIM}[empty=public]${R}: "; read -r server_pw; [[ "$server_pw" == "0" ]] && return 0

    mkdir -p "$MAPS_DIR/$display_name/backups"

    cat > "$MAPS_DIR/$display_name/map.conf" <<EOF
MapName=$internal_name
DisplayName=$display_name
SaveDir=$display_name
GamePort=$game_port
QueryPort=$query_port
RCONPort=$rcon_port
MaxPlayers=$max_players
AdminPassword=$admin_pw
ServerPassword=$server_pw
CustomStartParams=$(read_conf_value "$DEFAULTS_CONF" "DefaultStartParams" "-NoBattlEye -crossplay -NoHangDetection")
ClusterID=$(read_conf_value "$DEFAULTS_CONF" "ClusterID" "")
BatchEnabled=true
EOF

    chmod 600 "$MAPS_DIR/$display_name/map.conf" 2>/dev/null || true

    # Create empty mods.conf
    echo "# Mod configuration for $display_name" > "$MAPS_DIR/$display_name/mods.conf"
    echo "# Format: id|name|enabled" >> "$MAPS_DIR/$display_name/mods.conf"

    create_optimized_game_settings "$MAPS_DIR/$display_name" "high"
    [[ ! -f "$MAPS_DIR/$display_name/Game.ini" ]] && touch "$MAPS_DIR/$display_name/Game.ini"
    echo "${display_name}|${internal_name}|enabled" >> "$MAPS_CONF"
    create_systemd_service "$display_name"
    open_firewall_ports "$game_port" "$query_port" "$rcon_port"

    FEEDBACK="${GRN}${BLD}✔${R} Map '${BLD}$display_name${R}' created and ready!"
}

#-------------------------------------------------------------------------------
#  add_map_cli — non-interactive map creation (used by install.sh and automation)
#
#  Usage: ark-manager.sh add-map <DisplayName> [--internal NAME] [--game-port N]
#                                [--query-port N] [--rcon-port N] [--max-players N]
#                                [--admin-pw PW] [--server-pw PW]
#
#  Ports auto-increment past the highest existing map, exactly like the menu does.
#  An admin password is REQUIRED (generated if omitted) — an empty one leaves RCON
#  wide open, which is fine to be asked about interactively but never a safe default
#  for automation.
#-------------------------------------------------------------------------------
add_map_cli() {
    local display_name="${1:-}"; shift || true
    [[ -z "$display_name" ]] && { echo "Usage: $0 add-map <DisplayName> [--internal NAME] [--game-port N] ..."; return 1; }

    local internal_name="" game_port="" query_port="" rcon_port="" max_players="" admin_pw="" server_pw=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --internal)     internal_name="${2:-}"; shift 2 ;;
            --game-port)    game_port="${2:-}";     shift 2 ;;
            --query-port)   query_port="${2:-}";    shift 2 ;;
            --rcon-port)    rcon_port="${2:-}";     shift 2 ;;
            --max-players)  max_players="${2:-}";   shift 2 ;;
            --admin-pw)     admin_pw="${2:-}";      shift 2 ;;
            --server-pw)    server_pw="${2:-}";     shift 2 ;;
            *) echo "Unknown option for add-map: $1"; return 1 ;;
        esac
    done

    # Deze CLI-route valideerde de naam niet (het menu doet dat wel): de naam belandt in
    # paden, in een als root geschreven systemd-unit en in de start-commandline. Zelfde
    # regel als in het menu: alleen letters, cijfers en _.
    if [[ ! "$display_name" =~ ^[A-Za-z0-9_]+$ ]]; then
        log_err "Invalid map name '$display_name' — only letters, digits and _ are allowed, no spaces."
        return 1
    fi

    if [[ -d "$MAPS_DIR/$display_name" ]]; then
        log_ok "Map '$display_name' already exists — nothing to do."
        return 0
    fi

    # Known map? then we know its internal name; otherwise the caller must supply one.
    if [[ -z "$internal_name" ]]; then
        internal_name="${MAP_NAMES[$display_name]:-}"
        [[ -z "$internal_name" ]] && {
            log_err "'$display_name' is not a known map. Pass --internal <InternalName> (e.g. MyMap_WP)."
            return 1
        }
    fi
    if [[ ! "$internal_name" =~ ^[A-Za-z0-9_]+$ ]]; then
        log_err "Invalid internal map name '$internal_name' — only letters, digits and _ are allowed."
        return 1
    fi

    # Defaults from server-defaults.conf, then auto-increment past existing maps.
    game_port="${game_port:-$(read_conf_value "$DEFAULTS_CONF" "DefaultGamePort" "7777")}"
    query_port="${query_port:-$(read_conf_value "$DEFAULTS_CONF" "DefaultQueryPort" "27015")}"
    rcon_port="${rcon_port:-$(read_conf_value "$DEFAULTS_CONF" "DefaultRCONPort" "27020")}"
    max_players="${max_players:-$(read_conf_value "$DEFAULTS_CONF" "DefaultMaxPlayers" "70")}"

    local existing_maps; existing_maps=( $(get_maps) )
    if (( ${#existing_maps[@]} > 0 )); then
        local max_g=0 max_q=0 max_r=0 gp qp rp
        for em in "${existing_maps[@]}"; do
            gp=$(read_conf_value "$MAPS_DIR/$em/map.conf" "GamePort" "0")
            qp=$(read_conf_value "$MAPS_DIR/$em/map.conf" "QueryPort" "0")
            rp=$(read_conf_value "$MAPS_DIR/$em/map.conf" "RCONPort" "0")
            (( gp > max_g )) && max_g=$gp
            (( qp > max_q )) && max_q=$qp
            (( rp > max_r )) && max_r=$rp
        done
        game_port=$(( max_g + 2 )); query_port=$(( max_q + 1 )); rcon_port=$(( max_r + 1 ))
    fi

    # Never leave RCON unauthenticated in an automated run.
    if [[ -z "$admin_pw" ]]; then
        admin_pw=$(read_conf_value "$DEFAULTS_CONF" "DefaultAdminPassword" "")
    fi
    if [[ -z "$admin_pw" ]]; then
        admin_pw=$(tr -dc 'A-Za-z0-9' </dev/urandom | head -c 20)
        log_warn "No admin password given — generated one. It is stored in $MAPS_DIR/$display_name/map.conf (mode 600)."
    fi

    mkdir -p "$MAPS_DIR/$display_name/backups"
    cat > "$MAPS_DIR/$display_name/map.conf" <<EOF
MapName=$internal_name
DisplayName=$display_name
SaveDir=$display_name
GamePort=$game_port
QueryPort=$query_port
RCONPort=$rcon_port
MaxPlayers=$max_players
AdminPassword=$admin_pw
ServerPassword=$server_pw
CustomStartParams=$(read_conf_value "$DEFAULTS_CONF" "DefaultStartParams" "-NoBattlEye -crossplay -NoHangDetection")
ClusterID=$(read_conf_value "$DEFAULTS_CONF" "ClusterID" "")
BatchEnabled=true
EOF
    chmod 600 "$MAPS_DIR/$display_name/map.conf" 2>/dev/null || true

    echo "# Mod configuration for $display_name"  > "$MAPS_DIR/$display_name/mods.conf"
    echo "# Format: id|name|enabled"             >> "$MAPS_DIR/$display_name/mods.conf"

    create_optimized_game_settings "$MAPS_DIR/$display_name" "high"
    [[ ! -f "$MAPS_DIR/$display_name/Game.ini" ]] && touch "$MAPS_DIR/$display_name/Game.ini"
    echo "${display_name}|${internal_name}|enabled" >> "$MAPS_CONF"
    create_systemd_service "$display_name"
    open_firewall_ports "$game_port" "$query_port" "$rcon_port"

    log_ok "Map '$display_name' created — game $game_port/udp, query $query_port/udp, rcon $rcon_port (localhost)."
}

