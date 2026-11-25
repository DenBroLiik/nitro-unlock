//! Native Linux per-game Intel RAPL PSys boost for Acer Nitro Lite 16.
use std::{
    collections::BTreeMap,
    env, fs,
    ffi::OsString,
    fs::File,
    io::{self, IsTerminal, Read, Write},
    os::{fd::AsRawFd, raw::c_int, unix::process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
};

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("nitro-unlock supports Linux x86_64 only");

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const STOCK: u64 = 50_000_000;
const BOOST: u64 = 70_000_000;
const TEMP_LIMIT: u64 = 85_000;
const DRAIN_LIMIT: u64 = 5_000_000;
const READY: &str = "NITRO_UNLOCK_READY";
const PROGRAM: &str = "/usr/bin/nitro-unlock";
const SIG_HUP: c_int = 1;
const SIG_INT: c_int = 2;
const SIG_TERM: c_int = 15;
static STOP: AtomicI32 = AtomicI32::new(0);

// Linux x86_64 ABI. The FFI calls are confined to this section.
#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}
extern "C" {
    fn geteuid() -> u32;
    fn flock(fd: c_int, operation: c_int) -> c_int;
    fn poll(fds: *mut PollFd, nfds: usize, timeout: c_int) -> c_int;
    fn signal(sig: c_int, handler: usize) -> usize;
    fn kill(pid: c_int, sig: c_int) -> c_int;
}
extern "C" fn on_signal(sig: c_int) {
    STOP.store(sig, Ordering::Relaxed);
}
fn install_signals() -> Result<()> {
    for sig in [SIG_HUP, SIG_INT, SIG_TERM] {
        // The handler only writes a lock-free atomic.
        if unsafe { signal(sig, on_signal as usize) } == usize::MAX {
            return Err(io::Error::last_os_error().into());
        }
    }
    Ok(())
}
fn stopped() -> c_int { STOP.load(Ordering::Relaxed) }
fn is_root() -> bool { unsafe { geteuid() == 0 } }
fn readable(fd: c_int, ms: c_int) -> Result<bool> {
    let mut entry = PollFd { fd, events: 1, revents: 0 };
    // POLLHUP and POLLERR are returned even if not requested.
    let rc = unsafe { poll(&mut entry, 1, ms) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted { return Ok(false); }
        return Err(err.into());
    }
    Ok(rc > 0)
}
fn log(msg: impl std::fmt::Display) { eprintln!("nitro-unlock: {msg}"); }
fn read(path: &Path) -> Result<String> {
    Ok(fs::read_to_string(path)?.trim().to_owned())
}
fn number(path: &Path) -> Result<u64> { Ok(read(path)?.parse()?) }
fn write_limit(path: &Path, value: u64) -> Result<()> {
    fs::write(path, format!("{value}\n"))?;
    if number(path)? != value {
        return Err(format!("kernel did not retain {} = {value}", path.display()).into());
    }
    Ok(())
}

struct Hardware {
    model: PathBuf,
    powercap: PathBuf,
    supply: PathBuf,
    rating: PathBuf,
    hwmon: PathBuf,
}
impl Hardware {
    fn system() -> Self {
        Self {
            model: "/sys/devices/virtual/dmi/id/product_name".into(),
            powercap: "/sys/class/powercap".into(),
            supply: "/sys/class/power_supply".into(),
            rating: "/sys/bus/platform/devices/INTC10A4:00/dptf_power/adapter_rating_mw".into(),
            hwmon: "/sys/devices/platform/coretemp.0/hwmon".into(),
        }
    }
    fn ac(&self) -> PathBuf { self.supply.join("ADP1/online") }
    fn battery(&self, name: &str) -> PathBuf { self.supply.join("BAT0").join(name) }
    fn zone(&self) -> Result<PathBuf> {
        if read(&self.model)? != "Nitro NL16-71G" {
            return Err("unsupported model (expected Nitro NL16-71G)".into());
        }
        for entry in fs::read_dir(&self.powercap)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name.strip_prefix("intel-rapl:") {
                if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
                    && read(&entry.path().join("name"))? == "psys" {
                    return Ok(entry.path());
                }
            }
        }
        Err("Intel RAPL PSys is unavailable".into())
    }
    fn temperature(&self) -> Result<PathBuf> {
        for dir in fs::read_dir(&self.hwmon)? {
            let dir = dir?;
            if !dir.file_name().to_string_lossy().starts_with("hwmon") { continue; }
            for file in fs::read_dir(dir.path())? {
                let file = file?;
                let name = file.file_name();
                let name = name.to_string_lossy();
                if let Some(base) = name.strip_suffix("_label") {
                    if base.starts_with("temp") && read(&file.path())? == "Package id 0" {
                        return Ok(dir.path().join(format!("{base}_input")));
                    }
                }
            }
        }
        Err("CPU package temperature sensor unavailable".into())
    }
}
fn constraints(zone: &Path) -> Result<[PathBuf; 2]> {
    let mut paths = BTreeMap::new();
    for item in fs::read_dir(zone)? {
        let item = item?;
        let name = item.file_name();
        let name = name.to_string_lossy();
        if let Some(base) = name.strip_suffix("_name") {
            if base.starts_with("constraint_") {
                let key = read(&item.path())?;
                if paths.insert(key, zone.join(format!("{base}_power_limit_uw"))).is_some() {
                    return Err("duplicate PSys constraint".into());
                }
            }
        }
    }
    Ok([
        paths.remove("short_term").ok_or("missing PSys PL2")?,
        paths.remove("long_term").ok_or("missing PSys PL1")?,
    ])
}

