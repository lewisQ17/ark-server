#!/usr/bin/env bash
# Rooktests voor ark-manager.sh + lib/*.sh — zonder VM, zonder root, zonder echte services.
#
# Alles draait in een wegwerp-sandbox (kopie van het script) met nep-commando's voor
# sudo/systemctl/ufw/pgrep/ip. Niets raakt /etc, systemd of de firewall.
# Dynamische tests vragen bash >= 4 (het script gebruikt `declare -A` en ${var,,});
# onder oudere bash (macOS /bin/bash 3.2) draaien alleen de statische controles en
# de helpers die op 3.2 werken; de rest meldt SKIP.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PASS=0; FAIL=0; SKIP=0
ok()   { PASS=$((PASS+1)); }
bad()  { FAIL=$((FAIL+1)); echo "FAIL: $1"; }
skip() { SKIP=$((SKIP+1)); echo "SKIP: $1"; }
eq()   { if [[ "$2" == "$3" ]]; then ok; else bad "$1 — verwacht '$3', kreeg '$2'"; fi; }
has()  { if [[ "$2" == *"$3"* ]]; then ok; else bad "$1 — mist '$3' in '${2:0:200}'"; fi; }
BASH4=0; (( BASH_VERSINFO[0] >= 4 )) && BASH4=1

if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    echo "Niet als root draaien: sudo-wrappers zouden dan echte systeemcommando's aanroepen."; exit 2
fi

