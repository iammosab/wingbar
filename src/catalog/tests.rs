use super::*;
use crate::config::parse::{self, valid_key};
use crate::config::{Options, WidgetType, defaults};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

// --- definitions ---------------------------------------------------------------

/// A fresh install's settings with `def` added as `key` on `wing`.
fn with_widget(key: &str, def: Value, wing: Wing) -> Value {
    let mut doc = defaults::fresh(false);
    doc["widgets"][key] = def;
    doc[wing.name()].as_array_mut().unwrap().push(json!(key));
    doc
}

#[test]
fn every_definition_reads_without_warnings() {
    for e in entries() {
        for def in [e.definition(), e.definition_for(Path::new("/opt/wingbar widgets/x.sh"))] {
            let doc = with_widget(e.name, def, e.wing);
            let (cfg, warnings) = parse::parse(&doc, &|_: &Path| false);
            assert!(warnings.is_empty(), "{}: {warnings:?}", e.name);
            assert_eq!(cfg.wing_of(e.name), Some(e.wing), "{}", e.name);
            let Options::Script(s) = &cfg.def(e.name).unwrap().options else {
                panic!("{} is not a script widget", e.name)
            };
            assert!(s.command.starts_with("bash "), "{}: {}", e.name, s.command);
            assert!(s.icon.is_some(), "{} has no icon", e.name);
            assert!(!s.levels.is_empty(), "{} has no levels", e.name);
        }
        // and with every default colour gone from theme.colors
        let mut doc = with_widget(e.name, e.definition(), e.wing);
        doc["theme"]["colors"] = json!({});
        assert!(parse::parse(&doc, &|_: &Path| false).1.is_empty(), "{}", e.name);
    }
}

#[test]
fn names_are_keys_and_not_built_in_types() {
    let mut seen = Vec::new();
    for e in entries() {
        assert!(valid_key(e.name), "{}", e.name);
        assert!(WidgetType::parse(e.name).is_none(), "{} is also a built-in type", e.name);
        assert!(!seen.contains(&e.name), "{} twice", e.name);
        seen.push(e.name);
        assert!(get(e.name).is_some());
        assert!(!e.description.is_empty() && !e.description.contains('\n'));
    }
    assert_eq!(seen, ["storage", "network", "mic", "adb", "aerospace-mode", "git-branch"]);
    assert!(get("nope").is_none());
}

#[test]
fn commands_name_the_script() {
    let mic = get("mic").unwrap();
    let d = mic.definition();
    assert_eq!(d["command"], "bash ~/.config/wingbar/widgets/mic.sh");
    assert_eq!(d["click"], "bash ~/.config/wingbar/widgets/mic.sh toggle");
    assert_eq!(d["iconOnly"], true);

    let home = crate::config::home();
    let d = mic.definition_for(&home.join(".config/wingbar/widgets/mic.sh"));
    assert_eq!(d["command"], "bash ~/.config/wingbar/widgets/mic.sh");
    let d = mic.definition_for(Path::new("/tmp/a b/mic.sh"));
    assert_eq!(d["command"], "bash '/tmp/a b/mic.sh'");
    assert_eq!(d["click"], "bash '/tmp/a b/mic.sh' toggle");
    let d = mic.definition_for(&home.join("it's/mic.sh"));
    assert_eq!(d["command"], format!("bash '{}/it'\\''s/mic.sh'", home.display()));
}

/// The header says what it shows, its levels, what it needs, and an
/// interval that is the definition's `every`.
#[test]
fn scripts_have_a_header_matching_their_definition() {
    for e in entries() {
        assert!(e.script.starts_with("#!/bin/bash\n"), "{}", e.name);
        assert!(e.script.contains(&format!("# wingbar catalog widget: {}\n", e.name)), "{}", e.name);
        assert!(e.script.contains("# Levels"), "{}", e.name);
        assert!(e.script.contains("# Needs:"), "{}", e.name);
        let interval = e
            .script
            .lines()
            .find_map(|l| l.strip_prefix("# Suggested interval: "))
            .unwrap_or_else(|| panic!("{}: no suggested interval", e.name))
            .split(['.', ' '])
            .next()
            .unwrap();
        assert_eq!(e.definition()["every"], interval, "{}", e.name);
    }
}

