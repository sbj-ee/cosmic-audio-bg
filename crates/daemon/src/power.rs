//! Battery and lid state detection for power-aware FPS.

use std::fs;
use std::path::PathBuf;

pub fn on_battery() -> bool {
    let power_supply = PathBuf::from("/sys/class/power_supply");
    let Ok(entries) = fs::read_dir(&power_supply) else {
        return false;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let type_path = path.join("type");
        let status_path = path.join("status");

        let Ok(type_text) = fs::read_to_string(&type_path) else {
            continue;
        };
        if type_text.trim() != "Battery" {
            continue;
        }

        if let Ok(status) = fs::read_to_string(&status_path) {
            return status.trim() == "Discharging";
        }
    }

    false
}

pub fn lid_closed() -> bool {
    let lid_path = PathBuf::from("/proc/acpi/button/lid/LID0/state");
    if let Ok(text) = fs::read_to_string(&lid_path) {
        return text.contains("closed");
    }

    // Fallback for newer kernels
    let input_path = PathBuf::from("/sys/class/input");
    if let Ok(entries) = fs::read_dir(input_path) {
        for entry in entries.flatten() {
            let name_path = entry.path().join("device/name");
            if let Ok(name) = fs::read_to_string(&name_path) {
                if name.trim().contains("Lid") {
                    let state_path = entry.path().join("device/state");
                    if let Ok(state) = fs::read_to_string(&state_path) {
                        return state.trim() == "closed";
                    }
                }
            }
        }
    }

    false
}
