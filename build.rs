use std::{env, path::PathBuf, process::Command};

fn resource_compiler() -> PathBuf {
    if let Some(compiler) = env::var_os("RC") {
        return compiler.into();
    }
    if Command::new("rc.exe").arg("/?").output().is_ok() {
        return "rc.exe".into();
    }
    let programs =
        env::var_os("ProgramFiles(x86)").unwrap_or_else(|| "C:/Program Files (x86)".into());
    let kits = PathBuf::from(programs).join("Windows Kits/10/bin");
    let mut versions: Vec<_> = std::fs::read_dir(&kits)
        .expect("Install the Windows SDK from Visual Studio C++ Build Tools")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("10."))
        .map(|entry| entry.path())
        .collect();
    versions.sort();
    versions
        .into_iter()
        .rev()
        .map(|version| version.join("x64/rc.exe"))
        .find(|compiler| compiler.is_file())
        .expect("Windows SDK rc.exe was not found; set RC to its full path")
}

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/dotape.ico");
    println!("cargo:rerun-if-env-changed=RC");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let icon =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/icon/dotape.ico");
    let script = output.join("dotape.rc");
    let resource = output.join("dotape.res");
    let icon_path = icon.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(&script, format!("1 ICON \"{icon_path}\"\n")).unwrap();
    let result = Command::new(resource_compiler())
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg(&script)
        .output()
        .expect("Could not start the Windows resource compiler");
    assert!(
        result.status.success(),
        "Icon resource compilation failed: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("cargo:rustc-link-arg-bin=DOTAPE={}", resource.display());
}
