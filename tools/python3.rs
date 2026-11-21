use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

fn patch_driver(path: &Path) -> Result<PathBuf, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if !source.contains("pid = os.fork()") {
        return Ok(path.to_path_buf());
    }
    let start = source
        .find("def isolated(functions, c):")
        .ok_or_else(|| "missing isolated function".to_string())?;
    let end = source
        .find("\n\ndef cases_of(path):")
        .ok_or_else(|| "missing cases_of function".to_string())?;
    let direct = r#"def isolated(functions, c):
    """Windows shim for scalar functions without C crash paths."""
    try:
        return call(functions, c)
    except Exception as e:
        return {"error": "%s: %s" % (type(e).__name__, e)}
"#;
    let patched = format!("{}{}{}", &source[..start], direct, &source[end..]);
    let output = path.with_file_name("driver_c_windows.py");
    fs::write(&output, patched).map_err(|error| error.to_string())?;
    Ok(output)
}

fn main() {
    let mut args: Vec<OsString> = env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| {
        Path::new(arg).file_name() == Some(OsStr::new("driver_c.py"))
    }) {
        match patch_driver(Path::new(&args[0])) {
            Ok(path) => args[0] = path.into_os_string(),
            Err(error) => {
                eprintln!("python3 shim: failed to adapt C driver: {error}");
                process::exit(1);
            }
        }
    }

    let status = Command::new("python.exe").args(args).status();
    match status {
        Ok(status) => process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("python3 shim: failed to run python.exe: {error}");
            process::exit(1);
        }
    }
}
