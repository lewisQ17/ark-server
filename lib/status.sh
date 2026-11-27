#@lib shellcheck shell=bash
#@lib Map-status: draait, PID, spelers, health, uptime, RAM/CPU, queryable — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  MAP STATUS HELPERS
#═══════════════════════════════════════════════════════════════════════════════

get_maps() {
    local maps=()
    for d in "$MAPS_DIR"/*/; do
        [[ -d "$d" ]] || continue
        maps+=("$(basename "$d")")
    done
    echo "${maps[@]}"
}

# Het patroon was niet geankerd: 'AltSaveDirectoryName=Ragnarok' matchte ook het
# proces van 'Ragnarok2', waardoor twee maps als dezelfde werden gezien. Nu moet de
# mapnaam het einde van dat argument zijn: gevolgd door een spatie (het volgende
# argument) of het einde van de commandline.
is_map_running() {
    local map="$1"
    local save_dir
    save_dir=$(read_conf_value "$MAPS_DIR/$map/map.conf" "SaveDir" "$map")
    pgrep -f "ArkAscendedServer\.exe.*AltSaveDirectoryName=${save_dir}( |\$)" &>/dev/null
}

get_map_pid() {
    local map="$1"
    local save_dir
    save_dir=$(read_conf_value "$MAPS_DIR/$map/map.conf" "SaveDir" "$map")
    pgrep -f "ArkAscendedServer\.exe.*AltSaveDirectoryName=${save_dir}( |\$)" 2>/dev/null | head -1
}

get_player_count() {
    local map="$1"
    is_map_running "$map" || { echo "0"; return; }
    # Cache for 30 seconds (RCON call is slow)
    # Het cachepad was voorspelbaar (/tmp/.ark-players-<map>): elke lokale gebruiker kon
    # het vooraf aanmaken of als symlink neerzetten, en zo de getoonde waarde bepalen of
    # onze schrijfactie naar een ander bestand omleiden. Nu een private dir per uid met
    # eigenaarscontrole; is die niet van ons, dan cachen we niet (/dev/null).
    local cache_dir cache_file="/dev/null"
    cache_dir="${TMPDIR:-/tmp}/.ark-cache-$(id -u)"
    if mkdir -p -m 700 "$cache_dir" 2>/dev/null && [[ -d "$cache_dir" && ! -L "$cache_dir" && -O "$cache_dir" ]]; then
        chmod 700 "$cache_dir" 2>/dev/null || true
        cache_file="$cache_dir/players-${map}"
        [[ -L "$cache_file" ]] && rm -f "$cache_file"
    fi
    if [[ -f "$cache_file" ]]; then
        local now mtime age
        now=$(date +%s)
        if mtime=$(stat -c %Y "$cache_file" 2>/dev/null); then
            age=$(( now - mtime ))
        else
            age=0
        fi
        if (( age > 0 && age < 30 )); then
            cat "$cache_file"
            return
        fi
    fi
    local rcon_port admin_pw
    rcon_port=$(read_conf_value "$MAPS_DIR/$map/map.conf" "RCONPort" "27020")
    admin_pw=$(read_conf_value "$MAPS_DIR/$map/map.conf" "AdminPassword" "")
    [[ -z "$admin_pw" ]] && admin_pw=$(read_conf_value "$DEFAULTS_CONF" "DefaultAdminPassword" "")
    local count
    count=$(timeout 5 python3 "$RCON_SCRIPT" "localhost:$rcon_port" -p "$admin_pw" -c "ListPlayers" 2>/dev/null | grep -c "^[0-9]" || true)
    count="${count:-0}"
    echo "$count" | tee "$cache_file"
}

check_map_health() {
    local map="$1"
    is_map_running "$map" || { echo "STOPPED"; return; }
    local pid; pid=$(get_map_pid "$map")
    [[ -z "$pid" ]] && { echo "CRASHED"; return; }
    local state; state=$(ps -o state= -p "$pid" 2>/dev/null || echo "?")
    case "$state" in
        Z*) echo "ZOMBIE" ;;
        T*) echo "FROZEN" ;;
        *)
            local game_port
            game_port=$(read_conf_value "$MAPS_DIR/$map/map.conf" "GamePort" "7777")
            if ss -lunp 2>/dev/null | grep -q ":${game_port} "; then echo "HEALTHY"; else echo "DEGRADED"; fi
            ;;
    esac
}

get_map_uptime() {
    local map="$1"
    local pid; pid=$(get_map_pid "$map")
    [[ -z "$pid" ]] && { echo "-"; return; }
    local elapsed; elapsed=$(ps -o etimes= -p "$pid" 2>/dev/null | tr -d ' ')
    [[ -z "$elapsed" ]] && { echo "-"; return; }
    local d=$(( elapsed / 86400 )) h=$(( (elapsed % 86400) / 3600 )) m=$(( (elapsed % 3600) / 60 ))
    if (( d > 0 )); then echo "${d}d ${h}h"
    elif (( h > 0 )); then echo "${h}h ${m}m"
    else echo "${m}m"
    fi
}

# RAM per map — uses systemd cgroup (accurate) or fallback to process tree
get_map_memory() {
    local map="$1"
    is_map_running "$map" || { echo "-"; return; }

    # Method 1: systemd cgroup (all processes in service)
    local service="ark-${map,,}.service"
    local bytes
    bytes=$(systemctl show "$service" --property=MemoryCurrent 2>/dev/null | cut -d= -f2)
    if [[ "$bytes" =~ ^[0-9]+$ ]] && (( bytes > 1048576 )); then
        local mb=$(( bytes / 1048576 ))
        if (( mb > 1024 )); then
            printf "%.1fG" "$(awk "BEGIN{printf \"%.1f\", $mb/1024}")"
        else
            echo "${mb}M"
        fi
        return
    fi

    # Method 2: Sum RSS of all matching processes
    local save_dir; save_dir=$(read_conf_value "$MAPS_DIR/$map/map.conf" "SaveDir" "$map")
    local total_kb=0
    while IFS= read -r pid; do
        local rss; rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')
        (( total_kb += ${rss:-0} ))
    done < <(pgrep -f "AltSaveDirectoryName=${save_dir}( |\$)" 2>/dev/null)
    # Ankering (spatie of einde commandline): zonder anker telde 'Ragnarok' ook het
    # RAM van 'Ragnarok2' mee.
    (( total_kb == 0 )) && { echo "-"; return; }
    local mb=$(( total_kb / 1024 ))
    if (( mb > 1024 )); then
        printf "%.1fG" "$(awk "BEGIN{printf \"%.1f\", $mb/1024}")"
    else
        echo "${mb}M"
    fi
}

# CPU per map — sum %cpu of all related processes
get_map_cpu() {
    local map="$1"
    is_map_running "$map" || { echo "-"; return; }

    # Method 1: systemd cgroup PIDs
    local service="ark-${map,,}.service"
    local cgroup
    cgroup=$(systemctl show "$service" --property=ControlGroup 2>/dev/null | cut -d= -f2)
    if [[ -n "$cgroup" && -f "/sys/fs/cgroup${cgroup}/cgroup.procs" ]]; then
        local total
        total=$(cat "/sys/fs/cgroup${cgroup}/cgroup.procs" 2>/dev/null | \
            xargs -I{} ps -o %cpu= -p {} 2>/dev/null | \
            awk '{s+=$1}END{printf "%.0f", s}')
        echo "${total:-0}%"
        return
    fi

    # Method 2: pgrep
    local save_dir; save_dir=$(read_conf_value "$MAPS_DIR/$map/map.conf" "SaveDir" "$map")
    local total
    # Ankering (spatie of einde commandline): zonder anker telde 'Ragnarok' ook de CPU
    # van 'Ragnarok2' mee.
    total=$(pgrep -f "AltSaveDirectoryName=${save_dir}( |\$)" 2>/dev/null | \
        xargs -I{} ps -o %cpu= -p {} 2>/dev/null | \
        awk '{s+=$1}END{printf "%.0f", s}' 2>/dev/null)
    echo "${total:-0}%"
}

# Fast system CPU usage — delta from last sample (instant, no top -bn1)
get_system_cpu() {
    local cur prev
    cur=$(awk '/^cpu /{printf "%d %d", $2+$3+$4, $2+$3+$4+$5+$6+$7+$8}' /proc/stat 2>/dev/null) || { echo "?"; return; }
    # /tmp/.ark-sys-cpu was een vast, voorspelbaar pad: elke lokale gebruiker kon daar een
    # symlink neerzetten en zo deze (soms als root draaiende) schrijfactie naar een
    # willekeurig bestand laten gaan. Nu een private dir per uid met eigenaarscontrole;
    # lukt dat niet, dan houden we geen state bij (/dev/null).
    local state_dir state_file="/dev/null"
    state_dir="${TMPDIR:-/tmp}/.ark-cache-$(id -u)"
    if mkdir -p -m 700 "$state_dir" 2>/dev/null && [[ -d "$state_dir" && ! -L "$state_dir" && -O "$state_dir" ]]; then
        chmod 700 "$state_dir" 2>/dev/null || true
        state_file="$state_dir/sys-cpu"
        [[ -L "$state_file" ]] && rm -f "$state_file"
    fi
    prev=$(cat "$state_file" 2>/dev/null) || prev="$cur"
    [[ -z "$prev" ]] && prev="$cur"
    echo "$cur" > "$state_file"
    echo "$prev $cur" | awk '{db=$3-$1; dt=$4-$2; if(dt>0) printf "%.0f", 100*db/dt; else print "0"}'
}

# Check if server is externally joinable (real network test, cached 15s)
check_server_queryable() {
    local map="$1"
    is_map_running "$map" || { echo "-"; return; }

    # Cache result for 15 seconds
    # Zelfde probleem als bij de player-cache: /tmp/.ark-join-<map> was voorspelbaar en
    # door iedere lokale gebruiker vooraf te vullen (of te symlinken). Nu een private dir
    # per uid met eigenaarscontrole; anders niet cachen (/dev/null slikt de tee).
    local cache_dir cache_file="/dev/null"
    cache_dir="${TMPDIR:-/tmp}/.ark-cache-$(id -u)"
    if mkdir -p -m 700 "$cache_dir" 2>/dev/null && [[ -d "$cache_dir" && ! -L "$cache_dir" && -O "$cache_dir" ]]; then
        chmod 700 "$cache_dir" 2>/dev/null || true
        cache_file="$cache_dir/join-${map}"
        [[ -L "$cache_file" ]] && rm -f "$cache_file"
    fi
    if [[ -f "$cache_file" ]]; then
        local now mtime age
        now=$(date +%s)
        if mtime=$(stat -c %Y "$cache_file" 2>/dev/null); then
            age=$(( now - mtime ))
        else
            age=0
        fi
        if (( age > 0 && age < 15 )); then
            cat "$cache_file"
            return
        fi
    fi

    local game_port query_port rcon_port
    game_port=$(read_conf_value "$MAPS_DIR/$map/map.conf" "GamePort" "7777")
    query_port=$(read_conf_value "$MAPS_DIR/$map/map.conf" "QueryPort" "27015")
    rcon_port=$(read_conf_value "$MAPS_DIR/$map/map.conf" "RCONPort" "27020")

    local server_ip
    server_ip=$(hostname -I 2>/dev/null | awk '{print $1}')
    [[ -z "$server_ip" ]] && { echo "?" | tee "$cache_file"; return; }

    local result="YES"

    # 1) Ports must be listening; fall back gracefully if 'ss' is unavailable
    if command -v ss &>/dev/null; then
        ss -lunp 2>/dev/null | grep -q ":${game_port} " || result="NO"
        [[ "$result" == "YES" ]] && ss -lunp 2>/dev/null | grep -q ":${query_port} " || result="NO"
        [[ "$result" == "YES" ]] && ss -tlnp 2>/dev/null | grep -q ":${rcon_port} " || result="NO"
    else
        echo "?" | tee "$cache_file"
        return
    fi

    # 2) Firewall must allow game port
    if [[ "$result" == "YES" ]] && command -v ufw &>/dev/null; then
        local ufw_out
        ufw_out=$("${SUDO_N[@]}" ufw status 2>/dev/null || echo "")
        if echo "$ufw_out" | grep -q "Status: active"; then
            echo "$ufw_out" | grep -qE "${game_port}" || result="NO"
        fi
    fi

    # 3) TCP connect to RCON on server's network IP (proves real reachability)
    if [[ "$result" == "YES" ]]; then
        timeout 1 bash -c "echo >/dev/tcp/${server_ip}/${rcon_port}" 2>/dev/null || result="NO"
    fi

    echo "$result" | tee "$cache_file"
}

# Check if map is included in groep start/stop
is_batch_enabled() {
    local map="$1"
    local val
    val=$(read_conf_value "$MAPS_DIR/$map/map.conf" "BatchEnabled" "true")
    [[ "$val" == "true" ]]
}

# Check if systemd auto-start is enabled
is_autostart_enabled() {
    local map="$1"
    local service="ark-${map,,}.service"
    systemctl is-enabled "$service" 2>/dev/null | grep -q "enabled"
}

