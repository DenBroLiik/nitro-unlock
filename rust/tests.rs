use super::*;
use std::os::unix::net::UnixStream;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let p = env::temp_dir().join(format!("nitro-test-{}-{}", std::process::id(), ID.fetch_add(1, Ordering::Relaxed)));
            match fs::create_dir(&p) {
                Ok(()) => return Self(p),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("{e}"),
            }
        }
    }
    fn put(&self, p: &str, value: &str) {
        let p = self.0.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, value).unwrap();
    }
    fn fixture(&self) -> Hardware {
        for (p, v) in [
            ("model", "Nitro NL16-71G"), ("rating", "100000"),
            ("supply/ADP1/online", "1"), ("supply/BAT0/status", "Full"),
            ("supply/BAT0/power_now", "0"), ("hwmon/hwmon0/temp1_label", "Package id 0"),
            ("hwmon/hwmon0/temp1_input", "65000"), ("powercap/intel-rapl:1/name", "psys"),
            ("powercap/intel-rapl:1/enabled", "1"),
            ("powercap/intel-rapl:1/constraint_0_name", "long_term"),
            ("powercap/intel-rapl:1/constraint_1_name", "short_term"),
            ("powercap/intel-rapl:1/constraint_0_power_limit_uw", "50000000"),
            ("powercap/intel-rapl:1/constraint_1_power_limit_uw", "50000000"),
        ] { self.put(p, v); }
        Hardware { model: self.0.join("model"), rating: self.0.join("rating"),
            supply: self.0.join("supply"), hwmon: self.0.join("hwmon"), powercap: self.0.join("powercap") }
    }
}
impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn await_boost(paths: &[PathBuf; 2]) {
    let until = Instant::now() + Duration::from_secs(3);
    while !paths.iter().all(|p| number(p).unwrap_or(0) == BOOST) {
        assert!(Instant::now() < until, "worker never enabled boost");
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn restores_custom_limits_and_preserves_external_changes() {
    let tmp = Temp::new(); let hw = tmp.fixture();
    let zone = hw.zone().unwrap(); let [short, long] = constraints(&zone).unwrap();
    write_limit(&short, 53_000_000).unwrap();
    let mut lease = Lease::new(&zone, [short.clone(), long.clone()], BOOST, true).unwrap();
    lease.apply().unwrap();
    assert_eq!(number(&short).unwrap(), BOOST);
    assert!(Lease::new(&zone, [short.clone(), long.clone()], BOOST, true).is_err());
    write_limit(&long, 48_000_000).unwrap();
    lease.restore().unwrap();
    assert_eq!(number(&short).unwrap(), 53_000_000);
    assert_eq!(number(&long).unwrap(), 48_000_000);
}
#[test]
fn rollback_on_pipe_close_ac_loss_and_heat() {
    for reason in ["close", "ac", "heat"] {
        let tmp = Temp::new(); let hw = tmp.fixture();
        let paths = constraints(&hw.zone().unwrap()).unwrap();
        let (peer, fd) = UnixStream::pair().unwrap();
        let task = thread::spawn(move || worker(&hw, false, fd.as_raw_fd()));
        await_boost(&paths);
        // Ensure the worker has entered its monitoring loop.
        thread::sleep(Duration::from_millis(100));
        match reason {
            "ac" => tmp.put("supply/ADP1/online", "0"),
            "heat" => tmp.put("hwmon/hwmon0/temp1_input", "85000"),
            _ => drop(peer),
        }
        let result = task.join().unwrap();
        assert_eq!(result.is_err(), reason != "close");
        assert!(paths.iter().all(|p| number(p).unwrap() == STOCK));
    }
}
#[test]
fn rejects_unsafe_conditions() {
    let tmp = Temp::new(); let hw = tmp.fixture();
    let (_peer, fd) = UnixStream::pair().unwrap();
    tmp.put("hwmon/hwmon0/temp1_input", "85000");
    assert!(worker(&hw, false, fd.as_raw_fd()).is_err());
    tmp.put("hwmon/hwmon0/temp1_input", "65000");
    tmp.put("supply/BAT0/status", "Discharging");
    tmp.put("supply/BAT0/power_now", "6000000");
    assert!(worker(&hw, false, fd.as_raw_fd()).is_err());
    tmp.put("supply/BAT0/status", "Full");
    tmp.put("rating", "65000");
    assert!(worker(&hw, false, fd.as_raw_fd()).is_err());
    assert!(constraints(&hw.zone().unwrap()).unwrap().iter().all(|p| number(p).unwrap() == STOCK));
}
#[test]
fn sustained_discharge_stops_boost() {
    let tmp = Temp::new(); let hw = tmp.fixture();
    let paths = constraints(&hw.zone().unwrap()).unwrap();
    let (_peer, fd) = UnixStream::pair().unwrap();
    let task = thread::spawn(move || worker(&hw, false, fd.as_raw_fd()));
    await_boost(&paths);
    tmp.put("supply/BAT0/power_now", "6000000");
    tmp.put("supply/BAT0/status", "Discharging");
    assert!(task.join().unwrap().is_err());
    assert!(paths.iter().all(|p| number(p).unwrap() == STOCK));
}
#[test]
fn stock_reset_and_legacy_70w_restore() {
    let tmp = Temp::new(); let hw = tmp.fixture();
    let zone = hw.zone().unwrap(); let paths = constraints(&zone).unwrap();
    for p in &paths { write_limit(p, BOOST).unwrap(); }
    let mut lease = Lease::new(&zone, paths.clone(), BOOST, true).unwrap();
    lease.apply().unwrap(); lease.restore().unwrap(); drop(lease);
    assert!(paths.iter().all(|p| number(p).unwrap() == STOCK));
    for p in &paths { write_limit(p, BOOST).unwrap(); }
    worker(&hw, true, -1).unwrap();
    assert!(paths.iter().all(|p| number(p).unwrap() == STOCK));
}
