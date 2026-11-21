use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

fn patch_bench(path: &Path) -> Result<PathBuf, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if !source.contains("CLOCK_MONOTONIC") {
        return Ok(path.to_path_buf());
    }
    let source = source.replace("#include <time.h>", "#include <time.h>\n#include <windows.h>");
    let source = source.replace(
        "static double now_ns(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1e9 + t.tv_nsec; }",
        "static double now_ns(void) { LARGE_INTEGER frequency, counter; QueryPerformanceFrequency(&frequency); QueryPerformanceCounter(&counter); return (double)counter.QuadPart * 1e9 / (double)frequency.QuadPart; }",
    );
    let output = path.with_file_name("bench_windows.c");
    fs::write(&output, source).map_err(|error| error.to_string())?;
    Ok(output)
}

fn main() {
    let mut cl_args = Vec::new();
    let mut args = env::args_os().skip(1);

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--version") => {
                println!("cc shim using MSVC cl");
                return;
            }
            Some("-shared") => cl_args.push(OsString::from("/LD")),
            Some("-fPIC" | "-lm") => {}
            Some("-O1") => cl_args.push(OsString::from("/O1")),
            Some("-O2") => cl_args.push(OsString::from("/O2")),
            Some("-w") => cl_args.push(OsString::from("/W0")),
            Some("-o") => {
                let Some(output) = args.next() else {
                    eprintln!("cc shim: -o requires a path");
                    process::exit(2);
                };
                let mut flag = OsString::from("/Fe");
                flag.push(output);
                cl_args.push(flag);
            }
            Some(value) if value.starts_with("-I") || value.starts_with("-D") => {
                let mut flag = OsString::from("/");
                flag.push(&value[1..]);
                cl_args.push(flag);
            }
            Some(value) if value.starts_with("-Wl,") => {}
            Some(value) if value.starts_with('-') => {
                eprintln!("cc shim: unsupported option {value}");
                process::exit(2);
            }
            _ if Path::new(&arg).file_name() == Some(OsStr::new("bench.c")) => {
                match patch_bench(Path::new(&arg)) {
                    Ok(path) => cl_args.push(path.into_os_string()),
                    Err(error) => {
                        eprintln!("cc shim: failed to adapt benchmark: {error}");
                        process::exit(2);
                    }
                }
            }
            _ if Path::new(&arg).extension().is_some_and(|ext| ext == "dll") => {
                let mut import_library = PathBuf::from(arg);
                import_library.set_extension("dll.lib");
                cl_args.push(import_library.into_os_string());
            }
            _ => cl_args.push(arg),
        }
    }

    let output = Command::new("cl.exe")
        .arg("/nologo")
        .args(cl_args)
        .output();
    match output {
        Ok(output) => {
            if !output.status.success() {
                eprintln!("cl stdout:\n{}", String::from_utf8_lossy(&output.stdout));
                eprintln!("cl stderr:\n{}", String::from_utf8_lossy(&output.stderr));
            }
            process::exit(output.status.code().unwrap_or(1));
        }
        Err(error) => {
            eprintln!("cc shim: failed to run cl.exe: {error}");
            process::exit(1);
        }
    }
}
