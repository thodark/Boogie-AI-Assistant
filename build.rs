use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=assets/boogie.rc");
    println!("cargo:rerun-if-changed=assets/logo.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon_file = manifest_dir.join("assets").join("logo.ico");
    if !icon_file.exists() {
        return;
    }

    let pkg_ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.4.0".to_string());
    let nums: Vec<u16> = pkg_ver
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u16>()
                .unwrap_or(0)
        })
        .collect();
    let maj = nums.first().copied().unwrap_or(0);
    let min = nums.get(1).copied().unwrap_or(4);
    let pat = nums.get(2).copied().unwrap_or(0);

    let icon_rc_path = icon_file.to_string_lossy().replace('\\', "\\\\");
    let rc_content = format!(
        r#"1 ICON "{icon_rc_path}"

1 VERSIONINFO
FILEVERSION {maj},{min},{pat},0
PRODUCTVERSION {maj},{min},{pat},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Suyz Nguyen"
      VALUE "FileDescription", "Boogie AI Assistant v{pkg_ver}"
      VALUE "FileVersion", "{maj}.{min}.{pat}.0"
      VALUE "InternalName", "Boogie-v{pkg_ver}"
      VALUE "LegalCopyright", "MIT License - Suyz Nguyen"
      VALUE "OriginalFilename", "Boogie-v{pkg_ver}.exe"
      VALUE "ProductName", "Boogie AI Assistant"
      VALUE "ProductVersion", "{pkg_ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#
    );

    let generated_rc = out_dir.join("boogie_generated.rc");
    if std::fs::write(&generated_rc, rc_content).is_err() {
        return;
    }

    let res_file = out_dir.join("boogie_icon.res");

    if let Some(rc_exe) = find_rc_exe() {
        let status = Command::new(&rc_exe)
            .current_dir(&manifest_dir)
            .args([
                "/nologo",
                "/fo",
                res_file.to_str().unwrap(),
                generated_rc.to_str().unwrap(),
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
            generated_rc.to_str().unwrap(),
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