struct Lease {
    original: [(PathBuf, u64); 2],
    applied: u64,
    armed: bool,
    _lock: File,
}
impl Lease {
    fn new(zone: &Path, paths: [PathBuf; 2], target: u64, normalize: bool) -> Result<Self> {
        let lock = File::open(zone.join("name"))?;
        // Linux LOCK_EX | LOCK_NB; lock stays held during rollback.
        if unsafe { flock(lock.as_raw_fd(), 2 | 4) } != 0 {
            return Err(format!("cannot acquire PSys lock: {}", io::Error::last_os_error()).into());
        }
        let snapshot = |p: PathBuf| -> Result<(PathBuf, u64)> {
            let n = number(&p)?;
            Ok((p, if normalize && n == BOOST { STOCK } else { n }))
        };
        let [short, long] = paths;
        Ok(Self { original: [snapshot(short)?, snapshot(long)?], applied: target, armed: true, _lock: lock })
    }
    fn apply(&self) -> Result<()> {
        // On boost PL2 goes first; on a reduction PL1 goes first.
        let order = if self.applied == STOCK { [1, 0] } else { [0, 1] };
        for index in order {
            let (path, _) = &self.original[index];
            if number(path)? != self.applied { write_limit(path, self.applied)?; }
        }
        Ok(())
    }
    fn restore(&mut self) -> Result<()> {
        if !self.armed { return Ok(()); }
        let mut errors = Vec::new();
        let order = if self.applied == STOCK { [0, 1] } else { [1, 0] };
        for index in order {
            let (path, original) = &self.original[index];
            let result = (|| -> Result<()> {
                let current = number(path)?;
                if current == self.applied && current != *original {
                    write_limit(path, *original)?;
                } else if current != *original {
                    log(format!("{} changed externally; not overwriting", path.display()));
                }
                Ok(())
            })();
            if let Err(e) = result { errors.push(format!("{}: {e}", path.display())); }
        }
        if errors.is_empty() { self.armed = false; Ok(()) }
        else { Err(format!("restore failed: {}", errors.join("; ")).into()) }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Err(e) = self.restore() { log(e); }
    }
}
fn check_power(hw: &Hardware, temp: &Path) -> Result<()> {
    if number(&hw.ac())? != 1 { return Err("AC adapter disconnected".into()); }
    if number(temp)? >= TEMP_LIMIT { return Err("CPU package reached 85 C".into()); }
    Ok(())
}
fn battery_drain(hw: &Hardware) -> Result<bool> {
    let state = read(&hw.battery("status"))?;
    match state.as_str() {
        "Discharging" => Ok(number(&hw.battery("power_now"))? >= DRAIN_LIMIT),
        "Charging" | "Full" | "Not charging" => Ok(false),
        _ => Err(format!("unknown battery state: {state}").into()),
    }
}
fn worker(hw: &Hardware, stock: bool, fd: c_int) -> Result<()> {
    let zone = hw.zone()?;
    let paths = constraints(&zone)?;
    if stock {
        let mut lease = Lease::new(&zone, paths, STOCK, false)?;
        lease.apply()?;
        lease.armed = false;
        println!("{READY}");
        io::stdout().flush()?;
        return Ok(());
    }
    if number(&zone.join("enabled"))? != 1 || number(&hw.rating)? < 100_000 {
        return Err("active PSys and ACPI adapter rating >=100 W required".into());
    }
    let temp = hw.temperature()?;
    check_power(hw, &temp)?;
    if battery_drain(hw)? { return Err("battery already discharging at >=5 W".into()); }
    if readable(fd, 0)? || stopped() != 0 { return Err("launch canceled".into()); }
    let mut lease = Lease::new(&zone, paths, BOOST, true)?;
    let outcome = (|| -> Result<()> {
        // Recheck after acquiring the lock, before any mutation.
        check_power(hw, &temp)?;
        if battery_drain(hw)? || readable(fd, 0)? || stopped() != 0 {
            return Err("boost canceled before changing PSys".into());
        }
        lease.apply()?;
        if stopped() != 0 || readable(fd, 0)? { return Ok(()); }
        println!("{READY}");
        io::stdout().flush()?;
        log("PSys PL1/PL2 boosted to 70 W");
        let mut drain_since: Option<Instant> = None;
        while stopped() == 0 && !readable(fd, 500)? {
            check_power(hw, &temp)?;
            if battery_drain(hw)? {
                if drain_since.get_or_insert_with(Instant::now).elapsed() >= Duration::from_secs(3) {
                    return Err("battery discharging at >=5 W for 3 seconds".into());
                }
            } else { drain_since = None; }
            for (path, _) in &lease.original {
                if number(path)? != BOOST { return Err("PSys changed externally".into()); }
            }
        }
        Ok(())
    })();
    lease.restore()?;
    outcome
}

