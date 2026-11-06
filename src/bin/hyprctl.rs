//! Minimal `hyprctl` for hyprland-rust. Talks to `.socket.sock` using
//! Hyprland's wire format. Uses only std (no compositor modules).

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

const USAGE: &str = "\
Usage: hyprctl [-j] [-i <instance>] <command> [args...]
       hyprctl [-j] [-i <instance>] --batch \"cmd1; cmd2\"
";

/// Builds the request string: `[[BATCH]]` or `j/` or `/` prefix + command.
fn build_request(json: bool, batch: Option<&str>, words: &[String]) -> String {
    let flags = if json { "j" } else { "" };
    match batch {
        Some(b) => {
            let cmds: Vec<String> = b
                .split(';')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(|c| format!("{flags}/{c}"))
                .collect();
            format!("[[BATCH]]{}", cmds.join(";"))
        }
        None => format!("{flags}/{}", words.join(" ")),
    }
}

fn instance_dir(instance: Option<String>) -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let base = PathBuf::from(runtime).join("hypr");
    if let Some(sig) = instance.or_else(|| std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()) {
        return Ok(base.join(sig));
    }
    // Fallback: newest instance directory.
    std::fs::read_dir(&base)
        .map_err(|e| format!("cannot read {}: {e}", base.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join(".socket.sock").exists())
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
        .ok_or_else(|| "no Hyprland instance found (is HYPRLAND_INSTANCE_SIGNATURE set?)".into())
}

fn run() -> Result<(), String> {
    let mut json = false;
    let mut instance = None;
    let mut batch = None;
    let mut words = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        // Options are only recognised before the command word.
        if !words.is_empty() {
            words.push(a);
            continue;
        }
        match a.as_str() {
            "-j" => json = true,
            "-i" | "--instance" => instance = Some(args.next().ok_or("missing instance")?),
            "--batch" => batch = Some(args.next().ok_or("missing batch commands")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            _ => words.push(a),
        }
    }
    if words.is_empty() && batch.is_none() {
        return Err(USAGE.trim_end().to_string());
    }

    let sock = instance_dir(instance)?.join(".socket.sock");
    let mut s = UnixStream::connect(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    s.write_all(build_request(json, batch.as_deref(), &words).as_bytes())
        .map_err(|e| e.to_string())?;
    let mut reply = String::new();
    s.read_to_string(&mut reply).map_err(|e| e.to_string())?;
    println!("{}", reply.trim_end_matches('\n'));
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        let w = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(build_request(true, None, &w("clients")), "j/clients");
        assert_eq!(
            build_request(false, None, &w("dispatch workspace 2")),
            "/dispatch workspace 2"
        );
        assert_eq!(build_request(false, Some("a; b;"), &[]), "[[BATCH]]/a;/b");
    }
}
