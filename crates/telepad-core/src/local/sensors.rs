//! Sensors of My Computer on Windows (#193). Windows has no `/sys`, so the
//! readings come from three read-only sources, none needing administrator
//! rights, most complete first:
//!
//! 1. `nvidia-smi` (NVIDIA GPUs: temperature, fan %, power), parsed like the
//!    remote script's `===NVIDIA===` section;
//! 2. LibreHardwareMonitor's WMI namespace `root\LibreHardwareMonitor`, only
//!    if the user happens to run it (CPU, board, fans, drives and GPUs of every
//!    vendor). Owner decision on #193: Telepad never needs, suggests, installs
//!    or starts another program, so no message mentions it;
//! 3. the ACPI thermal zone (`sysinfo`), which many desktop boards lack.
//!
//! [`merge`] puts them together and says why readings are missing.

use crate::model::{Sensor, SensorKind};

/// Why a Windows PC shows no CPU / board temperatures: they sit behind the
/// hardware (MSRs, sensor chips), which only a kernel driver can read.
pub const NO_BOARD_TEMPS: &str = "this PC publishes no CPU or board temperatures to Windows (reading them needs a hardware driver, which Telepad does not install)";

/// Why a Windows PC shows no thermal zone to a normal user.
pub const ADMIN_ONLY: &str = "Windows shows this PC's temperatures only to administrators";

/// What the Windows thermal zone answered, for the note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneProbe {
    /// WMI refused: the zone exists but needs administrator rights.
    AccessDenied,
    /// The firmware publishes no thermal zone ("Not supported", or none).
    Missing,
    /// Zones exist, or WMI failed some other way.
    Other,
}

/// One LibreHardwareMonitor `Sensor` row (`SensorType` `Temperature` / `Fan`
/// / …; `Value` in °C / RPM; `Parent` = its hardware's identifier).
#[derive(Debug, Clone, PartialEq)]
pub struct LhmRow {
    pub name: String,
    pub sensor_type: String,
    pub value: f32,
    pub parent: String,
}

/// A LibreHardwareMonitor reading and the identifier of its hardware
/// (`/gpu-nvidia/0`, `/amdcpu/0`, …).
#[derive(Debug, Clone, PartialEq)]
pub struct LhmReading {
    pub parent: String,
    pub sensor: Sensor,
}

/// Temperatures and fans of LibreHardwareMonitor rows, labelled with their
/// hardware's name (`hardware`: identifier → name), sorted by chip and label.
/// "Distance to TjMax" rows count degrees below the limit, not a temperature.
pub fn lhm_readings(rows: Vec<LhmRow>, hardware: &[(String, String)]) -> Vec<LhmReading> {
    let mut out: Vec<LhmReading> = rows
        .into_iter()
        .filter_map(|r| {
            let kind = match r.sensor_type.as_str() {
                "Temperature" if crate::parse::plausible_temp(r.value) => SensorKind::Temperature,
                "Fan" if r.value.is_finite() && r.value >= 0.0 => SensorKind::Fan,
                _ => return None,
            };
            if r.name.starts_with("Distance to") {
                return None;
            }
            let chip = hardware
                .iter()
                .find(|(id, _)| *id == r.parent)
                .map_or_else(|| r.parent.clone(), |(_, n)| n.clone());
            Some(LhmReading {
                parent: r.parent,
                sensor: Sensor {
                    kind,
                    chip,
                    label: r.name,
                    value: r.value,
                },
            })
        })
        .collect();
    out.sort_by(|a, b| (&a.sensor.chip, &a.sensor.label).cmp(&(&b.sensor.chip, &b.sensor.label)));
    out
}

/// Merge the sources and say why readings are missing. `gpus` from
/// `nvidia-smi`, `lhm` from LibreHardwareMonitor, `zones` from the thermal
/// zone (only used without LibreHardwareMonitor). LibreHardwareMonitor's
/// NVIDIA sensors are dropped when `nvidia-smi` reported the GPUs already.
/// `probe` is only asked when no CPU / board temperature was found.
pub fn merge(
    gpus: Vec<Sensor>,
    lhm: Vec<LhmReading>,
    zones: Vec<Sensor>,
    probe: impl FnOnce() -> ZoneProbe,
) -> (Vec<Sensor>, Option<String>) {
    let have_lhm = !lhm.is_empty();
    let nvidia = !gpus.is_empty();
    let mut out = gpus;
    out.extend(
        lhm.into_iter()
            .filter(|r| !(nvidia && r.parent.starts_with("/gpu-nvidia")))
            .map(|r| r.sensor),
    );
    if !have_lhm {
        out.extend(zones);
    }
    let board = out.iter().any(|s| !s.chip.starts_with("GPU "));
    if have_lhm || board {
        return (out, None);
    }
    let why = match probe() {
        ZoneProbe::AccessDenied => ADMIN_ONLY,
        ZoneProbe::Missing | ZoneProbe::Other => NO_BOARD_TEMPS,
    };
    let note = if out.is_empty() {
        why.to_string()
    } else {
        format!("only the GPU is readable: {why}")
    };
    (out, Some(note))
}

