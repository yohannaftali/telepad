# Telepad remote collector. POSIX sh, read-only, BusyBox-friendly.
# Every section prints a ===NAME=== marker; a missing tool leaves it empty.
# The collector puts TP_STATIC= / TP_SLOW= lines in front: 1 runs that tier
# (static: once a day; slow: every 6 hours), empty skips it. Live metrics always run.
export LC_ALL=C
PATH="$PATH:/usr/sbin:/sbin:/usr/local/bin"
[ -n "${TP_STATIC+x}" ] || TP_STATIC=1
[ -n "${TP_SLOW+x}" ] || TP_SLOW=1

echo "===TELEPAD==="

if [ -n "$TP_STATIC" ]; then
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

echo "===CPUINFO==="
grep -m1 -E '^(model name|Hardware|Processor|cpu model)' /proc/cpuinfo 2>/dev/null
echo "cores: $(grep -c '^processor' /proc/cpuinfo 2>/dev/null)"
fi

echo "===UPTIME==="
cat /proc/uptime 2>/dev/null

echo "===LOADAVG==="
cat /proc/loadavg 2>/dev/null

echo "===STAT1==="
grep '^cpu ' /proc/stat 2>/dev/null
sleep 1
echo "===STAT2==="
grep '^cpu ' /proc/stat 2>/dev/null

echo "===MEMINFO==="
grep -E '^(MemTotal|MemFree|MemAvailable|Buffers|Cached|SReclaimable|SwapTotal|SwapFree):' /proc/meminfo 2>/dev/null

echo "===DF==="
df -P -k 2>/dev/null

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

echo "===NVIDIA==="
# NVIDIA GPUs (#193): the proprietary driver publishes nothing in hwmon. Read-only, no sudo.
if command -v nvidia-smi >/dev/null 2>&1; then
  NT=""; command -v timeout >/dev/null 2>&1 && NT="timeout 10"
  $NT nvidia-smi --query-gpu=index,name,temperature.gpu,fan.speed,power.draw     --format=csv,noheader,nounits 2>/dev/null
fi

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

if [ -n "$TP_SLOW" ]; then
echo "===IPADDR==="
if command -v ip >/dev/null 2>&1; then
  ip -o addr show 2>/dev/null
elif command -v ifconfig >/dev/null 2>&1; then
  ifconfig 2>/dev/null | awk '/^[a-zA-Z0-9]/{i=$1} /inet /{a=$2; sub("addr:","",a); print i, "inet", a}'
fi

echo "===LISTEN==="
# Process names need root; without passwordless sudo only ports are shown.
if command -v ss >/dev/null 2>&1; then
  sudo -n ss -tulnp 2>/dev/null || ss -tulnp 2>/dev/null || ss -tuln 2>/dev/null
elif command -v netstat >/dev/null 2>&1; then
  sudo -n netstat -tulnp 2>/dev/null || netstat -tuln 2>/dev/null
fi

echo "===UPDATES==="
# Read-only and offline: only the local package cache is consulted.
echo "today=$(date +%Y-%m-%d 2>/dev/null)"
T=""; command -v timeout >/dev/null 2>&1 && T="timeout 20"
if [ -f /var/run/reboot-required ]; then
  echo "reboot=yes"
  [ -r /var/run/reboot-required.pkgs ] && sort -u /var/run/reboot-required.pkgs 2>/dev/null | head -n 20 | sed 's/^/rebootpkg=/'
elif command -v needs-restarting >/dev/null 2>&1; then
  $T needs-restarting -r >/dev/null 2>&1
  case $? in 0) echo "reboot=no" ;; 1) echo "reboot=yes" ;; esac
elif [ -d /var/lib/dpkg ]; then
  echo "reboot=no"
elif command -v rpm >/dev/null 2>&1; then
  # RPM systems without needs-restarting: running kernel older than the newest installed one.
  k=$(rpm -q --last kernel 2>/dev/null | head -n 1 | awk '{print $1}')
  if [ -n "$k" ]; then
    if [ "$k" = "kernel-$(uname -r)" ]; then echo "reboot=no"; else echo "reboot=yes"; echo "rebootpkg=$k"; fi
  fi
fi
if [ -x /usr/lib/update-notifier/apt-check ]; then
  r=$($T /usr/lib/update-notifier/apt-check 2>&1 >/dev/null | tail -n 1)
  case "$r" in *[0-9]\;[0-9]*) echo "apt=$r" ;; esac
elif command -v apt-get >/dev/null 2>&1; then
  s=$($T apt-get -s -o Debug::NoLocking=1 upgrade 2>/dev/null | grep '^Inst ')
  n=$(printf '%s' "$s" | grep -c '^Inst ')
  sec=$(printf '%s' "$s" | grep -ci 'security')
  echo "apt=$n;$sec"