// --- install -------------------------------------------------------------------

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wingbar-catalog-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn install_writes_once_and_keeps_an_existing_file() {
    let d = temp_dir("install");
    let dir = d.join("widgets");
    let first = install("storage", &dir).unwrap();
    assert_eq!(first, Installed { path: dir.join("storage.sh"), written: true });
    assert!(first.message().starts_with("wrote "));
    assert_eq!(std::fs::read_to_string(&first.path).unwrap(), get("storage").unwrap().script);
    let mode = std::fs::metadata(&first.path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);

    std::fs::write(&first.path, "echo mine\n").unwrap();
    let again = install("storage", &dir).unwrap();
    assert_eq!(again, Installed { path: dir.join("storage.sh"), written: false });
    assert!(again.message().starts_with("kept the existing "));
    assert_eq!(std::fs::read_to_string(&first.path).unwrap(), "echo mine\n");

    // a link that points nowhere is kept as well
    std::os::unix::fs::symlink(d.join("missing"), dir.join("mic.sh")).unwrap();
    assert!(!install("mic", &dir).unwrap().written);
    assert!(!d.join("missing").exists());

    let err = install("nope", &dir).unwrap_err();
    assert!(err.contains("storage, network"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn installed_scripts_run_from_their_definition() {
    let d = temp_dir("run");
    let e = get("aerospace-mode").unwrap();
    let path = install(e.name, &d.join("widgets")).unwrap().path;
    let def = e.definition_for(&path);
    // no aerospace on this PATH: the widget hides (an empty line 1)
    let out = Command::new("/bin/bash")
        .arg("-c")
        .arg(def["command"].as_str().unwrap())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &d)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "\n");
    let _ = std::fs::remove_dir_all(&d);
}

// --- the scripts, against fake commands ----------------------------------------

/// A folder with the script, fake commands, and a home of its own, run with
/// only /usr/bin and /bin on PATH: nothing of the machine's state (adb, Wi-Fi,
/// AeroSpace, the disk) reaches the script.
///
/// The fakes are shell functions defined before the script runs (a function
/// wins over a command on PATH, and `command -v` finds it): macOS checks
/// every new executable file the first time it runs, which costs ~0.3 s each.
struct Sandbox {
    dir: PathBuf,
    script: PathBuf,
    fakes: PathBuf,
    env: Vec<(String, String)>,
}

impl Sandbox {
    fn new(name: &str, case: &str) -> Sandbox {
        let dir = temp_dir(&format!("{name}-{case}"));
        for sub in ["home", "tmp"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        let script = dir.join(format!("{name}.sh"));
        std::fs::write(&script, get(name).unwrap().script).unwrap();
        let fakes = dir.join("fakes.sh");
        std::fs::write(&fakes, "").unwrap();
        Sandbox { dir, script, fakes, env: Vec::new() }
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    /// A fake command: `body` is bash, run in a subshell with the command's
    /// arguments (so `exit` ends only the fake).
    fn fake(&self, cmd: &str, body: &str) -> &Sandbox {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.fakes).unwrap();
        write!(f, "{cmd}() (\n{body}\n)\n").unwrap();
        self
    }

    /// A fake command as an executable file at `path` (for scripts that look
    /// for a file, not a command).
    fn fake_at(&self, path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("#!/bin/bash\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn env(mut self, k: &str, v: &str) -> Sandbox {
        self.env.push((k.into(), v.into()));
        self
    }

    /// Runs the script with /bin/bash (3.2 on macOS), after the fakes; it
    /// must succeed and say nothing on stderr. Returns stdout.
    fn run(&self, args: &[&str]) -> String {
        let out = Command::new("/bin/bash")
            .arg("-c")
            .arg(". \"$WINGBAR_TEST_FAKES\"; . \"$WINGBAR_TEST_SCRIPT\"")
            .arg("bash")
            .args(args)
            .env_clear()
            .env("WINGBAR_TEST_FAKES", &self.fakes)
            .env("WINGBAR_TEST_SCRIPT", &self.script)
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.home())
            .env("TMPDIR", self.dir.join("tmp"))
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{}: exit {:?}, stderr: {stderr}", self.script.display(), out.status);
        assert!(stderr.is_empty(), "{}: stderr: {stderr}", self.script.display());
        String::from_utf8(out.stdout).unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn lines(s: &[&str]) -> String {
    s.iter().map(|l| format!("{l}\n")).collect()
}

// storage

fn df(size: u64, avail: u64) -> String {
    let used = size - avail;
    let pct = (100 * used).div_ceil(size);
    format!(
        "echo \"$*\" > \"$HOME/df-args\"\n\
         echo 'Filesystem     1024-blocks      Used Available Capacity iused ifree %iused  Mounted on'\n\
         echo '/dev/disk3s5   {size} {used} {avail} {pct}% 1 2 1% /System/Volumes/Data'"
    )
}

fn storage(case: &str, size: u64, avail: u64) -> Sandbox {
    let s = Sandbox::new("storage", case);
    s.fake("df", &df(size, avail));
    s
}

const STORAGE_MENU: &[&str] = &[
    "menu: ---",
    "menu: Open Disk Utility | open -a 'Disk Utility'",
    "menu: Open Storage Settings | open 'x-apple.systempreferences:com.apple.settings.Storage'",
];

#[test]
fn storage_levels() {
    let menu = |first: &str, free: &str| {
        let mut l = vec![first.to_string(), format!("menu: {free}")];
        l.extend(STORAGE_MENU.iter().map(|s| s.to_string()));
        lines(&l.iter().map(String::as_str).collect::<Vec<_>>())
    };
    // 500 GB with 250 GB free (1024-byte blocks)
    let out = storage("half", 488_281_250, 244_140_625).run(&[]);
    assert_eq!(out, menu("ok|50%", "250.0 GB free of 500.0 GB"));
    assert_eq!(storage("84", 1_000_000, 160_000).run(&[]).lines().next(), Some("ok|84%"));
    assert_eq!(storage("85", 1_000_000, 150_000).run(&[]).lines().next(), Some("warn|85%"));
    assert_eq!(storage("94", 1_000_000, 60_000).run(&[]).lines().next(), Some("warn|94%"));
    assert_eq!(storage("95", 1_000_000, 50_000).run(&[]).lines().next(), Some("alert|95%"));
    assert_eq!(storage("full", 1_000_000, 0).run(&[]).lines().next(), Some("alert|100%"));
    assert_eq!(storage("empty", 1_000_000, 1_000_000).run(&[]).lines().next(), Some("ok|0%"));
}

#[test]
fn storage_settings() {
    let s = storage("env", 1_000_000, 400_000)
        .env("WINGBAR_STORAGE_WARN", "50")
        .env("WINGBAR_STORAGE_ALERT", "70")
        .env("WINGBAR_STORAGE_PATH", "/Volumes/Backup");
    assert_eq!(s.run(&[]).lines().next(), Some("warn|60%"));
    assert_eq!(std::fs::read_to_string(s.home().join("df-args")).unwrap(), "-k /Volumes/Backup\n");
    let s = storage("alert-env", 1_000_000, 200_000).env("WINGBAR_STORAGE_ALERT", "80");
    assert_eq!(s.run(&[]).lines().next(), Some("alert|80%"));
    // a bad threshold falls back to the default
    let s = storage("bad-env", 1_000_000, 100_000).env("WINGBAR_STORAGE_WARN", "lots");
    assert_eq!(s.run(&[]).lines().next(), Some("warn|90%"));
}

#[test]
fn storage_odd_df_output() {
    // a filesystem name with spaces, and a mount point with spaces
    let s = Sandbox::new("storage", "spaces");
    s.fake(
        "df",
        "echo 'Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on'\n\
         echo '//me@nas/My Share 1000000 300000 700000 30% 0 0 100% /Volumes/My Share'",
    );
    assert_eq!(s.run(&[]).lines().next(), Some("ok|30%"));
    // df fails, or prints nothing useful: hidden
    let s = Sandbox::new("storage", "fails");
    s.fake("df", "echo 'df: /nope: No such file or directory' >&2; exit 1");
    assert_eq!(s.run(&[]), "\n");
    let s = Sandbox::new("storage", "garbage");
    s.fake("df", "echo 'something else'");
    assert_eq!(s.run(&[]), "\n");
    // df missing
    let s = Sandbox::new("storage", "missing").env("WINGBAR_DF", "/nonexistent/df");
    assert_eq!(s.run(&[]), "\n");
}

// network

const PORTS: &str = "echo\n\
    echo 'Hardware Port: USB 10/100/1000 LAN'\necho 'Device: en9'\necho 'Ethernet Address: 00:11:22:33:44:55'\necho\n\
    echo 'Hardware Port: Wi-Fi'\necho 'Device: en0'\necho 'Ethernet Address: 00:11:22:33:44:66'\necho\n\
    echo 'Hardware Port: Thunderbolt Bridge'\necho 'Device: bridge0'\necho\n\
    echo 'VLAN Configurations'\necho '==================='";

/// Fakes for a default route through `iface` ("" for none), Wi-Fi called
/// `ssid` in `ipconfig getsummary`, `airport` from `networksetup
/// -getairportnetwork`, and these interfaces with an address.
fn network(case: &str, iface: &str, ssid: &str, airport: &str, addrs: &[(&str, &str)]) -> Sandbox {
    let s = Sandbox::new("network", case);
    if iface.is_empty() {
        s.fake("route", "echo 'route: writing to routing socket: not in table' >&2; exit 1");
    } else {
        s.fake(
            "route",
            &format!(
                "[ \"$*\" = '-n get default' ] || exit 2\n\
                 echo '   route to: default'\necho 'destination: default'\necho '    gateway: 192.168.1.1'\n\
                 echo '  interface: {iface}'\necho '      flags: <UP,GATEWAY,DONE,STATIC,PRCLONING,GLOBAL>'"
            ),
        );
    }
    s.fake(
        "networksetup",
        &format!(
            "case \"$1\" in\n\
             -listallhardwareports) {PORTS} ;;\n\
             -getairportnetwork) echo '{airport}' ;;\n\
             *) exit 1 ;;\n\
             esac"
        ),
    );
    let getifaddr: String = addrs.iter().map(|(i, a)| format!("{i}) echo {a} ;; ")).collect();
    s.fake(
        "ipconfig",
        &format!(
            "case \"$1\" in\n\
             getifaddr) case \"$2\" in {getifaddr} *) exit 1 ;; esac ;;\n\
             getsummary) [ \"$2\" = en0 ] || exit 1\n\
               echo '<dictionary> {{'\necho '  BSSID : <redacted>'\necho '  InterfaceType : WiFi'\n\
               echo '  IPv4 : <array> {{'\necho '    0 : <dictionary> {{'\necho '      SSID : nested'\necho '    }}'\necho '  }}'\n\
               echo '  SSID : {ssid}'\necho '}}' ;;\n\
             *) exit 1 ;;\n\
             esac"
        ),
    );
    s
}

const NOT_ASSOCIATED: &str = "You are not associated with an AirPort network.";
const NETWORK_SETTINGS: &str =
    "menu: Open Network Settings | open 'x-apple.systempreferences:com.apple.Network-Settings.extension'";
const WIFI_SETTINGS: &str =
    "menu: Open Wi-Fi Settings | open 'x-apple.systempreferences:com.apple.wifi-settings-extension'";

#[test]
fn network_wifi() {
    let out = network("wifi", "en0", "Home Net", NOT_ASSOCIATED, &[("en0", "192.168.1.20")]).run(&[]);
    assert_eq!(
        out,
        lines(&["ok|Home Net", "menu: Wi-Fi (en0): 192.168.1.20", "menu: ---", WIFI_SETTINGS, NETWORK_SETTINGS])
    );
    // the name hidden without Location permission
    let out = network("redacted", "en0", "<redacted>", NOT_ASSOCIATED, &[("en0", "192.168.1.20")]).run(&[]);
    assert_eq!(out.lines().next(), Some("ok|Wi-Fi"));
    // older macOS: getsummary has no SSID, networksetup knows it
    let out = network("older", "en0", "", "Current Wi-Fi Network: Café 5G", &[]).run(&[]);
    assert_eq!(out, lines(&["ok|Café 5G", "menu: Wi-Fi (en0)", "menu: ---", WIFI_SETTINGS, NETWORK_SETTINGS]));
    // a "|" in a network's name doesn't split the line
    let out = network("pipe", "en0", "a|b", NOT_ASSOCIATED, &[]).run(&[]);
    assert_eq!(out.lines().next(), Some("ok|ab"));
}

#[test]
fn network_ethernet_offline_vpn() {
    let out = network("eth", "en9", "Home Net", NOT_ASSOCIATED, &[("en9", "10.0.0.5"), ("en0", "10.0.0.6")]).run(&[]);
    assert_eq!(
        out,
        lines(&["eth|Ethernet", "menu: USB 10/100/1000 LAN (en9): 10.0.0.5", "menu: ---", NETWORK_SETTINGS])
    );

    let out = network("offline", "", "", NOT_ASSOCIATED, &[]).run(&[]);
    assert_eq!(out, lines(&["dim|Offline", "menu: No default route", "menu: ---", NETWORK_SETTINGS]));

    // a VPN over Wi-Fi names the Wi-Fi
    let out = network("vpn-wifi", "utun4", "Home Net", NOT_ASSOCIATED, &[("en0", "192.168.1.20")]).run(&[]);
    assert_eq!(
        out,
        lines(&[
            "ok|Home Net",
            "menu: Wi-Fi (en0): 192.168.1.20",
            "menu: VPN: utun4",
            "menu: ---",
            WIFI_SETTINGS,
            NETWORK_SETTINGS
        ])
    );
    // over a cable
    let out = network("vpn-eth", "utun4", "", NOT_ASSOCIATED, &[("en9", "10.0.0.5")]).run(&[]);
    assert_eq!(
        out.lines().take(3).collect::<Vec<_>>(),
        ["eth|Ethernet", "menu: USB 10/100/1000 LAN (en9): 10.0.0.5", "menu: VPN: utun4"]
    );
    // nothing to be found under it
    let out = network("vpn", "utun4", "", NOT_ASSOCIATED, &[]).run(&[]);
    assert_eq!(out, lines(&["ok|VPN", "menu: VPN: utun4", "menu: ---", NETWORK_SETTINGS]));
}

#[test]
fn network_tools_missing() {
    // no route command at all: offline
    let s = Sandbox::new("network", "no-route").env("WINGBAR_ROUTE", "/nonexistent/route");
    assert_eq!(s.run(&[]).lines().next(), Some("dim|Offline"));
    // only route: no hardware ports known, so it can't tell more than "VPN"
    let s = Sandbox::new("network", "only-route");
    s.fake("route", "echo '  interface: en0'");
    assert_eq!(s.run(&[]).lines().next(), Some("ok|VPN"));
}

// mic

/// A fake osascript with the input volume in $HOME/volume ("missing value"
/// works too) that logs the volumes it is told to set.
fn mic(case: &str, volume: &str) -> Sandbox {
    let s = Sandbox::new("mic", case);
    std::fs::write(s.home().join("volume"), format!("{volume}\n")).unwrap();
    s.fake(
        "osascript",
        "[ \"$1\" = -e ] || exit 1\n\
         case \"$2\" in\n\
         'input volume of (get volume settings)') cat \"$HOME/volume\" ;;\n\
         'set volume input volume '*) v=${2##* }; echo \"$v\" > \"$HOME/volume\"; echo \"$v\" >> \"$HOME/set\" ;;\n\
         *) exit 1 ;;\n\
         esac",
    );
    s
}

#[test]
fn mic_levels() {
    assert_eq!(mic("on", "40").run(&[]), lines(&["ok|On", "icon: mic"]));
    assert_eq!(mic("muted", "0").run(&[]), lines(&["alert|Muted", "icon: mic.slash"]));
    assert_eq!(mic("no-level", "missing value").run(&[]), lines(&["dim|No level", "icon: mic"]));
    let s = Sandbox::new("mic", "fails");
    s.fake("osascript", "echo 'execution error: no' >&2; exit 1");
    assert_eq!(s.run(&[]), "\n");
    let s = Sandbox::new("mic", "missing").env("WINGBAR_OSASCRIPT", "/nonexistent/osascript");
    assert_eq!(s.run(&[]), "\n");
}

#[test]
fn mic_toggle() {
    let s = mic("toggle", "40");
    let read = |f: &str| std::fs::read_to_string(s.home().join(f)).unwrap_or_default();
    assert_eq!(s.run(&["toggle"]), "");
    assert_eq!(read("volume"), "0\n");
    assert_eq!(s.run(&[]), lines(&["alert|Muted", "icon: mic.slash"]));
    assert_eq!(s.run(&["toggle"]), "");
    assert_eq!(read("volume"), "40\n");
    assert_eq!(read("set"), "0\n40\n");
    // unmuting with nothing saved: 75
    let s = mic("toggle-fresh", "0");
    s.run(&["toggle"]);
    assert_eq!(std::fs::read_to_string(s.home().join("volume")).unwrap(), "75\n");
    // no input volume: nothing to toggle
    let s = mic("toggle-none", "missing value");
    s.run(&["toggle"]);
    assert!(!s.home().join("set").exists());
}

// adb

fn adb_devices(listing: &str) -> String {
    let echoes: String = listing.lines().map(|l| format!("echo '{l}'\n")).collect();
    format!("[ \"$*\" = 'devices -l' ] || exit 1\n{echoes}")
}

const PIXEL: &str = "1A2B3C4D5E       device usb:1-1 product:panther model:Pixel_7 device:panther transport_id:3";
const UNAUTHORIZED: &str = "ZX1Y2Z3           unauthorized usb:1-2 transport_id:4";
const EMULATOR: &str =
    "emulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1";

#[test]
fn adb_devices_and_levels() {
    let s = Sandbox::new("adb", "one");
    s.fake("adb", &adb_devices(&format!("List of devices attached\n{PIXEL}\n")));
    assert_eq!(
        s.run(&[]),
        lines(&[
            "ok|Pixel 7",
            "menu: Pixel 7 · 1A2B3C4D5E",
            "menu: ---",
            "menu: Restart adb server | adb kill-server; adb start-server"
        ])
    );

    // ready devices first; one not ready warns
    let s = Sandbox::new("adb", "mixed");
    s.fake("adb", &adb_devices(&format!("List of devices attached\n{UNAUTHORIZED}\n{PIXEL}\n{EMULATOR}\n")));
    assert_eq!(
        s.run(&[]),
        lines(&[
            "warn|Pixel 7 +2",
            "menu: Pixel 7 · 1A2B3C4D5E",
            "menu: sdk gphone64 arm64 · emulator-5554",
            "menu: ZX1Y2Z3 · unauthorized",
            "menu: ---",
            "menu: Restart adb server | adb kill-server; adb start-server"
        ])
    );

    // only an unauthorized phone: its serial
    let s = Sandbox::new("adb", "unauthorized");
    s.fake("adb", &adb_devices(&format!("List of devices attached\n{UNAUTHORIZED}\n")));
    assert_eq!(s.run(&[]).lines().next(), Some("warn|ZX1Y2Z3"));
}

#[test]
fn adb_hidden_without_devices_or_adb() {
    // none attached (and the server starting up on the way)
    let s = Sandbox::new("adb", "none");
    s.fake(
        "adb",
        &adb_devices(
            "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\n",
        ),
    );
    assert_eq!(s.run(&[]), "\n");
    // adb is not installed
    assert_eq!(Sandbox::new("adb", "missing").run(&[]), "\n");
    let s = Sandbox::new("adb", "missing-env").env("WINGBAR_ADB", "/nonexistent/adb");
    assert_eq!(s.run(&[]), "\n");
    // adb fails
    let s = Sandbox::new("adb", "fails");
    s.fake("adb", "echo 'adb: failed to check server version' >&2; exit 1");
    assert_eq!(s.run(&[]), "\n");
}

#[test]
fn adb_found_in_the_android_sdk() {
    let s = Sandbox::new("adb", "sdk");
    let adb = s.home().join("Library/Android/sdk/platform-tools/adb");
    s.fake_at(&adb, &adb_devices(&format!("List of devices attached\n{PIXEL}\n")));
    let out = s.run(&[]);
    assert_eq!(out.lines().next(), Some("ok|Pixel 7"));
    assert!(out.contains(&format!("| {} kill-server", adb.display())), "{out}");

    let s = Sandbox::new("adb", "android-home");
    let sdk = s.dir.join("sdk");
    s.fake_at(&sdk.join("platform-tools/adb"), &adb_devices(&format!("List of devices attached\n{PIXEL}\n")));
    let s = s.env("ANDROID_HOME", &sdk.display().to_string());
    assert_eq!(s.run(&[]).lines().next(), Some("ok|Pixel 7"));
}

// aerospace-mode

fn aerospace(case: &str, body: &str) -> Sandbox {
    let s = Sandbox::new("aerospace-mode", case);
    s.fake("aerospace", body);
    s
}

#[test]
fn aerospace_mode() {
    let current = |mode: &str| format!("[ \"$*\" = 'list-modes --current' ] || exit 1\necho {mode}");
    assert_eq!(
        aerospace("resize", &current("resize")).run(&[]),
        lines(&["ok|resize", "menu: Back to main mode | aerospace mode main"])
    );
    assert_eq!(aerospace("main", &current("main")).run(&[]), "\n");
    // not running
    let s = aerospace("down", "echo \"Can't connect to AeroSpace server. Is AeroSpace.app running?\" >&2; exit 1");
    assert_eq!(s.run(&[]), "\n");
    // not installed
    assert_eq!(Sandbox::new("aerospace-mode", "missing").run(&[]), "\n");
    let s = aerospace("env", &current("service")).env("WINGBAR_AEROSPACE", "/nonexistent/aerospace");
    assert_eq!(s.run(&[]), "\n");
}

// git-branch

/// A fake git: a repository at $FAKE_TOP (none when empty) on branch
/// $FAKE_BRANCH (detached at $FAKE_SHA when empty), `status --porcelain`
/// printing $FAKE_STATUS, and "behind ahead" counts $FAKE_COUNTS (no
/// upstream when empty).
const FAKE_GIT: &str = r#"[ "$1" = --no-optional-locks ] && [ "$2" = -C ] || exit 2
[ "$3" = "$FAKE_DIR" ] || exit 2
shift 3
case "$*" in
"rev-parse --show-toplevel") [ -n "$FAKE_TOP" ] || { echo 'fatal: not a git repository' >&2; exit 128; }; echo "$FAKE_TOP" ;;
"symbolic-ref --short -q HEAD") [ -n "$FAKE_BRANCH" ] || exit 1; echo "$FAKE_BRANCH" ;;
"rev-parse --short HEAD") echo "$FAKE_SHA" ;;
"status --porcelain") [ -n "$FAKE_STATUS" ] && printf '%s\n' "$FAKE_STATUS" ;;
"rev-list --left-right --count @{upstream}...HEAD") [ -n "$FAKE_COUNTS" ] || { echo 'fatal: no upstream' >&2; exit 128; }; printf '%s\n' "$FAKE_COUNTS" ;;
*) exit 2 ;;
esac
exit 0"#;

