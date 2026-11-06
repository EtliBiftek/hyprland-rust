//! Minimal `hyprctl` for hyprland-rust. Talks to `.socket.sock` using
//! Hyprland's wire format. Uses only std (no compositor modules).

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
};

const USAGE: &str = "\
usage: hyprctl [flags] <command> [args...]

commands:
    activewindow        Gets the active window and its properties
    activeworkspace     Gets the active workspace and its properties
    animations          Gets the configured animations and beziers
    binds               Lists all registered binds
    clients             Lists all windows with their properties
    configerrors        Lists all current config parsing errors
    cursorpos           Gets the current cursor position
    decorations <win>   Lists the decorations of a window
    devices             Lists all connected keyboards and mice
    dismissnotify [n]   Dismisses all or up to n notifications
    dispatch <name> [args]  Issues a dispatcher
    getoption <option>  Gets a config option and whether it is set
    globalshortcuts     Lists all global shortcuts
    instances           Lists all running instances
    keyword <name> <value>  Applies a config line at runtime
    kill                Enters kill mode
    layouts             Lists the available layouts
    monitors            Lists active outputs ('monitors all': all of them)
    notify <icon> <ms> <color> [fontsize:N] <text>  Sends a notification
    reload              Reloads the config file
    rollinglog          Prints the tail of the log
    seterror <color> <message>|disable  Sets the hyprctl error string
    splash              Gets the current splash
    switchxkblayout <kb|main|all> <next|prev|index>  Switches the xkb layout
    systeminfo          Gets system info
    version             Prints the version
    workspaces          Lists all workspaces with their properties

flags:
    -j                  Output in JSON
    -r                  Refresh state after issuing the command
    -a                  Include unmapped windows / inactive monitors
    -c                  Include the config (systeminfo)
    --batch \"a; b\"      Executes a batch of commands, separated by ';'
    --instance, -i      Instance signature or index (see 'instances')
    --quiet, -q         Do not print the reply
    --help, -h          This text
";

/// Builds the request string: `[[BATCH]]` or `flags/` prefix + command.
/// `flags` are single letters (`j`, `r`, `a`, `c`).
fn build_request(flags: &str, batch: Option<&str>, words: &[String]) -> String {
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

struct Instance {
    id: String,
    time: u64,
    pid: u64,
    wl_socket: String,
}

/// Instances with a `hyprland.lock` (pid, then Wayland socket name) in
/// `$XDG_RUNTIME_DIR/hypr/<time-ish>_<unix time>_<n>/`, oldest first.
fn instances() -> Vec<Instance> {
    let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(PathBuf::from(rt).join("hypr")) else {
        return Vec::new();
    };
    let mut v: Vec<Instance> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let mut parts = id.split('_');
            let time = parts.nth(1)?.parse().ok()?;
            let lock = std::fs::read_to_string(e.path().join("hyprland.lock")).ok()?;
            let mut lines = lock.lines();
            let pid = lines.next()?.parse().ok()?;
            let wl_socket = lines.next()?.to_string();
            Some(Instance {
                id,
                time,
                pid,
                wl_socket,
            })
        })
        .collect();
    v.sort_by_key(|i| (i.time, i.id.clone()));
    v
}

fn instances_text(list: &[Instance], json: bool) -> String {
    if json {
        let items: Vec<String> = list
            .iter()
            .map(|i| {
                format!(
                    "\n{{\n    \"instance\": \"{}\",\n    \"time\": {},\n    \"pid\": {},\n    \"wl_socket\": \"{}\"\n}}",
                    i.id, i.time, i.pid, i.wl_socket
                )
            })
            .collect();
        format!("[{}\n]", items.join(","))
    } else {
        list.iter()
            .map(|i| {
                format!(
                    "instance {}:\n\ttime: {}\n\tpid: {}\n\twl socket: {}\n\n",
                    i.id, i.time, i.pid, i.wl_socket
                )
            })
            .collect()
    }
}

fn instance_dir(instance: Option<String>) -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let base = PathBuf::from(runtime).join("hypr");
    if let Some(sel) = instance {
        // A small number selects by index in `hyprctl instances`.
        if let Ok(n) = sel.parse::<usize>() {
            let list = instances();
            return list
                .get(n)
                .map(|i| base.join(&i.id))
                .ok_or_else(|| format!("instance index {n} out of range"));
        }
        return Ok(base.join(sel));
    }
    if let Ok(sig) = std::env::var("HYPRLAND_INSTANCE_SIGNATURE") {
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
    let mut flags = String::new();
    let mut quiet = false;
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
            "-j" | "-r" | "-a" | "-c" | "-f" => {
                let f = &a[1..];
                if !flags.contains(f) {
                    flags.push_str(f);
                }
            }
            "-q" | "--quiet" => quiet = true,
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
    if words.first().map(String::as_str) == Some("instances") {
        println!("{}", instances_text(&instances(), flags.contains('j')));
        return Ok(());
    }

    let sock = instance_dir(instance)?.join(".socket.sock");
    let mut s = UnixStream::connect(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    s.write_all(build_request(&flags, batch.as_deref(), &words).as_bytes())
        .map_err(|e| e.to_string())?;
    let mut reply = String::new();
    s.read_to_string(&mut reply).map_err(|e| e.to_string())?;
    if !quiet {
        println!("{}", reply.trim_end_matches('\n'));
    }
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
        assert_eq!(build_request("j", None, &w("clients")), "j/clients");
        assert_eq!(
            build_request("", None, &w("dispatch workspace 2")),
            "/dispatch workspace 2"
        );
        assert_eq!(build_request("", Some("a; b;"), &[]), "[[BATCH]]/a;/b");
        assert_eq!(build_request("jr", Some("a"), &[]), "[[BATCH]]jr/a");
    }

    #[test]
    fn instances_output() {
        let i = Instance {
            id: "ab_100_1".into(),
            time: 100,
            pid: 7,
            wl_socket: "wayland-1".into(),
        };
        assert_eq!(
            instances_text(&[i], false),
            "instance ab_100_1:\n\ttime: 100\n\tpid: 7\n\twl socket: wayland-1\n\n"
        );
        assert_eq!(instances_text(&[], true), "[\n]");
    }
}
