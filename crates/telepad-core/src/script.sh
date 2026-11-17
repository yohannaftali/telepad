# Telepad remote collector. POSIX sh, read-only, BusyBox-friendly.
# Every section prints a ===NAME=== marker; a missing tool leaves it empty.
export LC_ALL=C
PATH="$PATH:/usr/sbin:/sbin:/usr/local/bin"

echo "===HOSTNAME==="
hostname 2>/dev/null || cat /proc/sys/kernel/hostname 2>/dev/null

echo "===OSRELEASE==="
if [ -r /etc/os-release ]; then cat /etc/os-release
elif [ -r /etc/synoinfo.conf ]; then echo "NAME=Synology DSM"; grep -E '^(productversion|buildnumber)=' /etc.defaults/VERSION 2>/dev/null
elif [ -r /etc/redhat-release ]; then echo "PRETTY_NAME=\"$(cat /etc/redhat-release)\""
fi

echo "===UNAME==="
uname -s 2>/dev/null
uname -r 2>/dev/null
uname -m 2>/dev/null

echo "===UPTIME==="
cat /proc/uptime 2>/dev/null

echo "===LOADAVG==="
cat /proc/loadavg 2>/dev/null

echo "===CPUINFO==="
grep -m1 -E '^(model name|Hardware|Processor|cpu model)' /proc/cpuinfo 2>/dev/null
echo "cores: $(grep -c '^processor' /proc/cpuinfo 2>/dev/null)"

echo "===STAT1==="
grep '^cpu ' /proc/stat 2>/dev/null
sleep 1
echo "===STAT2==="
grep '^cpu ' /proc/stat 2>/dev/null

echo "===MEMINFO==="
grep -E '^(MemTotal|MemFree|MemAvailable|Buffers|Cached|SReclaimable|SwapTotal|SwapFree):' /proc/meminfo 2>/dev/null

echo "===DF==="
df -P -k 2>/dev/null

echo "===IPADDR==="
if command -v ip >/dev/null 2>&1; then
  ip -o addr show 2>/dev/null
elif command -v ifconfig >/dev/null 2>&1; then
  ifconfig 2>/dev/null | awk '/^[a-zA-Z0-9]/{i=$1} /inet /{a=$2; sub("addr:","",a); print i, "inet", a}'
fi

echo "===THERMAL==="
for z in /sys/class/thermal/thermal_zone*; do
  [ -r "$z/temp" ] || continue
  echo "$(cat "$z/type" 2>/dev/null)|$(cat "$z/temp" 2>/dev/null)"
done

echo "===HWMON==="
for h in /sys/class/hwmon/hwmon*; do
  [ -d "$h" ] || continue
  n=$(cat "$h/name" 2>/dev/null)
  for t in "$h"/temp*_input "$h"/fan*_input; do
    [ -r "$t" ] || continue
    b=${t%_input}
    l=$(cat "${b}_label" 2>/dev/null)
    echo "$n|$(basename "$b")|$l|$(cat "$t" 2>/dev/null)"
  done
done

echo "===IPMI==="
# BMC sensors (Dell iDRAC etc.). /dev/ipmi0 is root-only, so this needs a sudoers rule
# allowing exactly "ipmitool sdr type Fan" and "ipmitool sdr type Temperature".
if [ -c /dev/ipmi0 ] || [ -c /dev/ipmi/0 ] || [ -c /dev/ipmidev/0 ]; then
  IPMI=""
  for b in /usr/bin/ipmitool /usr/sbin/ipmitool; do
    [ -x "$b" ] && { IPMI="$b"; break; }
  done
  if [ -z "$IPMI" ]; then
    echo "ERR ipmitool not installed"
  elif ! sudo -n -l "$IPMI" sdr type Fan >/dev/null 2>&1; then
    echo "ERR sudo rule missing for $IPMI"
  else
    for t in Fan Temperature; do
      echo "#$t"
      sudo -n "$IPMI" sdr type "$t" 2>/dev/null
    done
  fi
fi

echo "===LISTEN==="
# Process names need root; without passwordless sudo only ports are shown.
if command -v ss >/dev/null 2>&1; then
  sudo -n ss -tulnp 2>/dev/null || ss -tulnp 2>/dev/null || ss -tuln 2>/dev/null
elif command -v netstat >/dev/null 2>&1; then
  sudo -n netstat -tulnp 2>/dev/null || netstat -tuln 2>/dev/null
fi

echo "===DOCKER==="
# Docker first, then Podman (a podman-docker shim reports "podman" in --version).
D=""
for rt in docker podman; do
  command -v "$rt" >/dev/null 2>&1 || continue
  if "$rt" ps -q >/dev/null 2>&1; then D="$rt"; break
  elif sudo -n "$rt" ps -q >/dev/null 2>&1; then D="sudo -n $rt"; break
  fi
done
if [ -z "$D" ]; then
  if command -v docker >/dev/null 2>&1 || command -v podman >/dev/null 2>&1; then
    echo "ERR permission denied"
  else
    echo "ERR not installed"
  fi
else
  echo "RUNTIME $($D --version 2>/dev/null | head -n 1)"
  case "$D" in
    *podman*) L() { printf '{{index .Labels "%s"}}' "$1"; } ;;
    *) L() { printf '{{.Label "%s"}}' "$1"; } ;;
  esac
  P=com.docker.compose.project
  $D ps -a --no-trunc --format "{{.ID}}|{{.Names}}|{{.Image}}|{{.State}}|{{.Status}}|$(L $P)|$(L $P.working_dir)|$(L $P.config_files)|{{.Ports}}" 2>/dev/null
  echo "===DOCKERNET==="
  ids=$($D ps -aq --no-trunc 2>/dev/null)
  if [ -n "$ids" ]; then
    $D inspect -f '{{.Id}}|{{range $k, $v := .NetworkSettings.Networks}}{{$k}}={{$v.IPAddress}} {{end}}|{{range .Mounts}}{{.Source}} {{end}}' $ids 2>/dev/null
  fi
fi

echo "===END==="
