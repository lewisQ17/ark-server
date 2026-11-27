#@lib shellcheck shell=bash
#@lib Install/update van SteamCMD, Proton en de ASA-serverbestanden — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  INSTALL / UPDATE
#═══════════════════════════════════════════════════════════════════════════════

install_server() {
    clear
    log_info "Installing / Updating ARK server..."
    check_cpu_flags || return 1

    local running=0
    for map_dir in "$MAPS_DIR"/*/; do
        [[ -d "$map_dir" ]] || continue
        local mname; mname=$(basename "$map_dir")
        if is_map_running "$mname"; then
            log_err "Map '${BLD}$mname${R}' is running. Stop all maps first."
            ((running++))
        fi
    done
    (( running > 0 )) && return 1

    mkdir -p "$STEAMCMD_DIR" "$PROTON_DIR" "$SERVER_DIR"

    if [[ ! -f "$STEAMCMD_DIR/steamcmd.sh" ]]; then
        log_info "Downloading SteamCMD..."
        wget -q -O "$STEAMCMD_DIR/steamcmd_linux.tar.gz" "$STEAMCMD_URL"
        tar -xzf "$STEAMCMD_DIR/steamcmd_linux.tar.gz" -C "$STEAMCMD_DIR"
        rm -f "$STEAMCMD_DIR/steamcmd_linux.tar.gz"
        log_ok "SteamCMD installed."
    else
        log_ok "SteamCMD already present."
    fi

    mkdir -p "$HOME/.steam/sdk32" "$HOME/.steam/sdk64"
    ln -sf "$STEAMCMD_DIR/linux32/steamclient.so" "$HOME/.steam/sdk32/steamclient.so"
    ln -sf "$STEAMCMD_DIR/linux64/steamclient.so" "$HOME/.steam/sdk64/steamclient.so"

    if [[ ! -d "$PROTON_DIR/files" ]]; then
        log_info "Downloading ${BLD}$PROTON_VERSION${R}..."
        wget -q --show-progress -O "$PROTON_DIR/$PROTON_VERSION.tar.gz" "$PROTON_URL"
        tar -xzf "$PROTON_DIR/$PROTON_VERSION.tar.gz" -C "$PROTON_DIR" --strip-components=1
        rm -f "$PROTON_DIR/$PROTON_VERSION.tar.gz"
        log_ok "Proton installed."
    else
        log_ok "Proton already present."
    fi

    log_info "Downloading / Updating ARK (AppID ${BLD}$ARK_APPID${R})..."
    # De exit-status van SteamCMD werd genegeerd: na een mislukte of halve download
    # meldde install.sh alsnog "Server files installed". Nu controleren we de status
    # (en of de server-binary er echt staat) en falen we hard.
    local steam_rc=0
    "$STEAMCMD_DIR/steamcmd.sh" \
        +force_install_dir "$SERVER_DIR" \
        +login anonymous \
        +@sSteamCmdForcePlatformType windows \
        +app_update $ARK_APPID validate \
        +quit || steam_rc=$?
    if (( steam_rc != 0 )); then
        log_err "SteamCMD exited with status ${BLD}$steam_rc${R} — the server files are incomplete."
        log_err "Fix the cause (disk space / network) and run install again."
        return 1
    fi
    if [[ ! -f "$SERVER_DIR/ShooterGame/Binaries/Win64/ArkAscendedServer.exe" ]]; then
        log_err "SteamCMD reported success but ArkAscendedServer.exe is missing — install incomplete."
        return 1
    fi

    local pdb_count
    pdb_count=$(find "$SERVER_DIR" -name "*.pdb" 2>/dev/null | wc -l)
    (( pdb_count > 0 )) && {
        log_info "Removing ${BLD}$pdb_count${R} .pdb debug files..."
        find "$SERVER_DIR" -name "*.pdb" -delete
    }

    local compat_dir="$SERVER_DIR/steamapps/compatdata/$ARK_APPID"
    if [[ ! -d "$compat_dir/pfx" ]]; then
        log_info "Initializing Proton prefix..."
        mkdir -p "$compat_dir"
        cp -r "$PROTON_DIR/files/share/default_pfx/." "$compat_dir/"
    fi

    apply_optimizations
    mkdir -p "$HOME/.config/protonfixes"

    log_ok "Server installation/update complete!"
    echo ""
    echo "  ${GRN}Next step:${R} Use ${BLD}'Add Map'${R} to create your first map."
    press_enter
}

