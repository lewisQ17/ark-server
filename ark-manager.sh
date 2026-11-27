#!/usr/bin/env bash
#═══════════════════════════════════════════════════════════════════════════════
#  ARK: Survival Ascended — Linux Server Manager  v2.3
#  A clean, map-centered management tool for ASA dedicated servers.
#  https://github.com/lewisQ17/ARK-Server
#═══════════════════════════════════════════════════════════════════════════════
set -u

VERSION="2.3"
export LC_ALL=C.UTF-8 LANG=C.UTF-8

#───────────────────────────── Paths ──────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$(realpath "$0")")" && pwd)"

# Privileged helpers: when we are already root (install.sh runs us that way) call
# things directly; otherwise go through sudo exactly as before. Arrays so that the
# empty case doesn't turn into an empty argument.
if [[ ${EUID:-$(id -u)} -eq 0 ]]; then SUDO=(); SUDO_N=(); else SUDO=(sudo); SUDO_N=(sudo -n); fi

# Which user the generated systemd units run the game server as. install.sh sets this
# so the units never end up with User=root just because the installer was root.
ARK_SERVICE_USER="${ARK_SERVICE_USER:-$(whoami)}"
CONFIG_DIR="$SCRIPT_DIR/config"
MAPS_DIR="$SCRIPT_DIR/maps"
MAPS_CONF="$CONFIG_DIR/maps.conf"
DEFAULTS_CONF="$CONFIG_DIR/server-defaults.conf"
OPT_CONF="$CONFIG_DIR/optimization.conf"
RCON_SCRIPT="$SCRIPT_DIR/rcon.py"

SERVER_DIR="$SCRIPT_DIR/server-files"
STEAMCMD_DIR="$SCRIPT_DIR/steamcmd"
PROTON_VERSION="GE-Proton10-4"
PROTON_DIR="$SCRIPT_DIR/$PROTON_VERSION"

STEAMCMD_URL="https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz"
PROTON_URL="https://github.com/GloriousEggroll/proton-ge-custom/releases/download/$PROTON_VERSION/$PROTON_VERSION.tar.gz"
ARK_APPID=2430930

#───────────────────────────── Colors ($'...' = real ANSI bytes) ──────────────
R=$'\e[0m'
BLD=$'\e[1m'
DIM=$'\e[2m'
RED=$'\e[31m'
GRN=$'\e[32m'
YEL=$'\e[33m'
BLU=$'\e[34m'
MAG=$'\e[35m'
CYN=$'\e[36m'
WHT=$'\e[97m'
GRY=$'\e[90m'
BG_GRN=$'\e[42m'
BG_RED=$'\e[41m'
BG_YEL=$'\e[43m'
BG_BLU=$'\e[44m'
CL=$'\e[K'

#───────────────────────────── Known Maps ─────────────────────────────────────
declare -A MAP_NAMES=(
    ["TheIsland"]="TheIsland_WP"
    ["ScorchedEarth"]="ScorchedEarth_WP"
    ["Aberration"]="Aberration_WP"
    ["Extinction"]="Extinction_WP"
    ["TheCenter"]="TheCenter_WP"
    ["Ragnarok"]="Ragnarok_WP"
    ["Valguero"]="Valguero_WP"
    ["CrystalIsles"]="CrystalIsles_WP"
    ["LostIsland"]="LostIsland_WP"
    ["Fjordur"]="Fjordur_WP"
    ["GenesisPart1"]="Genesis_WP"
    ["GenesisPart2"]="Gen2_WP"
)

# Global feedback variable — shown once on next dashboard refresh
FEEDBACK=""

# Restore terminal state on exit (cursor visibility, colors)
cleanup_terminal() { printf '\e[?25h\e[0m' 2>/dev/null; stty echo 2>/dev/null || true; }
trap cleanup_terminal EXIT

#───────────────────────────── Config-helpers ─────────────────────────────────
# Blijven bewust in dit bestand: rust/ark-ctl/tests/vectors.rs snijdt deze twee
# functies uit ark-manager.sh om ze tegen de Rust-versie te vergelijken.
read_conf_value() {
    local file="$1" key="$2" default="${3:-}"
    local val
    val=$(grep -E "^${key}=" "$file" 2>/dev/null | tail -1 | cut -d'=' -f2-)
    echo "${val:-$default}"
}