fn git(case: &str, branch: &str, status: &str, counts: &str) -> Sandbox {
    let s = Sandbox::new("git-branch", case);
    s.fake("git", FAKE_GIT);
    s.env("WINGBAR_GIT_DIR", "/work/proj")
        .env("FAKE_DIR", "/work/proj")
        .env("FAKE_TOP", "/work/proj")
        .env("FAKE_BRANCH", branch)
        .env("FAKE_SHA", "abc1234")
        .env("FAKE_STATUS", status)
        .env("FAKE_COUNTS", counts)
}

const OPEN_PROJ: &[&str] =
    &["menu: ---", "menu: Open in Finder | open /work/proj", "menu: Open in Terminal | open -a Terminal /work/proj"];

#[test]
fn git_branch_levels() {
    let with_open = |l: &[&str]| {
        let mut v = l.to_vec();
        v.extend_from_slice(OPEN_PROJ);
        lines(&v)
    };
    assert_eq!(
        git("clean", "main", "", "1\t2").run(&[]),
        with_open(&["ok|main", "menu: proj", "menu: No changes", "menu: 2 ahead, 1 behind upstream"])
    );
    assert_eq!(
        git("dirty", "feature/x", " M src/a.rs\n?? new.txt", "").run(&[]),
        with_open(&["warn|feature/x", "menu: proj", "menu: 2 changed files"])
    );
    assert_eq!(
        git("one", "main", " M a", "0\t0").run(&[]),
        with_open(&["warn|main", "menu: proj", "menu: 1 changed file", "menu: 0 ahead, 0 behind upstream"])
    );
    assert_eq!(git("detached", "", "", "").run(&[]).lines().next(), Some("ok|abc1234"));
}

