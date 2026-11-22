use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/boogie.rc");
    println!("cargo:rerun-if-changed=assets/logo.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let rc_file = manifest_dir.join("assets").join("boogie.rc");
    let res_file = out_dir.join("boogie_icon.res");

    if !rc_file.exists() {
        return;
    }

    if let Some(rc_exe) = find_rc_exe() {
        let status = Command::new(&rc_exe)
            .current_dir(&manifest_dir)
            .args([
                "/nologo",
                "/fo",
                res_file.to_str().unwrap(),
                rc_file.to_str().unwrap(),
            ])
            .status();

        if let Ok(s) = status
            && s.success()
            && res_file.exists()
        {
            println!("cargo:rustc-link-arg-bins={}", res_file.display());
            return;
        }
    }

    // Fallback: MinGW windres
    let obj_file = out_dir.join("boogie_icon.o");
    let status = Command::new("windres")
        .current_dir(&manifest_dir)
        .args([
            "-i",
            rc_file.to_str().unwrap(),
            "-o",
            obj_file.to_str().unwrap(),
            "-O",
            "coff",
        ])
        .status();

    if let Ok(s) = status
        && s.success()
        && obj_file.exists()
    {
        println!("cargo:rustc-link-arg-bins={}", obj_file.display());
    }
}

fn find_rc_exe() -> Option<PathBuf> {
    if Command::new("rc.exe").arg("/?").output().is_ok() {
        return Some(PathBuf::from("rc.exe"));
    }

    let kits_bin = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(entries) = std::fs::read_dir(&kits_bin) {
        let mut candidates: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join("x64").join("rc.exe"))
            .filter(|p| p.exists())
            .collect();
        candidates.sort();
        if let Some(latest) = candidates.pop() {
            return Some(latest);
        }
    }
    None
}
