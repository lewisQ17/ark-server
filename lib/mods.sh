#@lib shellcheck shell=bash
#@lib Mod-beheer (mods.conf per map) — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  MOD MANAGEMENT (mods.conf per map, with names + toggle)
#═══════════════════════════════════════════════════════════════════════════════

fetch_mod_name() {
    local mod_id="$1"
    local name
    name=$(curl -sf --max-time 5 -X POST \
        "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/" \
        -d "itemcount=1&publishedfileids[0]=$mod_id" 2>/dev/null | \
        python3 -c "import sys,json; d=json.load(sys.stdin); print(d['response']['publishedfiledetails'][0].get('title',''))" 2>/dev/null)
    echo "${name:-}"
}

init_mods_conf() {
    local map="$1"
    local mods_file="$MAPS_DIR/$map/mods.conf"
    [[ -f "$mods_file" ]] && return

    echo "# Mod configuration for $map" > "$mods_file"
    echo "# Format: id|name|enabled" >> "$mods_file"

    local old_ids
    old_ids=$(read_conf_value "$MAPS_DIR/$map/map.conf" "ModIDs" "")
    if [[ -n "$old_ids" ]]; then
        IFS=',' read -ra ids <<< "$old_ids"
        for id in "${ids[@]}"; do
            id=$(echo "$id" | tr -d ' ')
            [[ -z "$id" ]] && continue
            local name; name=$(fetch_mod_name "$id")
            echo "${id}|${name}|true" >> "$mods_file"
        done
    fi
}

