#!/bin/sh
# Privileged setup for worki's macOS-user isolation. Run as root by worki through the system
# authentication dialog. Idempotent.
#
#   worki-admin.sh setup <human> <agent-user>
#   worki-admin.sh remove <agent-user>   (only users in the worki group; deletes their home)
set -eu

GROUP=_worki_agents
SUDOERS=/etc/sudoers.d/worki

valid_name() {
    case "$1" in
        ''|*[!a-zA-Z0-9_.-]*) echo "invalid name: $1" >&2; exit 2 ;;
    esac
}

free_id() {
    used=$(dscl . -list "$1" "$2" | awk '{print $2}')
    i=450
    while [ "$i" -lt 500 ]; do
        if ! printf '%s\n' "$used" | grep -qx "$i"; then
            echo "$i"
            return
        fi
        i=$((i + 1))
    done
    echo "no free id in 450-499 for $1" >&2
    exit 1
}

ensure_group() {
    if ! dscl . -read "/Groups/$GROUP" >/dev/null 2>&1; then
        gid=$(free_id /Groups PrimaryGroupID)
        dscl . -create "/Groups/$GROUP"
        dscl . -create "/Groups/$GROUP" PrimaryGroupID "$gid"
        dscl . -create "/Groups/$GROUP" RealName "worki agents"
        dscl . -create "/Groups/$GROUP" Password '*'
    fi
}

ensure_user() {
    user=$1
    # Not "staff": home folders are group-readable by staff (drwxr-x---), so an agent in staff
    # could read the human's files.
    gid=$(dscl . -read "/Groups/$GROUP" PrimaryGroupID | awk '{print $2}')
    if ! dscl . -read "/Users/$user" >/dev/null 2>&1; then
        uid=$(free_id /Users UniqueID)
        dscl . -create "/Users/$user"
        dscl . -create "/Users/$user" UserShell /bin/zsh
        dscl . -create "/Users/$user" RealName "worki agent"
        dscl . -create "/Users/$user" UniqueID "$uid"
        dscl . -create "/Users/$user" NFSHomeDirectory "/Users/$user"
        dscl . -create "/Users/$user" IsHidden 1
        dscl . -create "/Users/$user" Password '*'
    fi
    dscl . -create "/Users/$user" PrimaryGroupID "$gid"
    uid=$(dscl . -read "/Users/$user" UniqueID | awk '{print $2}')
    wait_until_resolvable "$user"
    if [ ! -d "/Users/$user" ]; then
        createhomedir -c -u "$user" >/dev/null 2>&1 || true
    fi
    if [ ! -d "/Users/$user" ]; then
        mkdir -p "/Users/$user"
        chmod 700 "/Users/$user"
    fi
    # Numeric ids: name lookups can lag behind freshly created records.
    chown "$uid:$gid" "/Users/$user"
    if ! group_lists "$user"; then
        dscl . -append "/Groups/$GROUP" GroupMembership "$user"
    fi
    group_lists "$user" || { echo "could not add $user to $GROUP" >&2; exit 1; }
    dscacheutil -flushcache 2>/dev/null || true
}

# New records reach the lookup cache (used by chown, createhomedir, sudo) with a delay.
wait_until_resolvable() {
    dscacheutil -flushcache 2>/dev/null || true
    i=0
    while ! id -u "$1" >/dev/null 2>&1 && [ "$i" -lt 20 ]; do
        sleep 0.5
        i=$((i + 1))
    done
}

group_lists() {
    dscl . -read "/Groups/$GROUP" GroupMembership 2>/dev/null | tr ' ' '\n' | grep -qx "$1"
}

# Guards deletion: a member of the worki group, or a user carrying the marker and UID range
# that ensure_user gives every account it creates.
created_by_worki() {
    if group_lists "$1" || dsmemberutil checkmembership -U "$1" -G "$GROUP" 2>/dev/null | grep -q "is a member"; then
        return 0
    fi
    name=$(dscl . -read "/Users/$1" RealName 2>/dev/null | sed 's/^RealName://' | tr -d '\n' | sed 's/^ *//')
    uid=$(dscl . -read "/Users/$1" UniqueID 2>/dev/null | awk '{print $2}')
    [ "$name" = "worki agent" ] && [ "${uid:-0}" -ge 450 ] && [ "${uid:-0}" -lt 500 ]
}

install_sudoers() {
    human=$1
    tmp=$(mktemp /tmp/worki-sudoers.XXXXXX)
    printf '# Managed by worki: lets %s run commands as worki agent users, nothing more.\n%s ALL=(%%%s) NOPASSWD: ALL\n' \
        "$human" "$human" "$GROUP" >"$tmp"
    visudo -cf "$tmp" >/dev/null
    install -m 0440 -o root -g wheel "$tmp" "$SUDOERS"
    rm -f "$tmp"
}

cmd=${1:-}
case "$cmd" in
    setup)
        human=$2 user=$3
        valid_name "$human"
        valid_name "$user"
        ensure_group
        ensure_user "$user"
        install_sudoers "$human"
        ;;
    remove)
        user=$2
        valid_name "$user"
        if ! created_by_worki "$user"; then
            echo "$user was not created by worki; refusing to remove it" >&2
            exit 3
        fi
        home=$(dscl . -read "/Users/$user" NFSHomeDirectory 2>/dev/null | awk '{print $2}')
        pkill -9 -u "$user" 2>/dev/null || true
        # Recent macOS refuses `dscl -delete` on user records even for root; sysadminctl is the
        # supported way (and removes the home folder too).
        sysadminctl_out=$(sysadminctl -deleteUser "$user" 2>&1) || true
        if dscl . -read "/Users/$user" >/dev/null 2>&1; then
            dscl_out=$(dscl . -delete "/Users/$user" 2>&1) || true
        fi
        if dscl . -read "/Users/$user" >/dev/null 2>&1; then
            printf 'macOS did not delete %s.\nsysadminctl: %s\ndscl: %s\n' "$user" "$sysadminctl_out" "${dscl_out:-}" >&2
            exit 1
        fi
        # Only once the account is gone: leaving the group earlier would strand a still-existing
        # user without its sudo rule.
        dscl . -delete "/Groups/$GROUP" GroupMembership "$user" 2>/dev/null || true
        if [ "$home" = "/Users/$user" ] && [ -d "$home" ]; then
            rm -rf "$home"
        fi
        members=$(dscl . -read "/Groups/$GROUP" GroupMembership 2>/dev/null | sed 's/^GroupMembership://' | tr -d ' ')
        if [ -z "$members" ]; then
            dseditgroup -o delete "$GROUP" >/dev/null 2>&1 || dscl . -delete "/Groups/$GROUP" 2>/dev/null || true
            rm -f "$SUDOERS"
        fi
        ;;
    *)
        echo "usage: $0 setup <human> <agent-user> | remove <agent-user>" >&2
        exit 2
        ;;
esac
