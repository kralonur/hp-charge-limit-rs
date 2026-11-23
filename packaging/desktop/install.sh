#!/bin/sh
# Per-user desktop files only: no service changes and no tray autostart.
set -eu

APP_ID=io.github.kralonur.HpChargeLimit
BINARIES='hp-charge-limit-rs hp-charge-limit-gui hp-charge-limit-tray'
case "${1:-}" in
    '') action=install ;;
    --uninstall) action=uninstall ;;
    -h|--help)
        printf 'Usage: %s [--uninstall]\nInstall optional desktop files for this user, without changing the service.\n' "$0"
        exit 0 ;;
    *) printf 'Unknown argument: %s\n' "$1" >&2; exit 2 ;;
esac
[ "$#" -le 1 ] || exit 2
[ "$(id -u)" -ne 0 ] || { printf 'Run as your desktop user, not with sudo.\n' >&2; exit 1; }
: "${HOME:?HOME must be set}"
source_dir=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
install_dir="$HOME/.local/lib/hp-charge-limit-desktop"
marker="$install_dir/.hp-charge-limit-desktop"
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
case "$data_home" in /*) ;; *) data_home="$HOME/.local/share" ;; esac
case "$config_home" in /*) ;; *) config_home="$HOME/.config" ;; esac
applications="$data_home/applications"
case "$install_dir" in
    *'
'*|*"$(printf '\r')"*) printf 'Installation path cannot contain newlines.\n' >&2; exit 1 ;;
esac

if [ -e "$install_dir" ]; then
    header=''
    [ ! -f "$marker" ] || IFS= read -r header < "$marker"
    [ "$header" = hp-charge-limit-desktop ] || {
        printf 'Refusing to replace an unrecognized installation at %s\n' "$install_dir" >&2
        exit 1
    }
fi

# Desktop Entry string unescaping happens before Exec argument unquoting.
desktop_quote() {
    printf '%s' "$1" | sed -e 's/\\/\\\\\\\\/g' -e 's/"/\\\\"/g' \
        -e 's/`/\\\\`/g' -e 's/\$/\\\\$/g' -e 's/%/%%/g'
}

if [ "$action" = uninstall ]; then
    [ -f "$marker" ] || { printf 'No owned desktop installation found.\n' >&2; exit 1; }
    rm -f "$applications/$APP_ID.Gui.desktop" "$applications/$APP_ID.Tray.desktop"
    autostart="$config_home/autostart/$APP_ID.Tray.desktop"
    exec_line="Exec=\"$(desktop_quote "$install_dir/hp-charge-limit-tray")\""
    if [ -f "$autostart" ] && grep -Fxq "$exec_line" "$autostart"; then
        rm -f "$autostart"
    fi
    for binary in $BINARIES; do rm -f "$install_dir/$binary"; done
    rm -f "$marker"
    rmdir "$install_dir" 2>/dev/null || true
else
    for binary in $BINARIES; do
        [ -x "$source_dir/$binary" ] || { printf 'Missing executable beside installer: %s\n' "$binary" >&2; exit 1; }
    done
    "$source_dir/hp-charge-limit-rs" --version >/dev/null
    mkdir -p "$install_dir" "$applications"
    stage=$(mktemp -d "$install_dir/.update.XXXXXX")
    trap 'rm -rf "$stage"' EXIT
    trap 'exit 1' HUP INT TERM
    for binary in $BINARIES; do
        install -m755 "$source_dir/$binary" "$stage/$binary"
    done
    for binary in $BINARIES; do mv -f "$stage/$binary" "$install_dir/$binary"; done
    printf 'hp-charge-limit-desktop\n' > "$marker"
    for kind in Gui Tray; do
        case "$kind" in
            Gui) name='HP Charge Limit'; executable=hp-charge-limit-gui ;;
            Tray) name='HP Charge Limit Tray'; executable=hp-charge-limit-tray ;;
        esac
        {
            printf '[Desktop Entry]\nType=Application\nName=%s\n' "$name"
            printf 'Exec="%s"\n' "$(desktop_quote "$install_dir/$executable")"
            printf 'Icon=battery-good-symbolic\nTerminal=false\nCategories=System;\n'
        } > "$applications/$APP_ID.$kind.desktop"
    done
    printf 'Installed in %s. Open HP Charge Limit from your application menu.\n' "$install_dir"
fi
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$applications" >/dev/null 2>&1 || true
fi
printf 'The system CLI, service and its settings were not changed. Tray login startup is opt-in.\n'