/// Read every source. `run(program, args)` runs a short read-only command
/// (`None` when it is missing; `(success, stdout)` otherwise).
#[cfg(target_os = "windows")]
pub fn read(
    run: impl Fn(&str, &[&str]) -> Option<(bool, String)>,
) -> (Vec<Sensor>, Option<String>) {
    let mut gpus = Vec::new();
    if let Some((true, out)) = run(
        "nvidia-smi",
        &[
            "--query-gpu=index,name,temperature.gpu,fan.speed,power.draw",
            "--format=csv,noheader,nounits",
        ],
    ) {
        let lines: Vec<String> = out.lines().map(str::to_string).collect();
        gpus = crate::parse::parse_nvidia(&lines);
    }
    let lhm = wmi_read::lhm();
    let zones = if lhm.is_empty() {
        thermal_zones()
    } else {
        Vec::new()
    };
    merge(gpus, lhm, zones, wmi_read::thermal_zone_probe)
}

/// ACPI thermal zone temperatures through `sysinfo`.
#[cfg(target_os = "windows")]
fn thermal_zones() -> Vec<Sensor> {
    let comps = sysinfo::Components::new_with_refreshed_list().ok();
    comps
        .iter()
        .flatten()
        .filter_map(|c| {
            let t = c
                .temperature()
                .filter(|&t| crate::parse::plausible_temp(t))?;
            Some(Sensor {
                kind: SensorKind::Temperature,
                chip: "ACPI".into(),
                label: c.label().to_string(),
                value: t,
            })
        })
        .collect()
}

/// The WMI reads (the `wmi` crate initialises COM for this thread).
#[cfg(target_os = "windows")]
mod wmi_read {
    use std::collections::HashMap;

    use serde::Deserialize;

    use super::{LhmReading, LhmRow, ZoneProbe};