#[test]
fn git_branch_repository_choice() {
    // no repository named: hidden
    let s = Sandbox::new("git-branch", "unset");
    s.fake("git", FAKE_GIT);
    assert_eq!(s.run(&[]), "\n");
    // the argument wins over the variable, and ~ is the home folder
    let s = git("arg", "main", "", "");
    let home = s.home().display().to_string();
    let s = s.env("FAKE_DIR", &format!("{home}/code/proj")).env("FAKE_TOP", &format!("{home}/code/proj"));
    assert_eq!(s.run(&["~/code/proj"]).lines().next(), Some("ok|main"));
    // not a repository
    let s = git("not-repo", "main", "", "").env("FAKE_TOP", "");
    assert_eq!(s.run(&[]), lines(&["dim|not a repo", "menu: /work/proj is not a git repository"]));
    // git missing
    let s = git("missing", "main", "", "").env("WINGBAR_GIT", "/nonexistent/git");
    assert_eq!(s.run(&[]), "\n");
}

// --- text the scripts don't control never becomes a menu command -----------------

/// Every `menu:` line that runs a command is one of `allowed`; no other has a
/// `|` (the first `|` would start a command), and no line has a control character.
fn only_these_commands(out: &str, allowed: &[&str]) {
    for l in out.lines() {
        assert!(!l.chars().any(|c| c.is_control()), "a control character in {l:?}\n{out}");
        if l.starts_with("menu:") && l.contains('|') {
            assert!(allowed.contains(&l), "an unexpected command: {l:?}\n{out}");
        }
    }
}