write_conf_value() {
    local file="$1" key="$2" value="$3"
    # De waarde ging ongeescaped als sed-vervangtekst mee: een '|', '&' of '\' in een
    # wachtwoord of pad veranderde de sed-expressie of injecteerde tekst in het bestand.
    # Nu escapen we patroon en vervangtekst apart, en weigeren we waarden met een
    # newline (die passen niet in een key=value-regel).
    if [[ "$value" == *$'\n'* ]]; then
        log_err "Value for '$key' contains a newline — refusing to write it."
        return 1
    fi
    local key_re key_repl val_repl
    key_re=$(printf '%s' "$key" | sed -e 's/[][\\.^$*|/]/\\&/g')
    key_repl=$(printf '%s' "$key" | sed -e 's/[\\&|]/\\&/g')
    val_repl=$(printf '%s' "$value" | sed -e 's/[\\&|]/\\&/g')
    if grep -qE "^${key}=" "$file" 2>/dev/null; then
        sed -i "s|^${key_re}=.*|${key_repl}=${val_repl}|" "$file"
    else
        echo "${key}=${value}" >> "$file"
    fi
}

#───────────────────────────── Modules ────────────────────────────────────────
# De overige functies staan in lib/*.sh (alleen definities; niets draait bij het sourcen).
LIB_DIR="$SCRIPT_DIR/lib"
for _lib in core install status dashboard add_map game_settings system mods \
            lifecycle rcon map_admin ops menus cli; do
    if [[ ! -f "$LIB_DIR/$_lib.sh" ]]; then
        echo "ark-manager: ontbrekende module $LIB_DIR/$_lib.sh" >&2
        exit 1
    fi
    # shellcheck source=/dev/null
    source "$LIB_DIR/$_lib.sh"
done
unset _lib

#═══════════════════════════════════════════════════════════════════════════════
#  MAIN ENTRY POINT
#═══════════════════════════════════════════════════════════════════════════════

mkdir -p "$CONFIG_DIR" "$MAPS_DIR"

[[ -f "$DEFAULTS_CONF" ]] || cat > "$DEFAULTS_CONF" <<'EOF'
DefaultMaxPlayers=70
DefaultServerPassword=
DefaultAdminPassword=
DefaultGamePort=7777
DefaultQueryPort=27015
DefaultRCONPort=27020
DefaultStartParams=-NoBattlEye -crossplay -NoHangDetection
XPMultiplier=1.0
TamingSpeedMultiplier=1.0
HarvestAmountMultiplier=1.0
ClusterID=
EOF

[[ -f "$MAPS_CONF" ]] || touch "$MAPS_CONF"

[[ -f "$OPT_CONF" ]] || cat > "$OPT_CONF" <<'EOF'
UseNullRHI=true
ServerNiceLevel=-5
EOF

if [[ $# -eq 0 ]]; then
    check_dependencies
    main_menu
else
    case "$1" in
        install|update)   install_server ;;
        plan-install)     plan_install ;;
        start)            [[ -n "${2:-}" ]] && start_map "$2" || { echo "Usage: $0 start <map>"; exit 1; } ;;
        stop)             [[ -n "${2:-}" ]] && stop_map "$2"  || { echo "Usage: $0 stop <map>"; exit 1; } ;;
        restart)          [[ -n "${2:-}" ]] && restart_map "$2" || { echo "Usage: $0 restart <map>"; exit 1; } ;;
        start-all)        start_all_maps ;;
        stop-all)         stop_all_maps ;;
        add-map)          shift; add_map_cli "$@" ;;
        status)           cli_status ;;
        rcon)             [[ -n "${2:-}" ]] && [[ -n "${3:-}" ]] && send_rcon "$2" "${*:3}" || { echo "Usage: $0 rcon <map> \"cmd\""; exit 1; } ;;
        backup)           [[ -n "${2:-}" ]] && backup_map "$2" || { echo "Usage: $0 backup <map>"; exit 1; } ;;
        broadcast)        [[ -n "${2:-}" ]] && broadcast_message "${*:2}" || { echo "Usage: $0 broadcast \"msg\""; exit 1; } ;;
        help|--help|-h)   show_help ;;
        *)                echo "Unknown command: $1"; show_help; exit 1 ;;
    esac
fi