# ── 1. Statisch ──────────────────────────────────────────────────────────────
FILES=("$ROOT/ark-manager.sh" "$ROOT"/lib/*.sh "$ROOT/install.sh" "$ROOT/healthcheck.sh")
for f in "${FILES[@]}"; do
    if bash -n "$f" 2>/dev/null; then ok; else bad "bash -n $f"; fi
done
if command -v shellcheck >/dev/null 2>&1; then
    # SC1091/SC2034 e.d. zijn bewust stil: de modules delen globals met ark-manager.sh.
    # Alleen waarschuwen: de bestaande code is nooit door shellcheck gehaald; falen mag de rest niet blokkeren.
    if shellcheck -S error -x "$ROOT/ark-manager.sh" "$ROOT"/lib/*.sh >/dev/null 2>&1; then ok; else echo "WARN: shellcheck -S error meldt bevindingen (draai het handmatig voor details)"; fi
else
    skip "shellcheck niet geïnstalleerd"
fi

# Elke functie uit tests/functions.txt staat precies één keer gedefinieerd (main + lib).
nfn=0
while read -r fn; do
    [[ -z "$fn" || "$fn" == \#* ]] && continue
    nfn=$((nfn+1))
    n=$(cat "$ROOT/ark-manager.sh" "$ROOT"/lib/*.sh | grep -cE "^${fn}\(\) *\{")
    eq "functie $fn" "$n" "1"
done < "$ROOT/tests/functions.txt"
eq "aantal functies in functions.txt" "$nfn" "$(cat "$ROOT/ark-manager.sh" "$ROOT"/lib/*.sh | grep -cE '^[a-zA-Z_0-9]+\(\) *\{')"

# lib/*.sh bevat alleen definities: sourcen in een lege omgeving mag niets uitvoeren.
for f in "$ROOT"/lib/*.sh; do
    out=$(bash -c 'set -u; source "$1"' _ "$f" 2>&1) && eq "sourcen $(basename "$f") zonder output" "$out" "" || bad "sourcen $(basename "$f"): $out"
done

# ── 2. Sandbox ───────────────────────────────────────────────────────────────
SB="$(mktemp -d "${TMPDIR:-/tmp}/ark-test.XXXXXX")"
trap 'rm -rf "$SB"' EXIT
mkdir -p "$SB/app" "$SB/mock"
cp -R "$ROOT/ark-manager.sh" "$ROOT/lib" "$ROOT/rcon.py" "$SB/app/"
MOCKLOG="$SB/mock.log"; : > "$MOCKLOG"
for c in sudo systemctl ufw ip crontab logrotate sysctl ss; do
    printf '#!/bin/sh\necho "%s $*" >> "%s"\ncat >/dev/null 2>&1 </dev/null || true\nexit 0\n' "$c" "$MOCKLOG" > "$SB/mock/$c"
    chmod +x "$SB/mock/$c"
done
# pgrep: standaard geen proces; MOCK_PGREP=<pid> simuleert een draaiende map.
printf '#!/bin/sh\n[ -n "${MOCK_PGREP:-}" ] && { echo "$MOCK_PGREP"; exit 0; }\nexit 1\n' > "$SB/mock/pgrep"; chmod +x "$SB/mock/pgrep"
# BSD-sed (macOS) wil `-i ''`; GNU-sed niet. Op macOS een kleine wrapper zodat de tests overal draaien.
if ! sed --version >/dev/null 2>&1; then
    printf '#!/bin/sh\nif [ "$1" = -i ]; then shift; exec /usr/bin/sed -i "" "$@"; fi\nexec /usr/bin/sed "$@"\n' > "$SB/mock/sed"
    chmod +x "$SB/mock/sed"
fi
export PATH="$SB/mock:$PATH" ARK_SERVICE_USER="$(whoami)" HOME="$SB/home" TMPDIR="$SB/tmp"
mkdir -p "$HOME" "$TMPDIR"

# Functies-runner: het script tot (niet inclusief) het hoofdgedeelte, plus `"$@"` om één functie aan te roepen.
sed '/^mkdir -p "\$CONFIG_DIR" "\$MAPS_DIR"/,$d' "$SB/app/ark-manager.sh" > "$SB/app/fn.sh"
if (( BASH4 == 0 )); then
    # bash 3.2 kent `declare -A` niet; MAP_NAMES wordt dan weggelaten (alleen voor de helpers die het niet gebruiken).
    awk '/^declare -A MAP_NAMES=\(/{skip=1} skip&&/^\)/{skip=0;next} !skip' "$SB/app/fn.sh" > "$SB/app/fn3.sh"; mv "$SB/app/fn3.sh" "$SB/app/fn.sh"
fi
printf 'trap - EXIT\n"$@"\n' >> "$SB/app/fn.sh"
if (( BASH4 == 0 )); then
    awk '/^declare -A MAP_NAMES=\(/{skip=1} skip&&/^\)/{skip=0;next} !skip' "$SB/app/ark-manager.sh" > "$SB/app/m3.sh" && mv "$SB/app/m3.sh" "$SB/app/ark-manager.sh"
fi
fn() { bash "$SB/app/fn.sh" "$@" 2>&1; }
mgr() { bash "$SB/app/ark-manager.sh" "$@" 2>&1; }

# ── 3. Hulpfuncties ──────────────────────────────────────────────────────────
C="$SB/t.conf"; printf 'A=1\nB=x=y\nA=3\nE=\n' > "$C"
eq "read_conf_value laatste winnaar" "$(fn read_conf_value "$C" A d)" "3"
eq "read_conf_value waarde met =" "$(fn read_conf_value "$C" B d)" "x=y"
eq "read_conf_value lege waarde -> default" "$(fn read_conf_value "$C" E dflt)" "dflt"
eq "read_conf_value ontbrekend -> default" "$(fn read_conf_value "$C" ZZ dflt)" "dflt"
eq "read_conf_value ontbrekend bestand" "$(fn read_conf_value "$SB/nee.conf" A dflt)" "dflt"
fn write_conf_value "$C" A 'p|a&s\w' >/dev/null
eq "write_conf_value speciale tekens" "$(fn read_conf_value "$C" A d)" 'p|a&s\w'
fn write_conf_value "$C" NEW nieuw >/dev/null
eq "write_conf_value nieuwe sleutel" "$(fn read_conf_value "$C" NEW d)" "nieuw"
out=$(fn write_conf_value "$C" X $'a\nb'); rc=$?
has "write_conf_value weigert newline" "$out" "newline"
eq "pad kort" "$(fn pad 6 ab)|" "ab    |"
eq "pad afkappen" "$(fn pad 3 abcdef)" "abc"

mkdir -p "$SB/app/maps/Alpha" "$SB/app/maps/Beta" "$SB/app/config"
printf 'GamePort=7777\nBatchEnabled=false\nSaveDir=AlphaSave\n' > "$SB/app/maps/Alpha/map.conf"
printf 'GamePort=7779\n' > "$SB/app/maps/Beta/map.conf"
eq "get_maps" "$(fn get_maps)" "Alpha Beta"
fn is_batch_enabled Alpha >/dev/null; eq "is_batch_enabled false" "$?" "1"
fn is_batch_enabled Beta >/dev/null;  eq "is_batch_enabled default true" "$?" "0"
MOCK_PGREP= fn is_map_running Alpha >/dev/null; eq "is_map_running zonder proces" "$?" "1"
MOCK_PGREP=4242 fn is_map_running Alpha >/dev/null; eq "is_map_running met proces" "$?" "0"
eq "get_map_pid" "$(MOCK_PGREP=4242 fn get_map_pid Alpha)" "4242"

# ── 4. Dispatch (CLI-subcommando's) ──────────────────────────────────────────
out=$(mgr help);            has "help" "$out" "Usage:"
out=$(mgr --help);          has "--help" "$out" "Usage:"
out=$(mgr -h);              has "-h" "$out" "Usage:"
out=$(mgr bogus); rc=$?;    has "onbekend commando" "$out" "Unknown command: bogus"; eq "onbekend commando exit" "$rc" "1"
for c in start stop restart backup; do
    out=$(mgr $c); rc=$?;   has "$c zonder map" "$out" "Usage:"; eq "$c zonder map exit" "$rc" "1"
done
out=$(mgr rcon); rc=$?;     has "rcon zonder args" "$out" "Usage:"; eq "rcon zonder args exit" "$rc" "1"
out=$(mgr rcon Alpha); rc=$?; has "rcon zonder cmd" "$out" "Usage:"; eq "rcon zonder cmd exit" "$rc" "1"
out=$(mgr broadcast); rc=$?; has "broadcast zonder msg" "$out" "Usage:"; eq "broadcast zonder msg exit" "$rc" "1"
out=$(mgr add-map); rc=$?;  has "add-map zonder naam" "$out" "Usage:"; eq "add-map zonder naam exit" "$rc" "1"

if (( BASH4 == 1 )); then
    out=$(mgr plan-install); has "plan-install" "$out" "no changes"; has "plan-install toont maps" "$out" "Alpha"
    out=$(mgr status);       has "status kopregel" "$out" "MAP"; has "status map" "$out" "Alpha"
    out=$(mgr add-map 'bad name'); rc=$?; has "add-map ongeldige naam" "$out" "Invalid map name"; eq "add-map ongeldige naam exit" "$rc" "1"
    out=$(mgr add-map Onbekend); rc=$?;   has "add-map onbekende map" "$out" "not a known map"; eq "add-map onbekende map exit" "$rc" "1"
    out=$(mgr add-map Alpha);             has "add-map bestaat al" "$out" "already exists"
    : > "$MOCKLOG"
    out=$(mgr add-map Nieuw --internal Nieuw_WP --admin-pw geheim123); rc=$?
    eq "add-map exit" "$rc" "0"
    [[ -f "$SB/app/maps/Nieuw/map.conf" ]] && ok || bad "add-map maakt map.conf"
    eq "add-map MapName" "$(fn read_conf_value "$SB/app/maps/Nieuw/map.conf" MapName x)" "Nieuw_WP"
    eq "add-map poort volgt bestaande maps" "$(fn read_conf_value "$SB/app/maps/Nieuw/map.conf" GamePort x)" "7781"
    eq "add-map modus 600" "$(stat -c %a "$SB/app/maps/Nieuw/map.conf" 2>/dev/null || stat -f %Lp "$SB/app/maps/Nieuw/map.conf")" "600"
    has "add-map registreert in maps.conf" "$(cat "$SB/app/config/maps.conf")" "Nieuw|Nieuw_WP|enabled"
    has "add-map vraagt systemd (mock)" "$(cat "$MOCKLOG")" "daemon-reload"
    out=$(mgr add-map Aberration --admin-pw x); has "add-map bekende map" "$out" "created"
    eq "add-map bekende map internal" "$(fn read_conf_value "$SB/app/maps/Aberration/map.conf" MapName x)" "Aberration_WP"
else
    skip "dispatch plan-install/status/add-map vragen bash >= 4 (declare -A, \${var,,})"
fi

# Standaard-configs worden bij een eerste start aangemaakt.
for f in server-defaults.conf maps.conf optimization.conf; do
    [[ -f "$SB/app/config/$f" ]] && ok || { (( BASH4 )) && bad "config/$f aangemaakt" || ok; }
done

echo "tests: $PASS ok, $FAIL fout, $SKIP overgeslagen"
(( FAIL == 0 ))