get_enabled_mod_ids() {
    local map="$1"
    local mods_file="$MAPS_DIR/$map/mods.conf"
    if [[ ! -f "$mods_file" ]]; then
        read_conf_value "$MAPS_DIR/$map/map.conf" "ModIDs" ""
        return
    fi
    local ids=()
    while IFS='|' read -r id name enabled; do
        [[ "$id" =~ ^#.*$ || -z "$id" ]] && continue
        [[ "$enabled" == "true" ]] && ids+=("$id")
    done < "$mods_file"
    local IFS=','
    echo "${ids[*]}"
}

manage_mods() {
    local map="$1"
    init_mods_conf "$map"
    local mods_file="$MAPS_DIR/$map/mods.conf"

    while true; do
        clear
        echo ""
        echo "  ${BLD}${MAG}═══ Mod Management — ${CYN}$map${MAG} ═══${R}"
        thin_sep

        local mod_ids=() mod_names=() mod_enabled=() mod_count=0
        while IFS='|' read -r id name enabled; do
            [[ "$id" =~ ^#.*$ || -z "$id" ]] && continue
            mod_ids+=("$id")
            mod_names+=("${name:-???}")
            mod_enabled+=("$enabled")
            ((mod_count++))
        done < "$mods_file"

        if (( mod_count > 0 )); then
            echo ""
            printf "  ${BLD}${WHT}  %-4s %-12s %-30s %s${R}\n" "#" "ID" "Name" "Status"
            thin_sep
            for idx in "${!mod_ids[@]}"; do
                local stat_icon
                if [[ "${mod_enabled[$idx]}" == "true" ]]; then
                    stat_icon="${GRN}${BLD}✔ ON${R}"
                else
                    stat_icon="${RED}✖ OFF${R}"
                fi
                printf "  ${CYN}%3d${R}) %-12s %-30s %s\n" "$((idx+1))" "${mod_ids[$idx]}" "${mod_names[$idx]}" "$stat_icon"
            done
        else
            echo ""
            echo "  ${DIM}No mods installed.${R}"
        fi

        echo ""
        echo "  ${CYN}a${R}) Add mod(s)"
        echo "  ${CYN}r${R}) Remove mod"
        echo "  ${CYN}t${R}) Toggle mod on/off"
        echo "  ${CYN}n${R}) Rename mod"
        echo "  ${CYN}f${R}) Fetch all names from Steam"
        echo ""
        echo "  ${DIM}0) Back${R}"
        echo ""
        echo -n "  ${WHT}Choice: ${R}"
        read -r choice

        case "$choice" in
            a|A)
                echo -n "  ${WHT}Mod ID(s) to add (comma-separated): ${R}"
                read -r new_ids
                [[ -z "$new_ids" ]] && continue
                IFS=',' read -ra id_arr <<< "$new_ids"
                for id in "${id_arr[@]}"; do
                    id=$(echo "$id" | tr -d ' ')
                    [[ -z "$id" ]] && continue
                    if grep -q "^${id}|" "$mods_file" 2>/dev/null; then
                        log_warn "Mod $id already exists, skipping."
                        continue
                    fi
                    log_info "Fetching name for mod ${BLD}$id${R}..."
                    local name; name=$(fetch_mod_name "$id")
                    echo "${id}|${name}|true" >> "$mods_file"
                    if [[ -n "$name" ]]; then
                        log_ok "Added: ${BLD}$id${R} — ${CYN}$name${R}"
                    else
                        log_ok "Added: ${BLD}$id${R} ${DIM}(no name found)${R}"
                    fi
                done
                sleep 1
                ;;
            r|R)
                (( mod_count == 0 )) && continue
                echo -n "  ${WHT}Mod # to remove: ${R}"
                read -r num
                if (( num > 0 && num <= mod_count )) 2>/dev/null; then
                    local del_id="${mod_ids[$((num-1))]}"
                    sed -i "/^${del_id}|/d" "$mods_file"
                    log_ok "Removed mod ${BLD}$del_id${R}"
                    sleep 1
                fi
                ;;
            t|T)
                (( mod_count == 0 )) && continue
                echo -n "  ${WHT}Mod # to toggle: ${R}"
                read -r num
                if (( num > 0 && num <= mod_count )) 2>/dev/null; then
                    local tog_id="${mod_ids[$((num-1))]}"
                    local cur_state="${mod_enabled[$((num-1))]}"
                    if [[ "$cur_state" == "true" ]]; then
                        sed -i "s/^${tog_id}|\(.*\)|true$/${tog_id}|\1|false/" "$mods_file"
                        log_ok "Mod ${BLD}$tog_id${R} → ${RED}OFF${R}"
                    else
                        sed -i "s/^${tog_id}|\(.*\)|false$/${tog_id}|\1|true/" "$mods_file"
                        log_ok "Mod ${BLD}$tog_id${R} → ${GRN}ON${R}"
                    fi
                    sleep 1
                fi
                ;;
            n|N)
                (( mod_count == 0 )) && continue
                echo -n "  ${WHT}Mod # to rename: ${R}"
                read -r num
                if (( num > 0 && num <= mod_count )) 2>/dev/null; then
                    local ren_id="${mod_ids[$((num-1))]}"
                    local ren_en="${mod_enabled[$((num-1))]}"
                    echo -n "  ${WHT}New name: ${R}"
                    read -r new_name
                    [[ -z "$new_name" ]] && continue
                    sed -i "s/^${ren_id}|.*$/${ren_id}|${new_name}|${ren_en}/" "$mods_file"
                    log_ok "Renamed mod ${BLD}$ren_id${R} → ${CYN}$new_name${R}"
                    sleep 1
                fi
                ;;
            f|F)
                (( mod_count == 0 )) && continue
                log_info "Fetching mod names from Steam Workshop..."
                for idx in "${!mod_ids[@]}"; do
                    local fid="${mod_ids[$idx]}"
                    local fen="${mod_enabled[$idx]}"
                    local fname; fname=$(fetch_mod_name "$fid")
                    if [[ -n "$fname" ]]; then
                        sed -i "s/^${fid}|.*$/${fid}|${fname}|${fen}/" "$mods_file"
                        log_ok "${BLD}$fid${R} → ${CYN}$fname${R}"
                    else
                        log_warn "${BLD}$fid${R} — no name found"
                    fi
                done
                sleep 1
                ;;
            0|"") return ;;
            *) ;;
        esac
    done
}