/// What a hostile name could try: end the title, then run a command.
const EVIL: &str = "x|touch /tmp/wingbar-pwned";

#[test]
fn adb_device_names_cant_add_commands() {
    let restart = "menu: Restart adb server | adb kill-server; adb start-server";
    // a serial with a "|" and no model (the serial is the name), and a model with one
    let s = Sandbox::new("adb", "evil");
    s.fake(
        "adb",
        &adb_devices("List of devices attached\nx|reboot  unauthorized usb:1-2\na|b  device usb:1-1 model:Pi|xel_7\n"),
    );
    let out = s.run(&[]);
    only_these_commands(&out, &[restart]);
    assert_eq!(
        out,
        lines(&["warn|Pixel 7 +1", "menu: Pixel 7 · ab", "menu: xreboot · unauthorized", "menu: ---", restart])
    );
    // control characters (a carriage return) in a serial are dropped too
    let s = Sandbox::new("adb", "control");
    s.fake("adb", "echo 'List of devices attached'; printf 'ser\\001ial\\r  device\\n'");
    let out = s.run(&[]);
    only_these_commands(&out, &[restart]);
    assert_eq!(out.lines().nth(1), Some("menu: serial · serial"));
}

#[test]
fn network_names_cant_add_commands() {
    let allowed = [NETWORK_SETTINGS, WIFI_SETTINGS];
    // an SSID and an address with a "|" in them
    let addr = "'1.2.3.4|touch /tmp/wingbar-pwned'";
    let out = network("evil-ssid", "en0", EVIL, NOT_ASSOCIATED, &[("en0", addr)]).run(&[]);
    only_these_commands(&out, &allowed);
    assert_eq!(
        out.lines().take(2).collect::<Vec<_>>(),
        ["ok|xtouch /tmp/wingbar-pwned", "menu: Wi-Fi (en0): 1.2.3.4touch /tmp/wingbar-pwned"]
    );
    // an interface (a VPN's tunnel)
    let out = network("evil-vpn", &format!("utun{EVIL}"), "", NOT_ASSOCIATED, &[]).run(&[]);
    only_these_commands(&out, &allowed);
    assert_eq!(out.lines().nth(1), Some("menu: VPN: utunxtouch"), "{out}");
    // an older macOS's network name, with a tab
    let out = network("evil-older", "en0", "", &format!("Current Wi-Fi Network: a\tb{EVIL}"), &[]).run(&[]);
    only_these_commands(&out, &allowed);
    assert_eq!(out.lines().next(), Some("ok|abxtouch /tmp/wingbar-pwned"));
}