    const WBEM_E_ACCESS_DENIED: u32 = 0x8004_1003;
    const WBEM_E_NOT_SUPPORTED: u32 = 0x8004_100C;

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct WSensor {
        name: String,
        sensor_type: String,
        value: Option<f32>,
        parent: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct WHardware {
        identifier: String,
        name: String,
    }

    /// LibreHardwareMonitor's readings; empty when it does not run (its
    /// namespace is missing) or WMI fails.
    pub fn lhm() -> Vec<LhmReading> {
        let Ok(con) = wmi::WMIConnection::with_namespace_path(r"ROOT\LibreHardwareMonitor") else {
            return Vec::new();
        };
        let Ok(rows) = con.raw_query::<WSensor>(
            "SELECT Name, SensorType, Value, Parent FROM Sensor \
             WHERE SensorType = 'Temperature' OR SensorType = 'Fan'",
        ) else {
            return Vec::new();
        };
        let hardware: Vec<(String, String)> = con
            .raw_query::<WHardware>("SELECT Identifier, Name FROM Hardware")
            .map(|v| v.into_iter().map(|h| (h.identifier, h.name)).collect())
            .unwrap_or_default();
        let rows = rows
            .into_iter()
            .filter_map(|r| {
                Some(LhmRow {
                    name: r.name,
                    sensor_type: r.sensor_type,
                    value: r.value?,
                    parent: r.parent,
                })
            })
            .collect();
        super::lhm_readings(rows, &hardware)
    }

    /// Ask WMI for the ACPI thermal zone directly: `sysinfo` hides the error
    /// that tells "needs administrator" from "this PC has none".
    pub fn thermal_zone_probe() -> ZoneProbe {
        let Ok(con) = wmi::WMIConnection::with_namespace_path(r"ROOT\WMI") else {
            return ZoneProbe::Other;
        };
        match con.raw_query::<HashMap<String, wmi::Variant>>(
            "SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature",
        ) {
            Ok(v) if v.is_empty() => ZoneProbe::Missing,
            Ok(_) => ZoneProbe::Other,
            Err(wmi::WMIError::HResultError { hres }) => match hres as u32 {
                WBEM_E_ACCESS_DENIED => ZoneProbe::AccessDenied,
                WBEM_E_NOT_SUPPORTED => ZoneProbe::Missing,
                _ => ZoneProbe::Other,
            },
            Err(_) => ZoneProbe::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(kind: SensorKind, chip: &str, label: &str, value: f32) -> Sensor {
        Sensor {
            kind,
            chip: chip.into(),
            label: label.into(),
            value,
        }
    }

    fn row(name: &str, ty: &str, value: f32, parent: &str) -> LhmRow {
        LhmRow {
            name: name.into(),
            sensor_type: ty.into(),
            value,
            parent: parent.into(),
        }
    }

    fn hw() -> Vec<(String, String)> {
        vec![
            ("/amdcpu/0".into(), "Example CPU 8-Core".into()),
            ("/lpc/nct6799d".into(), "Example Board Chip".into()),
            ("/gpu-nvidia/0".into(), "Example GPU".into()),
        ]
    }

    #[test]
    fn lhm_rows_become_named_temperatures_and_fans() {
        let rows = vec![
            row("Core (Tctl/Tdie)", "Temperature", 55.5, "/amdcpu/0"),
            row("Distance to TjMax", "Temperature", 45.0, "/amdcpu/0"),
            row("CPU Fan", "Fan", 1180.0, "/lpc/nct6799d"),
            row("Package", "Power", 61.0, "/amdcpu/0"),
            row("GPU Core", "Temperature", 47.0, "/gpu-nvidia/0"),
            row("Bogus", "Temperature", 0.0, "/unknown/0"),
        ];
        let got: Vec<_> = lhm_readings(rows, &hw())
            .into_iter()
            .map(|r| r.sensor)
            .collect();
        assert_eq!(
            got,
            [
                s(SensorKind::Fan, "Example Board Chip", "CPU Fan", 1180.0),
                s(
                    SensorKind::Temperature,
                    "Example CPU 8-Core",
                    "Core (Tctl/Tdie)",
                    55.5
                ),
                s(SensorKind::Temperature, "Example GPU", "GPU Core", 47.0),
            ]
        );
    }

    #[test]
    fn merge_prefers_nvidia_smi_and_explains_what_is_missing() {
        let gpu = || vec![s(SensorKind::Temperature, "GPU 0", "Example GPU", 47.0)];
        let lhm = || {
            lhm_readings(
                vec![
                    row("Core (Tctl/Tdie)", "Temperature", 55.5, "/amdcpu/0"),
                    row("GPU Core", "Temperature", 47.0, "/gpu-nvidia/0"),
                ],
                &hw(),
            )
        };
        let zone = || vec![s(SensorKind::Temperature, "ACPI", "TZ0", 40.0)];
        let never = || -> ZoneProbe { panic!("not asked when the CPU is readable") };

        // LibreHardwareMonitor running: its GPU row gives way to nvidia-smi,
        // the thermal zone is not used, no note.
        let (v, note) = merge(gpu(), lhm(), zone(), never);
        let chips: Vec<_> = v.iter().map(|s| s.chip.as_str()).collect();
        assert_eq!(chips, ["GPU 0", "Example CPU 8-Core"]);
        assert_eq!(note, None);
        // Without nvidia-smi its NVIDIA rows stay.
        assert_eq!(merge(vec![], lhm(), vec![], never).0.len(), 2);
        // Only a thermal zone: shown, no note.
        assert_eq!(merge(vec![], vec![], zone(), never), (zone(), None));

        // Nothing but the GPU (this PC's case: board publishes no zone).
        let (v, note) = merge(gpu(), vec![], vec![], || ZoneProbe::Missing);
        assert_eq!(v, gpu());
        assert_eq!(
            note.as_deref(),
            Some(
                "only the GPU is readable: this PC publishes no CPU or board temperatures to Windows (reading them needs a hardware driver, which Telepad does not install)"
            )
        );
        assert!(
            !note.unwrap_or_default().contains("LibreHardwareMonitor"),
            "no other program is suggested (owner decision)"
        );
        // Nothing at all, zone refused for non-administrators.
        let (v, note) = merge(vec![], vec![], vec![], || ZoneProbe::AccessDenied);
        assert!(v.is_empty());
        assert_eq!(note.as_deref(), Some(ADMIN_ONLY));
        let (_, note) = merge(vec![], vec![], vec![], || ZoneProbe::Missing);
        assert_eq!(note.as_deref(), Some(NO_BOARD_TEMPS));
    }
}
