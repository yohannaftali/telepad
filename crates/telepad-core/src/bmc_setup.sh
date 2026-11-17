# Telepad: one-time BMC setup, run by the user in an interactive
# terminal ("Set up BMC access" button). Unlike script.sh this CHANGES the host,
# so it only runs after the user reads it and presses Enter, and sudo asks for
# the password. Stdin is the script itself, so prompts read from /dev/tty.
set -e
U=$(id -un)
find_ipmitool() {
  for b in /usr/bin/ipmitool /usr/sbin/ipmitool; do
    [ -x "$b" ] && { echo "$b"; return 0; }
  done
  return 1
}
if command -v apt-get >/dev/null 2>&1; then PM="apt-get install -y ipmitool"
elif command -v dnf >/dev/null 2>&1; then PM="dnf install -y ipmitool"
elif command -v yum >/dev/null 2>&1; then PM="yum install -y ipmitool"
elif command -v zypper >/dev/null 2>&1; then PM="zypper --non-interactive install ipmitool"
else PM=""; fi

B=$(find_ipmitool || true)
echo "=== Telepad: BMC sensor access for '$U' on $(hostname) ==="
echo
echo "These commands will run with sudo:"
echo
if [ -z "$B" ]; then
  if [ -z "$PM" ]; then
    echo "  ipmitool is not installed and no apt/dnf/yum/zypper was found."
    echo "  Install ipmitool by hand, then press Refresh in the app."
    printf '\nPress Enter to close. '; read _ </dev/tty
    exit 1
  fi
  echo "  sudo $PM"
  SHOWN_B="/usr/bin/ipmitool"
else
  SHOWN_B="$B"
fi
echo "  echo '$U ALL=(root) NOPASSWD: $SHOWN_B sdr type Fan, $SHOWN_B sdr type Temperature' > /etc/sudoers.d/telepad-ipmi"
echo "  (checked with visudo first, installed with mode 0440)"
echo
echo "The rule lets the monitor read fan and temperature sensors only, nothing else."
printf 'Press Enter to run them (sudo will ask for your password), or Ctrl+C to cancel. '
read _ </dev/tty

if [ -z "$B" ]; then
  # shellcheck disable=SC2086
  sudo $PM </dev/tty
  B=$(find_ipmitool) || { echo "ipmitool still not found after install."; exit 1; }
fi
T=$(mktemp)
trap 'rm -f "$T"' EXIT
printf '%s ALL=(root) NOPASSWD: %s sdr type Fan, %s sdr type Temperature\n' "$U" "$B" "$B" >"$T"
sudo visudo -cf "$T" </dev/tty
sudo install -m 0440 -o root -g root "$T" /etc/sudoers.d/telepad-ipmi </dev/tty
echo
if sudo -n "$B" sdr type Fan >/dev/null 2>&1; then
  echo "Done: sensors are readable without a password. Press Refresh in Telepad."
else
  echo "The rule is installed but 'sudo -n ipmitool sdr type Fan' still fails; check /etc/sudoers.d."
fi
printf '\nPress Enter to close. '
read _ </dev/tty