fn status(hw: &Hardware) -> Result<()> {
    let [short, long] = constraints(&hw.zone()?)?;
    println!("PSys PL1: {} W\nPSys PL2: {} W", number(&long)? as f64 / 1e6, number(&short)? as f64 / 1e6);
    println!("AC: {}\nAC adapter rating (ACPI): {} W",
        if number(&hw.ac())? == 1 { "online" } else { "offline" }, number(&hw.rating)? as f64 / 1e3);
    println!("Battery: {}%, {}, {} W", read(&hw.battery("capacity"))?,
        read(&hw.battery("status"))?, number(&hw.battery("power_now"))? as f64 / 1e6);
    Ok(())
}
fn auth(stock: bool) -> Result<Child> {
    // Never elevate a binary from a user-writable checkout.
    let mut cmd = Command::new("/usr/bin/pkexec");
    if !io::stdin().is_terminal() && !io::stderr().is_terminal() {
        if Path::new("/usr/lib/systemd/user/plasma-polkit-agent.service").is_file() &&
            (env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("DISPLAY").is_some()) {
            let _ = Command::new("/usr/bin/systemctl")
                .args(["--user", "start", "--no-block", "plasma-polkit-agent.service"])
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        cmd.arg("--disable-internal-agent");
    }
    cmd.arg(PROGRAM).arg(if stock { "--_restore" } else { "--_worker" });
    for key in ["LD_PRELOAD", "LD_LIBRARY_PATH", "LD_AUDIT"] { cmd.env_remove(key); }
    Ok(cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?)
}
fn wait_auth(child: &mut Child) -> Result<bool> {
    let mut stdout = child.stdout.take().ok_or("missing worker stdout")?;
    let mut line = Vec::new();
    let mut buffer = [0u8; 256];
    while stopped() == 0 {
        if !readable(stdout.as_raw_fd(), 200)? { continue; }
        let count = stdout.read(&mut buffer)?;
        if count == 0 { return Ok(false); }
        for &byte in &buffer[..count] {
            if byte == b'\n' {
                if line == READY.as_bytes() { return Ok(true); }
                log(String::from_utf8_lossy(&line));
                line.clear();
            } else if line.len() < 4096 { line.push(byte); }
            else { return Err("unexpected worker output".into()); }
        }
    }
    Ok(false)
}
fn exit_code(s: std::process::ExitStatus) -> i32 { s.code().unwrap_or(128 + s.signal().unwrap_or(1)) }
fn close_worker(mut child: Child) -> Result<i32> {
    drop(child.stdin.take());
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(s) = child.try_wait()? { return Ok(exit_code(s)); }
        if Instant::now() >= until { return Err("worker did not confirm rollback; check --status".into()); }
        thread::sleep(Duration::from_millis(50));
    }
}
fn launch(args: &[OsString], stock: bool) -> Result<i32> {
    let mut helper = match auth(stock) {
        Ok(c) => Some(c),
        Err(e) if !stock => { log(format!("authorization unavailable: {e}; launching without boost")); None }
        Err(e) => return Err(e),
    };
    let mut boosted = false;
    if let Some(c) = helper.as_mut() {
        match wait_auth(c) {
            Ok(ready) => boosted = ready,
            Err(e) => log(e),
        }
        if !boosted { drop(c.stdin.take()); }
    }
    let result = (|| -> Result<i32> {
        if stopped() != 0 { return Ok(128 + stopped()); }
        if stock { return Ok(if boosted { 0 } else { 1 }); }
        if !boosted { log("boost unavailable; launching game normally"); }
        let mut game = Command::new(&args[0]).args(&args[1..]).spawn()?;
        let mut forwarded = false;
        loop {
            if stopped() != 0 && !forwarded {
                // Only forward to our own child; never run the game as root.
                if game.try_wait()?.is_none() { unsafe { kill(game.id() as c_int, stopped()); } }
                if let Some(c) = helper.as_mut() { drop(c.stdin.take()); }
                forwarded = true;
            }
            if let Some(s) = game.try_wait()? { return Ok(exit_code(s)); }
            thread::sleep(Duration::from_millis(50));
        }
    })();
    let cleanup = match helper { Some(c) => close_worker(c), None => Ok(0) };
    let game = result?;
    match cleanup {
        Ok(code) => Ok(if game != 0 { game } else if boosted { code } else { 0 }),
        Err(e) => { log(e); Ok(if game != 0 { game } else { 1 }) }
    }
}
fn run() -> Result<i32> {
    let mut args: Vec<_> = env::args_os().skip(1).collect();
    let first = args.first().and_then(|a| a.to_str()).unwrap_or("");
    if args.is_empty() || (args.len() == 1 && matches!(first, "--help" | "-h")) {
        println!("nitro-unlock [--] GAME [ARGUMENT ...]\nnitro-unlock --status\nnitro-unlock --restore-stock\nnitro-unlock --version");
        return Ok(0);
    }
    if first == "--version" && args.len() == 1 {
        println!("nitro-unlock {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    install_signals()?;
    let hw = Hardware::system();
    if matches!(first, "--_worker" | "--_restore") && args.len() == 1 {
        if !is_root() || env::var_os("PKEXEC_UID").is_none() {
            return Err("system operation requires Polkit authorization".into());
        }
        worker(&hw, first == "--_restore", 0)?;
        return Ok(0);
    }
    if is_root() { return Err("run the wrapper as a normal user".into()); }
    if first == "--status" && args.len() == 1 { status(&hw)?; return Ok(0); }
    if first == "--restore-stock" && args.len() == 1 { return launch(&[], true); }
    if first == "--" { args.remove(0); }
    if args.is_empty() || args[0].to_string_lossy().starts_with('-') {
        return Err("specify a game or --help".into());
    }
    launch(&args, false)
}
fn main() {
    let code = match run() { Ok(c) => c, Err(e) => { log(e); 1 } };
    std::process::exit(code);
}

#[cfg(test)]
mod tests;
