//! Embeds the module host zip named by `FACEBRIDGE_BUNDLED_HOST_ZIP` (the `module-host-<rid>.zip` release asset) into
//! the binary. Unset: nothing is bundled and `discover_module_host` looks beside the executable and in the dev output.

use std::{env, fs, path::PathBuf};

const VAR: &str = "FACEBRIDGE_BUNDLED_HOST_ZIP";

fn main() {
    println!("cargo::rustc-check-cfg=cfg(bundled_host)");
    println!("cargo::rerun-if-env-changed={VAR}");
    let Some(zip) = env::var_os(VAR).filter(|v| !v.is_empty()).map(PathBuf::from) else {
        return;
    };
    let zip = fs::canonicalize(&zip).unwrap_or_else(|e| panic!("{VAR}: {}: {e}", zip.display()));
    println!("cargo::rerun-if-changed={}", zip.display());
    let bytes = fs::read(&zip).unwrap_or_else(|e| panic!("{VAR}: reading {}: {e}", zip.display()));
    println!("cargo::rustc-cfg=bundled_host");
    println!("cargo::rustc-env=FACEBRIDGE_BUNDLED_HOST_ZIP={}", zip.display());
    println!("cargo::rustc-env=FACEBRIDGE_BUNDLED_HOST_ID={:016x}", fnv1a(&bytes));
}

/// FNV-1a: only needs to tell apart host builds, so the extracted copy is not reused across different hosts.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}