#[test]
fn git_names_cant_add_commands() {
    // a branch and a repository folder with a "|" in their names
    let top = "/work/a|b";
    let s = git("evil-branch", EVIL, "", "").env("WINGBAR_GIT_DIR", top).env("FAKE_DIR", top).env("FAKE_TOP", top);
    let out = s.run(&[]);
    only_these_commands(
        &out,
        &["menu: Open in Finder | open /work/a\\|b", "menu: Open in Terminal | open -a Terminal /work/a\\|b"],
    );
    assert_eq!(out.lines().take(2).collect::<Vec<_>>(), ["ok|xtouch /tmp/wingbar-pwned", "menu: ab"]);
    // a folder that isn't a repository, with a "|" and a line break in its name
    let dir = format!("/work/{EVIL}\nmenu: Evil | touch /tmp/wingbar-pwned");
    let s = git("evil-dir", "main", "", "").env("WINGBAR_GIT_DIR", &dir).env("FAKE_DIR", &dir).env("FAKE_TOP", "");
    let out = s.run(&[]);
    only_these_commands(&out, &[]);
    assert_eq!(out.lines().count(), 2, "{out}");
}

#[test]
fn aerospace_mode_names_cant_add_commands() {
    let back = "menu: Back to main mode | aerospace mode main";
    let out = aerospace("evil", &format!("printf '%s\\001\\n' '{EVIL}'")).run(&[]);
    only_these_commands(&out, &[back]);
    assert_eq!(out, lines(&["ok|xtouch /tmp/wingbar-pwned", back]));
}

#[test]
fn storage_and_mic_print_nothing_they_read_into_a_menu() {
    let s = Sandbox::new("storage", "evil");
    s.fake(
        "df",
        &format!(
            "echo 'Filesystem 1024-blocks Used Available Capacity iused ifree %iused Mounted on'\n\
             echo '{EVIL} 1000000 300000 700000 30% 0 0 100% /Volumes/{EVIL}'"
        ),
    );
    let out = s.run(&[]);
    only_these_commands(&out, STORAGE_MENU);
    assert_eq!(out.lines().next(), Some("ok|30%"));
    let out = mic("evil", EVIL).run(&[]);
    only_these_commands(&out, &[]);
    assert_eq!(out, lines(&["dim|No level", "icon: mic"]));
}