elif command -v dnf >/dev/null 2>&1; then
  n=$($T dnf -q --cacheonly check-update 2>/dev/null | grep -cE '^[^ ]+[.][^ ]+ +[^ ]+ +[^ ]+$')
  sec=$($T dnf -q --cacheonly check-update --security 2>/dev/null | grep -cE '^[^ ]+[.][^ ]+ +[^ ]+ +[^ ]+$')
  echo "dnf=$n;$sec"
elif command -v yum >/dev/null 2>&1; then
  n=$($T yum -q -C check-update 2>/dev/null | grep -cE '^[^ ]+[.][^ ]+ +[^ ]+ +[^ ]+$')
  sec=$($T yum -q -C --security check-update 2>/dev/null | grep -cE '^[^ ]+[.][^ ]+ +[^ ]+ +[^ ]+$')
  echo "yum=$n;$sec"
elif command -v apk >/dev/null 2>&1; then
  echo "apk=$($T apk -u list 2>/dev/null | grep -c .)"
fi

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

# Cluster sections: read-only, every call time-limited, output capped.
TO=""
command -v timeout >/dev/null 2>&1 && TO="timeout 15"

echo "===SWARM==="
if [ -n "$D" ]; then
  sw=$($TO $D info --format '{{.Swarm.LocalNodeState}}|{{.Swarm.ControlAvailable}}' 2>/dev/null)
  case "$sw" in
    active\|true)
      cid=$($TO $D info --format '{{.Swarm.Cluster.ID}}' 2>/dev/null)
      nid=$($TO $D info --format '{{.Swarm.NodeID}}' 2>/dev/null)
      echo "info=$sw|$cid|$nid"
      $TO $D node ls --format '{{json .}}' 2>/dev/null | head -n 200 | sed 's/^/node=/'
      $TO $D stack ls --format '{{json .}}' 2>/dev/null | head -n 200 | sed 's/^/stack=/'
      $TO $D service ls --format '{{json .}}' 2>/dev/null | head -n 500 | sed 's/^/service=/'
      svcs=$($TO $D service ls -q 2>/dev/null | head -n 500)
      if [ -n "$svcs" ]; then
        $TO $D service ps $svcs --filter desired-state=running --format '{{json .}}' 2>/dev/null \
          | grep -v '"CurrentState":"Running' | head -n 50 | sed 's/^/task=/'
      fi
      ;;
    active\|*)
      nid=$($TO $D info --format '{{.Swarm.NodeID}}' 2>/dev/null)
      echo "info=$sw||$nid"
      ;;
  esac
fi

echo "===KUBE==="
for k in kubectl "k3s kubectl" "microk8s kubectl"; do
  set -- $k
  command -v "$1" >/dev/null 2>&1 || continue
  K=""
  if $TO $k get nodes --request-timeout=8s -o name >/dev/null 2>&1; then K="$k"
  elif sudo -n $k get nodes --request-timeout=8s -o name >/dev/null 2>&1; then K="sudo -n $k"
  fi
  if [ -z "$K" ]; then
    echo "ERR $1 found but no access to the cluster (kubeconfig not readable by this user)"
    continue
  fi
  echo "tool=$k"
  echo "id=$($TO $K get namespace kube-system --request-timeout=8s -o jsonpath='{.metadata.uid}' 2>/dev/null)"
  $TO $K get nodes --request-timeout=15s -o jsonpath='{range .items[*]}node={.metadata.name}|{.status.conditions[?(@.type=="Ready")].status}|{.status.nodeInfo.kubeletVersion}{"\n"}{end}' 2>/dev/null | head -n 500
  for kind in deployments statefulsets; do
    $TO $K get "$kind" -A --request-timeout=15s -o jsonpath='{range .items[*]}wl='"$kind"'|{.metadata.namespace}|{.metadata.name}|{.status.readyReplicas}|{.spec.replicas}{"\n"}{end}' 2>/dev/null | head -n 1000
  done
  $TO $K get daemonsets -A --request-timeout=15s -o jsonpath='{range .items[*]}wl=daemonsets|{.metadata.namespace}|{.metadata.name}|{.status.numberReady}|{.status.desiredNumberScheduled}{"\n"}{end}' 2>/dev/null | head -n 1000
  # Only problem pods: not Running/Succeeded, waiting (CrashLoopBackOff...), or 5+ restarts.
  $TO $K get pods -A --request-timeout=15s -o jsonpath='{range .items[*]}{.metadata.namespace}|{.metadata.name}|{.status.phase}|{.status.containerStatuses[*].restartCount}|{.status.containerStatuses[*].state.waiting.reason}{"\n"}{end}' 2>/dev/null \
    | awk -F'|' '{ bad = ($3 != "Running" && $3 != "Succeeded") || $5 != ""; n = split($4, r, " "); for (i = 1; i <= n; i++) if (r[i] + 0 >= 5) bad = 1; if (bad && shown < 100) { print "pod=" $0; shown++ } } END { print "pods=" NR }'
  break
done

echo "===END==="
