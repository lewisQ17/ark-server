#@lib shellcheck shell=bash
#@lib Hulpfuncties (log, pad, confirm) en dependency-check — wordt door ark-manager.sh ge-sourced; enkel functies.
#═══════════════════════════════════════════════════════════════════════════════
#  UTILITY FUNCTIONS
#═══════════════════════════════════════════════════════════════════════════════

log_info()  { echo "$(date '+%Y-%m-%dT%H:%M:%S') ${CYN}${BLD}[INFO]${R} $*"; }
log_ok()    { echo "$(date '+%Y-%m-%dT%H:%M:%S') ${GRN}${BLD}[OK]${R} $*"; }
log_warn()  { echo "$(date '+%Y-%m-%dT%H:%M:%S') ${YEL}${BLD}[WARN]${R} $*"; }
log_err()   { echo "$(date '+%Y-%m-%dT%H:%M:%S') ${RED}${BLD}[ERROR]${R} $*"; }

separator() { echo "  ${GRY}──────────────────────────────────────────────────────────────────────────────${R}${CL}"; }
thin_sep()  { echo "  ${GRY}╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌${R}${CL}"; }

# Pad text to exact visible display width (handles multi-byte chars correctly)
pad() {
    local w="$1" t="${2:0:$1}"
    printf "%s" "$t"
    local n=$(( w - ${#t} ))
    (( n > 0 )) && printf "%*s" "$n" ""
}

confirm() {
    local msg="${1:-Continue?}"
    echo -n "  ${YEL}${msg} [y/N]: ${R}"
    read -r ans
    [[ "$ans" =~ ^[Yy] ]]
}

press_enter() {
    echo ""
    echo -n "  ${DIM}Press Enter to continue...${R}"
    read -r
}

#═══════════════════════════════════════════════════════════════════════════════
#  DEPENDENCY CHECK
#═══════════════════════════════════════════════════════════════════════════════

check_dependencies() {
    local missing=()
    local deps=(wget tar grep python3 curl cron)
    if command -v apt-get &>/dev/null; then
        for pkg in libc6:i386 libstdc++6:i386 libncursesw6:i386 libfreetype6:i386 libfreetype6:amd64; do
            dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "install ok installed" || missing+=("$pkg")
        done
    fi
    for cmd in "${deps[@]}"; do
        if [[ "$cmd" == "cron" ]]; then
            command -v crontab &>/dev/null || missing+=("cron")
        else
            command -v "$cmd" &>/dev/null || missing+=("$cmd")
        fi
    done
    if (( ${#missing[@]} > 0 )); then
        log_warn "Missing packages: ${missing[*]}"
        if confirm "Install them now? (requires sudo)"; then
            if command -v apt-get &>/dev/null; then
                "${SUDO[@]}" dpkg --add-architecture i386 2>/dev/null || true
                "${SUDO[@]}" apt-get update -qq
                "${SUDO[@]}" apt-get install -y "${missing[@]}"
            fi
            log_ok "Dependencies installed."
        fi
    fi
}

check_cpu_flags() {
    if ! grep -qw avx /proc/cpuinfo || ! grep -qw avx2 /proc/cpuinfo; then
        log_err "CPU missing AVX/AVX2 support. Set CPU type to 'host' in your hypervisor."
        return 1
    fi
    return 0
}

